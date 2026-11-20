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
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.net.ServerSocket

@RunWith(AndroidJUnit4::class)
class NativeCodecIntegrationTest {
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
    private fun fixture(name: String) = File(runner.targetContext.cacheDir, name).apply {
        writeBytes(runner.context.assets.open(name).use { it.readBytes() })
    }
    private fun input(file: File, rate: Int, bits: Int, lossless: Boolean, position: Double = 0.0) = JSONObject()
        .put("input", file.absolutePath).put("sample_rate", rate).put("channels", 2)
        .put("bits", bits).put("lossless", lossless).put("position", position)
    private fun httpSource(file: File, check: (String) -> Unit) {
        val server = ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")); server.soTimeout = 10000
        val failure = java.util.concurrent.atomic.AtomicReference<Throwable>()
        val worker = Thread {
            try { server.accept().use { socket ->
                socket.soTimeout = 10000
                val request = socket.getInputStream().bufferedReader()
                while (!request.readLine().isNullOrEmpty()) { }
                val bytes = file.readBytes(); val output = socket.getOutputStream()
                output.write("HTTP/1.1 200 OK\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                output.write(bytes); output.flush()
            } } catch (error: Throwable) { failure.set(error) }
        }
        worker.start()
        try { check("http://127.0.0.1:${server.localPort}/native24.m4a") }
        finally { server.close(); worker.join(10000) }
        failure.get()?.let { throw AssertionError("Loopback fixture failed", it) }
    }
    @Test fun alac24PreservesSamplesAndExactSeekAcrossQueue() {
        val file = fixture("native24.m4a")
        val reference = runner.context.assets.open("queue24.s32").use { it.readBytes() }
        val metadata = JSONObject(DecoderDiagnostics.probe(file.absolutePath))
        assertEquals("alac", metadata.getString("format"))
        assertEquals(24, metadata.getInt("bit_depth"))
        assertTrue(metadata.getBoolean("source_integer_lossless"))
        httpSource(file) { url ->
            val queue = JSONArray().put(input(file,96000,24,true)).put(input(file,96000,24,true,7.0 / 96000).put("input",url))
            assertArrayEquals(reference + reference.copyOfRange(7 * 8,reference.size), DecoderDiagnostics.decode(queue.toString()))
        }
        assertEquals(0, DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun mp3AndVorbisDecodeAsLossyAndTrimQueueFrames() {
        val queue = JSONArray()
        val references = ArrayList<ByteArray>()
        for ((name, codec) in listOf("native.mp3" to "mp3", "native.ogg" to "vorbis")) {
            val file = fixture(name)
            val metadata = JSONObject(DecoderDiagnostics.probe(file.absolutePath))
            assertEquals(codec, metadata.getString("format"))
            assertFalse(metadata.getBoolean("source_integer_lossless"))
            queue.put(input(file,48000,0,false))
            references.add(runner.context.assets.open("$name.s32").use { it.readBytes() })
        }
        val expected = references.reduce { a,b -> a + b }
        lateinit var actual: ByteArray
        httpSource(fixture("native.mp3")) { url ->
            queue.getJSONObject(0).put("input",url)
            actual = DecoderDiagnostics.decode(queue.toString())
        }
        assertEquals("Decoded frame count must match independent FFmpeg decoding", expected.size, actual.size)
        val a = ByteBuffer.wrap(actual).order(ByteOrder.LITTLE_ENDIAN)
        val b = ByteBuffer.wrap(expected).order(ByteOrder.LITTLE_ENDIAN)
        var difference = 0.0; var energy = 0.0
        while (a.hasRemaining()) {
            val sample = a.int / 2147483648.0; val reference = b.int / 2147483648.0
            difference += (sample - reference) * (sample - reference); energy += reference * reference
        }
        assertTrue("Lossy decoding must align with independent reference: $difference / $energy", difference < energy * 0.0001)
        httpSource(fixture("native.ogg")) { url ->
            assertEquals(references[1].size,DecoderDiagnostics.decode(JSONArray().put(input(fixture("native.ogg"),48000,0,false).put("input",url)).toString()).size)
        }
        assertEquals(0, DecoderDiagnostics.activeDecoderCount())
    }
}
