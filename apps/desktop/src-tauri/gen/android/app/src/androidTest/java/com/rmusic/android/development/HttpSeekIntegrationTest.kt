package com.rmusic.android.development

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith
import java.net.ServerSocket
import java.net.SocketException
import java.util.Collections

@RunWith(AndroidJUnit4::class)
class HttpSeekIntegrationTest {
    companion object {
        @BeforeClass @JvmStatic fun launch() {
            val runner = InstrumentationRegistry.getInstrumentation()
            runner.startActivitySync(Intent(runner.targetContext, DecoderTestActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            val deadline = System.nanoTime() + 15000000000L
            while (!DecoderDiagnostics.ready() && System.nanoTime() < deadline) Thread.sleep(20)
            assertTrue(DecoderDiagnostics.ready())
        }
    }
    private val runner get() = InstrumentationRegistry.getInstrumentation()
    private fun bytes(name: String) = runner.context.assets.open(name).use { it.readBytes() }
    private fun input(url: String, rate: Int = 96000, bits: Int = 24, position: Double = 0.0) = JSONObject()
        .put("input",url).put("sample_rate",rate).put("channels",2).put("bits",bits)
        .put("lossless",bits > 0).put("position",position)
    internal class Source(private val bytes: ByteArray, private val fault: String = "", private val platformOnly: Boolean = false) : AutoCloseable {
        private val server = ServerSocket(0, 8, java.net.InetAddress.getByName("127.0.0.1"))
        @Volatile private var closed = false
        val ranges = Collections.synchronizedList(ArrayList<Long>())
        val validators = Collections.synchronizedList(ArrayList<String>())
        private val failures = Collections.synchronizedList(ArrayList<Throwable>())
        private val connections = Collections.synchronizedList(ArrayList<Thread>())
        val url get() = "http://127.0.0.1:${server.localPort}/audio/fixture"
        private val worker = Thread {
            try {
                while (!closed) {
                    val socket = server.accept()
                    val task = Thread {
                        socket.use {
                            try {
                                it.soTimeout = 5000
                                val request = it.getInputStream().bufferedReader()
                                request.readLine()
                                val headers = HashMap<String,String>()
                                while (true) {
                                    val line = request.readLine() ?: break
                                    if (line.isEmpty()) break
                                    val pair = line.split(':',limit=2)
                                    if (pair.size == 2) headers[pair[0].lowercase()] = pair[1].trim()
                                }
                                assertEquals("identity",headers["accept-encoding"])
                                val range = headers["range"]?.removePrefix("bytes=")?.substringBefore('-')?.toLong()
                                val fault = if (platformOnly && DecoderDiagnostics.activePlatformSources() == 0) "" else this@Source.fault
                                if (range != null) { ranges.add(range); validators.add(headers["if-range"] ?: "") }
                                val start = if (fault == "ignored") 0 else range?.toInt() ?: 0
                                val status = if (range == null || fault == "ignored") "200 OK" else "206 Partial Content"
                                val etag = if (fault == "weak") "W/\"fixture-v1\"" else if (range != null && fault == "etag") "\"changed\"" else "\"fixture-v1\""
                                val contentRange = if (status.startsWith("206")) "Content-Range: bytes ${start + if (fault == "offset") 1 else 0}-${bytes.size - 1}/${bytes.size + if (fault == "total") 1 else 0}\r\n" else ""
                                val encoding = if (fault == "encoding") "Content-Encoding: gzip\r\n" else ""
                                val output = it.getOutputStream()
                                val identity = if (fault == "stream") "" else "Content-Length: ${bytes.size - start}\r\nAccept-Ranges: bytes\r\nETag: $etag\r\n"
                                output.write("HTTP/1.1 $status\r\n${identity}${contentRange}${encoding}Connection: close\r\n\r\n".toByteArray())
                                if (range == null || fault != "truncated") output.write(bytes,start,bytes.size-start)
                                output.flush()
                            } catch (_: SocketException) { // Seeking deliberately closes the preceding response.
                            } catch (error: Throwable) { failures.add(error) }
                        }
                    }
                    connections.add(task); task.start()
                }
            } catch (error: SocketException) { if (!closed) failures.add(error) }
        }.apply { start() }
        override fun close() {
            closed = true; server.close(); worker.join(5000)
            connections.toList().forEach { it.join(5000) }
            failures.firstOrNull()?.let { throw AssertionError("HTTP fixture failed",it) }
        }
    }
    @Test fun rangeReadsDecodeTailMoovAlacAndExactVorbisTail() {
        val reference = bytes("native-tail24.s32")
        Source(bytes("native-tail24.m4a")).use { source ->
            val queue = JSONArray().put(input(source.url)).put(input(source.url,position=7.0/96000))
            assertArrayEquals(reference + reference.copyOfRange(56,reference.size),DecoderDiagnostics.decode(queue.toString()))
            assertTrue("MP4 tail metadata requires a real HTTP seek",source.ranges.any { it > 65536 })
            assertTrue(source.validators.all { it == "\"fixture-v1\"" })
        }
        Source(bytes("native.ogg")).use { source ->
            val expected = bytes("native.ogg.s32")
            val actual = DecoderDiagnostics.decode(JSONArray().put(input(source.url,48000,0)).toString())
            assertEquals("Vorbis tail padding must not add programme frames",expected.size,actual.size)
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun liveOpusWithoutLengthOrRangeReplaysOnlyTheCodecProbeWindow() {
        val expected = bytes("platform.opus.s32")
        Source(bytes("platform.opus"),"stream").use { source ->
            val actual = DecoderDiagnostics.decode(JSONArray().put(input(source.url,48000,0).put("duration",0.5)).toString())
            assertEquals("Encoder delay and tail padding must not add frames",expected.size,actual.size)
            val a=java.nio.ByteBuffer.wrap(actual).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            val b=java.nio.ByteBuffer.wrap(expected).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            var error=0.0;var energy=0.0
            while(a.hasRemaining()){val x=a.int/2147483648.0;val y=b.int/2147483648.0;error+=(x-y)*(x-y);energy+=y*y}
            assertTrue("Streaming waveform alignment",error<energy*0.0001)
            assertTrue("No HTTP seeking without entity identity",source.ranges.isEmpty())
        }
        Source(bytes("platform.opus"),"stream").use { source ->
            try { DecoderDiagnostics.decode(JSONArray().put(input(source.url,48000,0)).toString());fail("Unknown live duration must not produce padded programme frames") }
            catch (expected: IllegalStateException) {assertTrue(expected.message!!.contains("节目时长"))}
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
        assertEquals(0,DecoderDiagnostics.activePlatformSources())
        assertEquals(0,NativeDecoder.activeSessions())
    }
    @Test fun liveOpusContinuesPastTheBoundedReplayWindow() {
        val encoded = bytes("platform-long.opus")
        assertTrue("Fixture must pass both bounded windows",encoded.size > 512 * 1024)
        Source(encoded,"stream").use { source ->
            val actual = DecoderDiagnostics.decode(JSONArray().put(input(source.url,48000,0).put("duration",18.0)).toString())
            assertEquals("Complete programme, without encoder padding",864000 * 2 * 4,actual.size)
            assertTrue("No new HTTP range response",source.ranges.isEmpty())
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
        assertEquals(0,DecoderDiagnostics.activePlatformSources())
        assertEquals(0,NativeDecoder.activeSessions())
    }
    @Test fun rawSeeksPreserveBytesAndRejectChangedEntitiesOrRanges() {
        val data = ByteArray(262144) { (it * 31 % 251).toByte() }
        Source(data).use { source ->
            assertArrayEquals(data.copyOfRange(0,16) + data.copyOfRange(data.size-16,data.size) + data.copyOfRange(7,23),DecoderDiagnostics.httpSeek(source.url))
            assertTrue(source.ranges.contains((data.size-16).toLong()))
            assertTrue(source.ranges.contains(7L))
            assertTrue(source.validators.all { it == "\"fixture-v1\"" })
        }
        for ((fault, message) in listOf("etag" to "实体", "ignored" to "206", "offset" to "Content-Range", "total" to "Content-Range", "encoding" to "编码", "weak" to "强实体", "truncated" to "body")) {
            Source(data,fault).use { source ->
                try { DecoderDiagnostics.httpSeek(source.url); fail("Must reject $fault before mixing source bytes") }
                catch (expected: IllegalStateException) { assertTrue("$fault: ${expected.message}",expected.message!!.contains(message)) }
            }
        }
    }
    @Test fun cancellationClosesPreviousAndPendingSeekResponses() {
        for (body in listOf(false,true)) {
            val elapsed = DecoderDiagnostics.cancelHttpSeek(body)
            assertTrue("Cancellation during body=$body took $elapsed ms",elapsed in 0..2000)
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
}
