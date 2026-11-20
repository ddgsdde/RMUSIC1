package com.rmusic.android.development

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

@RunWith(AndroidJUnit4::class)
class PlatformCodecIntegrationTest {
    companion object { @BeforeClass @JvmStatic fun launch() { DecoderIntegrationTest.launch() } }
    private val runner get() = InstrumentationRegistry.getInstrumentation()
    private fun bytes(name: String) = runner.context.assets.open(name).use { it.readBytes() }
    private fun fixture(name: String) = File(runner.targetContext.cacheDir,name).apply { writeBytes(bytes(name)) }
    private fun input(path: String, position: Double = 0.0) = JSONObject().put("input",path).put("sample_rate",48000)
        .put("channels",2).put("bits",0).put("lossless",false).put("position",position)
    private fun compare(expected: ByteArray,actual: ByteArray) {
        assertEquals("Logical programme frames must exclude encoder delay and padding",expected.size,actual.size)
        val a = ByteBuffer.wrap(actual).order(ByteOrder.LITTLE_ENDIAN)
        val b = ByteBuffer.wrap(expected).order(ByteOrder.LITTLE_ENDIAN)
        var error = 0.0; var energy = 0.0
        while (a.hasRemaining()) {
            val x = a.int / 2147483648.0; val y = b.int / 2147483648.0
            error += (x-y)*(x-y); energy += y*y
        }
        assertTrue("Decoded waveform must align with independent FFmpeg reference: $error / $energy",error < energy*0.0001)
    }
    @Test fun aacAndOpusCrossTrackFramesAndExactSeek() {
        val aac = fixture("platform.aac.m4a")
        val a = bytes("platform.aac.s32"); val b = bytes("platform.opus.s32")
        HttpSeekIntegrationTest.Source(bytes("platform.opus")).use { source ->
            compare(a+b,DecoderDiagnostics.decode(JSONArray().put(input(aac.absolutePath)).put(input(source.url)).toString()))
            compare(a.copyOfRange(56,a.size)+b.copyOfRange(56,b.size),DecoderDiagnostics.decode(JSONArray().put(input(aac.absolutePath,7.0/48000)).put(input(source.url,7.0/48000)).toString()))
        }
        compare(bytes("platform.raw.s32"),DecoderDiagnostics.decode(JSONArray().put(input(fixture("platform.raw.aac").absolutePath)).toString()))
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
        assertEquals(0,DecoderDiagnostics.activePlatformSources())
        assertEquals(0,NativeDecoder.activeSessions())
    }
    @Test fun platformCallbackCancellationAndReadErrorsReleaseEverySession() {
        val file = fixture("platform.aac.m4a")
        for (body in listOf(false,true)) {
            val elapsed = DecoderDiagnostics.cancelPlatform(file.absolutePath,body)
            assertTrue("Platform callback cancellation with body=$body took $elapsed ms",elapsed in 0..2000)
        }
        for (fault in listOf("etag","truncated")) {
            HttpSeekIntegrationTest.Source(bytes("platform.aac.m4a"),fault,true).use { source ->
                try { DecoderDiagnostics.decode(JSONArray().put(input(source.url)).toString()); fail("MediaDataSource errors must not become natural EOF") }
                catch (expected: IllegalStateException) { assertTrue(expected.message!!.contains("媒体读取失败")) }
            }
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
        assertEquals(0,DecoderDiagnostics.activePlatformSources())
        assertEquals(0,NativeDecoder.activeSessions())
    }
}
