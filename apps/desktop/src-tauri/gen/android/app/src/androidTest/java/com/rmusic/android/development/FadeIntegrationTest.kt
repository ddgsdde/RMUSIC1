package com.rmusic.android.development

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.abs

@RunWith(AndroidJUnit4::class)
class FadeIntegrationTest {
    @Before fun launch() {
        val runner = InstrumentationRegistry.getInstrumentation()
        runner.startActivitySync(Intent(runner.targetContext, DecoderTestActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        assertTrue(DecoderDiagnostics.ready())
    }
    @Test fun equalPowerEnvelopesMatchFfmpegAcrossDspChangeAndSeek() {
        val runner = InstrumentationRegistry.getInstrumentation()
        for ((rate, bits, extension) in listOf(Triple(96000,24,"flac"), Triple(768000,32,"wav"))) {
            val name = "dsp-$rate"
            val file = File(runner.targetContext.cacheDir,"$name.$extension")
            file.writeBytes(runner.context.assets.open("$name.$extension").use { it.readBytes() })
            val fadeMs = if (rate == 96000) 10 else 1
            val fade = JSONObject().put("fade_in_ms",fadeMs).put("fade_out_ms",fadeMs)
            val config = JSONObject().put("enabled",true).put("preamp_db",-6).put("balance",.25).put("normalize",true).put("target_lufs",-18)
                .put("bands",JSONArray().put(JSONObject().put("enabled",true).put("frequency",1200).put("gain_db",4).put("q",1.2)))
            fun input(position:Double) = JSONObject().put("input",file.absolutePath).put("sample_rate",rate).put("channels",2).put("bits",bits)
                .put("position",position).put("duration",4096.0/rate).put("transitions",fade)
            val queue = JSONArray().put(input(0.0).put("dsp",config).put("measured_lufs",-14)).put(input(7.0/rate))
            val actual = DecoderDiagnostics.decode(queue.toString())
            val reference = runner.context.assets.open("$name-faded-dsp.s32").use { it.readBytes() } +
                runner.context.assets.open("$name-faded-bypass.s32").use { it.readBytes() }
            assertEquals(reference.size,actual.size)
            val a = ByteBuffer.wrap(actual).order(ByteOrder.LITTLE_ENDIAN)
            val b = ByteBuffer.wrap(reference).order(ByteOrder.LITTLE_ENDIAN)
            var maximum = 0L
            while (b.hasRemaining()) maximum = maxOf(maximum,abs(a.int.toLong() - b.int.toLong()))
            assertTrue("$rate Hz fade differs from FFmpeg qsin by $maximum S32 LSB",maximum <= 4)
            fun raw() = input(0.0).put("transitions",JSONObject()).put("usb_bitperfect",true)
            for (guarded in listOf(
                JSONArray().put(input(0.0).put("usb_bitperfect",true)),
                JSONArray().put(raw()).put(input(0.0))
            )) {
                val failure = runCatching { DecoderDiagnostics.decode(guarded.toString()) }.exceptionOrNull()
                assertTrue("USB raw must reject fades initially and during preload: $failure",failure?.message?.contains("USB 原样输出不支持淡入淡出") == true)
            }
            assertEquals(0,DecoderDiagnostics.activeDecoderCount())
            println("RMusicFade " + JSONObject().put("rate",rate).put("source_bits",bits).put("fade_ms",fadeMs).put("frames",8185)
                .put("max_s32_lsb_error",maximum).put("seek_keeps_absolute_envelope",true).put("usb_fade_initial_and_preload_rejected",true))
        }
    }
}
