package com.rmusic.android.development
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

@RunWith(AndroidJUnit4::class)
class QueueOutputTest {
    @Before fun launch() {
        val runner = InstrumentationRegistry.getInstrumentation()
        runner.startActivitySync(Intent(runner.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val ready = System.nanoTime() + 15000000000L
        while (!DecoderDiagnostics.ready() && System.nanoTime() < ready) Thread.sleep(20)
        assertTrue(DecoderDiagnostics.ready())
    }
    private fun startQueue(configured: Boolean = false, scheduled: Boolean = false, adjusted: Boolean = false) {
        val runner = InstrumentationRegistry.getInstrumentation()
        fun source(name: String, bits: Int): File {
            val bytes = 96000 * 2 * 2 * (bits / 8)
            val header = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN)
            header.put("RIFF".toByteArray()).putInt(36 + bytes).put("WAVEfmt ".toByteArray()).putInt(16)
                .putShort(1).putShort(2).putInt(96000).putInt(96000 * 2 * bits / 8).putShort((2 * bits / 8).toShort()).putShort(bits.toShort())
                .put("data".toByteArray()).putInt(bytes)
            return File(runner.targetContext.cacheDir,name).apply { outputStream().use { it.write(header.array()); it.write(ByteArray(bytes)) } }
        }
        val first = source("queue-output16.wav",16); val second = source("queue-output32.wav",32)
        val options = JSONObject().put("input",first.absolutePath).put("output",if(configured) JSONObject().put("max_sample_rate",48000).put("max_bit_depth",16).put("channels",1).put("buffer_ms",300).put("resync_delay_ms",20) else JSONObject())
        if(scheduled) options.put("scheduled",true)
        if(adjusted) options.put("rate_ppm",100.0)
        val input = options.toString()
        QueueDiagnostics.start(input,second.absolutePath)
    }
    @Test fun preparedGroupConsumesNothingBeforeCommitAndCancellationNeverStartsProgramme() {
        for (cancel in listOf(true,false)) {
            startQueue(scheduled=true)
            try {
                var current=JSONObject(QueueDiagnostics.state(false));val ready=System.nanoTime()+6000000000L
                while(current.getJSONObject("gate").getString("phase")!="ready"&&!current.getBoolean("finished")&&System.nanoTime()<ready){Thread.sleep(10);current=JSONObject(QueueDiagnostics.state(false))}
                assertEquals(current.toString(),"ready",current.getJSONObject("gate").getString("phase"))
                Thread.sleep(100);assertEquals(0L,JSONObject(QueueDiagnostics.state(false)).getLong("played"))
                if(cancel) {val stopped=JSONObject(QueueDiagnostics.stop());assertEquals(0L,stopped.getLong("played"));assertEquals("cancelled",stopped.getJSONObject("gate").getString("phase"))}
                else {
                    QueueDiagnostics.commit(200);Thread.sleep(60);assertEquals(0L,JSONObject(QueueDiagnostics.state(false)).getLong("played"))
                    val end=System.nanoTime()+12000000000L
                    while(!current.getBoolean("finished")&&System.nanoTime()<end){Thread.sleep(20);current=JSONObject(QueueDiagnostics.state(false))}
                    assertTrue(current.toString(),current.getBoolean("finished"));val final=JSONObject(QueueDiagnostics.stop());println("RMusicScheduledOutput $final")
                    assertEquals(384000L,final.getLong("played"));assertEquals(192000L,final.getJSONObject("ended").getLong("first"));assertEquals(192000L,final.getJSONObject("ended").getLong("second"))
                    assertEquals("released",final.getJSONObject("gate").getString("phase"));assertFalse(final.getBoolean("continuity_lost"))
                }
            } finally {QueueDiagnostics.stop()}
        }
    }
    @Test fun occupiedPhysicalRouteDoesNotCancelTheExistingWorkerOrItsForegroundService() {
        startQueue()
        try {
            var state=JSONObject(QueueDiagnostics.state(false));val ready=System.nanoTime()+6000000000L
            while(state.getLong("played")<9600&&System.nanoTime()<ready){Thread.sleep(10);state=JSONObject(QueueDiagnostics.state(false))}
            val before=state.getLong("played");assertTrue(before>=9600)
            val error=QueueDiagnostics.conflict();assertTrue(error,error.contains("占用"));Thread.sleep(200)
            state=JSONObject(QueueDiagnostics.state(false));assertTrue(state.toString(),state.getLong("played")>before)
            assertFalse(state.getBoolean("finished"));println("RMusicOutputConflict $state")
        } finally {QueueDiagnostics.stop()}
    }
    @Test fun adaptiveGroupOutputKeepsIndependentSourceAndDeviceFrames() {
        startQueue(scheduled=true,adjusted=true)
        try {
            var state=JSONObject(QueueDiagnostics.state(false));val ready=System.nanoTime()+6000000000L
            while(state.getJSONObject("gate").getString("phase")!="ready"&&!state.getBoolean("finished")&&System.nanoTime()<ready){Thread.sleep(10);state=JSONObject(QueueDiagnostics.state(false))}
            assertEquals(state.toString(),"ready",state.getJSONObject("gate").getString("phase"));assertEquals(0L,state.getLong("played"));QueueDiagnostics.commit(100)
            var sawClock=false;val end=System.nanoTime()+12000000000L
            while(!state.getBoolean("finished")&&System.nanoTime()<end){Thread.sleep(20);state=JSONObject(QueueDiagnostics.state(false));if(!state.isNull("device_clock")){val clock=state.getJSONObject("device_clock");assertEquals("android_frames",clock.getString("source"));assertEquals("monotonic",clock.getString("timestamp_domain"));sawClock=true}}
            assertTrue(state.toString(),state.getBoolean("finished"));val final=JSONObject(QueueDiagnostics.stop());println("RMusicAdaptiveOutput "+final.put("saw_device_clock",sawClock))
            assertTrue(final.toString(),final.getLong("played") in 383800L..383999L);assertEquals(2.0,final.getDouble("position"),0.00002)
            assertFalse(final.getJSONObject("signal_path").getBoolean("bit_perfect_eligible"));assertFalse(final.getBoolean("continuity_lost"))
        } finally {QueueDiagnostics.stop()}
    }
    @Test fun configuredRatePrecisionAndChannelsKeepPauseAndExactQueueTiming() {
        startQueue(true)
        try {
            var current = JSONObject(QueueDiagnostics.state(false));val started=System.nanoTime()+10000000000L
            while(current.getLong("played")<4800 && System.nanoTime()<started){Thread.sleep(20);current=JSONObject(QueueDiagnostics.state(false))}
            assertTrue(current.toString(),current.getLong("played")>=4800)
            QueueDiagnostics.state(true);Thread.sleep(150);val paused=JSONObject(QueueDiagnostics.state(true)).getLong("played")
            Thread.sleep(200);assertEquals(paused,JSONObject(QueueDiagnostics.state(true)).getLong("played"))
            current=JSONObject(QueueDiagnostics.state(false));val end=System.nanoTime()+15000000000L
            while(!current.getBoolean("finished")&&System.nanoTime()<end){Thread.sleep(20);current=JSONObject(QueueDiagnostics.state(false))}
            assertTrue(current.toString(),current.getBoolean("finished"));val final=JSONObject(QueueDiagnostics.stop());println("RMusicConfiguredOutput $final")
            assertEquals(final.toString(),192000L,final.getLong("played"));assertEquals(192000L,final.getLong("written"))
            assertEquals(96000L,final.getJSONObject("ended").getLong("first"));assertEquals(96000L,final.getJSONObject("ended").getLong("second"))
            val path=final.getJSONObject("signal_path");assertEquals(48000,path.getInt("sample_rate"));assertEquals(1,path.getInt("channels"));assertEquals(16,path.getInt("bit_depth"));assertFalse(path.getBoolean("bit_perfect_eligible"))
        } finally {QueueDiagnostics.stop()}
    }
    @Test fun oneAudioTrackKeepsFeedingAcrossBusyMainLooperPauseAndExactEof() {
        startQueue()
        try {
            fun state(paused: Boolean = false) = JSONObject(QueueDiagnostics.state(paused))
            var current = state(); val started = System.nanoTime() + 10000000000L
            while (current.getLong("played") < 9600 && System.nanoTime() < started) { Thread.sleep(20); current = state() }
            assertTrue(current.toString(),current.getLong("played") >= 9600)
            state(true); Thread.sleep(150); val paused = state(true).getLong("played")
            Thread.sleep(250); assertEquals(paused,state(true).getLong("played"))
            state()
            val entered = java.util.concurrent.CountDownLatch(1)
            val release = java.util.concurrent.CountDownLatch(1)
            val finished = java.util.concurrent.CountDownLatch(1)
            android.os.Handler(android.os.Looper.getMainLooper()).post {
                entered.countDown()
                try { release.await(5, java.util.concurrent.TimeUnit.SECONDS) } finally { finished.countDown() }
            }
            try {
                assertTrue(entered.await(2, java.util.concurrent.TimeUnit.SECONDS))
                val beforeState = state(); val before = beforeState.getLong("played")
                Thread.sleep(1200)
                val afterState = state(); val after = afterState.getLong("played")
                println("RMusicMainLooper " + JSONObject().put("block_ms",1200).put("before_frames",before).put("after_frames",after)
                    .put("before_continuity_lost",beforeState.optBoolean("continuity_lost")).put("after_continuity_lost",afterState.optBoolean("continuity_lost")))
                assertTrue("Native PCM stalled behind the main Looper: $before -> $after", after - before >= 76800L)
            } finally {
                release.countDown()
                assertTrue(finished.await(2, java.util.concurrent.TimeUnit.SECONDS))
            }
            current = state(); val end = System.nanoTime() + 15000000000L
            while (!current.getBoolean("finished") && System.nanoTime() < end) { Thread.sleep(20); current = state() }
            assertTrue(current.toString(),current.getBoolean("finished"))
            val final = JSONObject(QueueDiagnostics.stop())
            println("RMusicMainLooperEof $final")
            assertEquals(final.toString(),384000L,final.getLong("played")); assertEquals(384000L,final.getLong("written"))
            assertEquals(192000L,final.getJSONObject("ended").getLong("first")); assertEquals(192000L,final.getJSONObject("ended").getLong("second"))
            assertTrue(final.toString(),final.getBoolean("saw_second"))
            if (final.optBoolean("continuity_lost")) {
                val observed = final.getJSONObject("first_underrun_observation")
                assertEquals(final.toString(),"second",observed.getString("playback_id"))
                assertEquals(final.toString(),192000L,observed.getLong("output_frames"))
                assertEquals(2.0,observed.getDouble("position"),0.0)
            }
        } finally { QueueDiagnostics.stop() }
    }
    @Test fun realProducerStarvationRemainsVisibleBeforeEof() {
        startQueue()
        try {
            QueueDiagnostics.stall(700)
            var current = JSONObject(QueueDiagnostics.state(false)); val end = System.nanoTime() + 10000000000L
            while (!current.optBoolean("continuity_lost") && !current.getBoolean("finished") && System.nanoTime() < end) {
                Thread.sleep(20); current = JSONObject(QueueDiagnostics.state(false))
            }
            println("RMusicUnderrunNegative $current")
            assertEquals(current.toString(),700L,current.getLong("injected_producer_stall_ms"))
            assertTrue(current.toString(),current.getBoolean("continuity_lost"))
            val observed = current.getJSONObject("first_underrun_observation")
            assertEquals("first",observed.getString("playback_id"))
            assertTrue(observed.toString(),observed.getLong("output_frames") < 192000L)
        } finally { QueueDiagnostics.stop() }
    }
}
