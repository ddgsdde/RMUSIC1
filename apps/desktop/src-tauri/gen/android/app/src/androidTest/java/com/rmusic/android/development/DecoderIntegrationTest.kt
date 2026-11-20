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
class DecoderIntegrationTest {
    companion object {
        @BeforeClass @JvmStatic fun launch() {
            val runner = InstrumentationRegistry.getInstrumentation()
            runner.startActivitySync(Intent(runner.targetContext, DecoderTestActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            val deadline = System.nanoTime() + 15000000000L
            while (!DecoderDiagnostics.ready() && System.nanoTime() < deadline) Thread.sleep(20)
            assertTrue("Native audio adapter not initialized", DecoderDiagnostics.ready())
        }
    }
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun input(file: File, rate: Int, bits: Int, position: Double = 0.0) = input(file.absolutePath,rate,bits,position)
    private fun input(source: String, rate: Int, bits: Int, position: Double = 0.0) = JSONObject()
        .put("input", source).put("sample_rate", rate).put("channels", 2).put("bits", bits).put("position", position)
    private fun httpSource(file: File, check: (String) -> Unit) {
        val server = ServerSocket(0, 2, java.net.InetAddress.getByName("127.0.0.1")); server.soTimeout = 10000
        val failure = java.util.concurrent.atomic.AtomicReference<Throwable>()
        val worker = Thread {
            try { repeat(2) { server.accept().use { socket ->
                socket.soTimeout = 10000
                val request = socket.getInputStream().bufferedReader()
                while (!request.readLine().isNullOrEmpty()) { }
                val bytes = file.readBytes(); val output = socket.getOutputStream()
                output.write("HTTP/1.1 200 OK\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                output.write(bytes); output.flush()
            } } } catch (error: Throwable) { failure.set(error) }
        }
        worker.start()
        try { check("http://127.0.0.1:${server.localPort}/queue24.flac") }
        finally { server.close(); worker.join(10000) }
        failure.get()?.let { throw AssertionError("Loopback fixture failed", it) }
    }
    private fun wav(name: String, rate: Int, bits: Int, samples: IntArray): File {
        val width = bits / 8; val data = ByteBuffer.allocate(samples.size * width).order(ByteOrder.LITTLE_ENDIAN)
        for (sample in samples) if (bits == 16) data.putShort(sample.toShort()) else data.putInt(sample)
        val header = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN)
        header.put("RIFF".toByteArray()).putInt(36 + data.capacity()).put("WAVEfmt ".toByteArray()).putInt(16)
            .putShort(1).putShort(2).putInt(rate).putInt(rate * 2 * width).putShort((2 * width).toShort()).putShort(bits.toShort())
            .put("data".toByteArray()).putInt(data.capacity())
        return File(context.cacheDir, name).apply { writeBytes(header.array() + data.array()) }
    }
    @Test fun sharedStreamPreservesPcmAndFlacAcrossPreloadAndHighRateSeek() {
        val values = IntArray(192) { (it * 997 % 65536) - 32768 }
        val first = wav("queue16.wav", 96000, 16, values)
        val expected = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach { expected.putInt(it shl 16) }
        val assets = InstrumentationRegistry.getInstrumentation().context.assets
        val flac = File(context.cacheDir, "queue24.flac").apply { writeBytes(assets.open("queue24.flac").use { it.readBytes() }) }
        val reference = assets.open("queue24.s32").use { it.readBytes() }
        httpSource(flac) { url ->
            val metadata = JSONObject(DecoderDiagnostics.probe(url))
            assertEquals("flac", metadata.getString("format")); assertEquals(24, metadata.getInt("bit_depth"))
            assertTrue(metadata.getBoolean("source_integer_lossless"))
            assertArrayEquals(expected.array() + reference, DecoderDiagnostics.decode(JSONArray().put(input(first,96000,16)).put(input(url,96000,24)).toString()))
        }
        val pattern = intArrayOf(Int.MIN_VALUE, Int.MAX_VALUE, 0, 1, -1, 17, -17, 0x12345678, 0x6fffff01)
        val samples = IntArray(192) { pattern[it % pattern.size] }
        val high = wav("queue768.wav", 768000, 32, samples)
        assertEquals(32, JSONObject(DecoderDiagnostics.probe(high.absolutePath)).getInt("bit_depth"))
        val tail = ByteBuffer.allocate((samples.size - 14) * 4).order(ByteOrder.LITTLE_ENDIAN)
        samples.drop(14).forEach { tail.putInt(it) }
        assertArrayEquals(tail.array(), DecoderDiagnostics.decode(JSONArray().put(input(high,768000,32,7.0 / 768000)).toString()))
        assertEquals(0, DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun cancellationReleasesTheRustReaderWhileSourceHeadersAreBlocked() {
        val elapsed = DecoderDiagnostics.cancelSlow()
        assertTrue("Cancellation took $elapsed ms", elapsed in 0..2000)
        assertEquals(0, DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun sharedOutputRateChangeRequestsANewStreamWithoutUpsampling() {
        for ((firstRate,nextRate) in listOf(48000 to 44100,44100 to 48000)) {
            val first = wav("rate-first-$firstRate.wav",firstRate,16,IntArray(192) { it - 96 })
            val next = wav("rate-next-$nextRate.wav",nextRate,16,IntArray(192) { 96 - it })
            try {
                DecoderDiagnostics.decode(JSONArray().put(input(first,firstRate,16)).put(input(next,nextRate,16)).toString())
                fail("A native rate boundary requires reopening the output")
            } catch (expected: IllegalStateException) {
                assertTrue("Classify a normal rate boundary: ${expected.message}",expected.message!!.contains("device renegotiation"))
            }
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun sourceRateMismatchIsRejectedInsteadOfResampled() {
        val file = wav("queue-mismatch.wav", 96000, 16, IntArray(192) { it - 96 })
        try {
            DecoderDiagnostics.decode(JSONArray().put(input(file,48000,16)).toString())
            fail("A changed source rate must not be resampled implicitly")
        } catch (expected: IllegalStateException) { assertTrue(expected.message!!.contains("不自动重采样")) }
        assertEquals(0, DecoderDiagnostics.activeDecoderCount())
    }
}
