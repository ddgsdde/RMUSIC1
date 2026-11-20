package com.rmusic.android.development
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

@RunWith(AndroidJUnit4::class)
class SyncPrecisionTest {
    @Test fun pcm32At768kKeepsEveryIntegerBitWhenEnteringTheF64SyncProcessor() {
        val runner=InstrumentationRegistry.getInstrumentation()
        runner.startActivitySync(Intent(runner.targetContext,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val ready=System.nanoTime()+15000000000L
        while(!DecoderDiagnostics.ready()&&System.nanoTime()<ready)Thread.sleep(20)
        assertTrue(DecoderDiagnostics.ready())
        val values=intArrayOf(Int.MIN_VALUE,Int.MAX_VALUE,1,-1,0,123456789,-987654321)
        val frames=512;val size=frames*8;val data=ByteBuffer.allocate(44+size).order(ByteOrder.LITTLE_ENDIAN)
        data.put("RIFF".toByteArray()).putInt(36+size).put("WAVEfmt ".toByteArray()).putInt(16)
            .putShort(1).putShort(2).putInt(768000).putInt(768000*8).putShort(8).putShort(32).put("data".toByteArray()).putInt(size)
        for(index in 0 until frames*2)data.putInt(values[index%values.size])
        val file=File(runner.targetContext.cacheDir,"sync-768k32.wav").apply{writeBytes(data.array())}
        val decoded=ByteBuffer.wrap(DecoderDiagnostics.decodeFloat(file.absolutePath)).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals((frames-7)*16,decoded.remaining())
        for(index in 14 until frames*2)assertEquals(values[index%values.size].toDouble()/2147483648.0,decoded.double,0.0)
        assertEquals(0,DecoderDiagnostics.activeDecoderCount())
    }
}
