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
class VorbisBoundaryIntegrationTest {
    companion object { @BeforeClass @JvmStatic fun launch() { DecoderIntegrationTest.launch() } }
    private val runner get() = InstrumentationRegistry.getInstrumentation()
    private fun bytes(name: String) = runner.context.assets.open(name).use { it.readBytes() }
    private fun input(path: String,position: Double = 0.0) = JSONObject().put("input",path).put("sample_rate",48000)
        .put("channels",2).put("bits",0).put("lossless",false).put("position",position)
    private fun compare(expected: ByteArray,actual: ByteArray) {
        assertEquals("Ogg logical frame boundaries must match independent libvorbis reference",expected.size,actual.size)
        val a = ByteBuffer.wrap(actual).order(ByteOrder.LITTLE_ENDIAN); val b = ByteBuffer.wrap(expected).order(ByteOrder.LITTLE_ENDIAN)
        var error = 0.0; var energy = 0.0
        while (a.hasRemaining()) { val x = a.int / 2147483648.0; val y = b.int / 2147483648.0; error += (x-y)*(x-y); energy += y*y }
        assertTrue("Vorbis waveform must align: $error / $energy",error < energy*0.00001)
    }
    @Test fun shortTrimmedAndOffsetStreamsUseExactGranuleBoundaries() {
        for (name in listOf("native-short","vorbis-start-cut","vorbis-start-offset","vorbis-eos-short")) {
            val file = File(runner.targetContext.cacheDir,"$name.ogg").apply { writeBytes(bytes("$name.ogg")) }
            val reference = bytes("$name.s32")
            compare(reference,DecoderDiagnostics.decode(JSONArray().put(input(file.absolutePath)).toString()))
            compare(reference.copyOfRange(56,reference.size),DecoderDiagnostics.decode(JSONArray().put(input(file.absolutePath,7.0/48000)).toString()))
        }
        HttpSeekIntegrationTest.Source(bytes("native.ogg"),"weak").use { source ->
            compare(bytes("native.ogg.s32"),DecoderDiagnostics.decode(JSONArray().put(input(source.url)).toString()))
            assertTrue("A buffered rewind must not stitch unvalidated HTTP ranges",source.ranges.isEmpty())
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
    @Test fun missingEosAndCorruptPagesAreNotNaturalEof() {
        for (name in listOf("vorbis-missing-eos","vorbis-corrupt")) {
            val file = File(runner.targetContext.cacheDir,"$name.ogg").apply { writeBytes(bytes("$name.ogg")) }
            try { DecoderDiagnostics.decode(JSONArray().put(input(file.absolutePath)).toString()); fail("Incomplete or corrupt Vorbis must fail") }
            catch (expected: IllegalStateException) { assertTrue(expected.message!!.isNotEmpty()) }
        }
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
}
