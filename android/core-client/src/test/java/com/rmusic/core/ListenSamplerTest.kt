package com.rmusic.core

import java.time.Instant
import org.junit.Assert.*
import org.junit.Test

class ListenSamplerTest {
    private val at = Instant.parse("2026-10-29T23:59:58Z").toEpochMilli()

    private fun sampler() =
        ListenSampler(Track(json("id" to "track", "title" to "QA", "duration" to 120)), at)

    private fun seconds(s: ListenSampler) =
        s.snapshot.child("days").let { d ->
            d.keys().asSequence().sumOf { d.child(it).optDouble("seconds") }
        }

    @Test
    fun listeningSplitsAtUtcMidnight() {
        val s = sampler()
        s.sample(0.0, true, at, 1000)
        s.sample(4.0, true, at + 4000, 5000)
        assertEquals(4.0, seconds(s), 1e-8)
        assertEquals(2.0, s.snapshot.child("days").child("2026-10-29").optDouble("seconds"), 1e-8)
        assertEquals(at + 1999, s.snapshot.child("days").child("2026-10-29").optLong("last_at"))
    }

    @Test
    fun seekPauseAndSuspensionNeverInventListens() {
        val s = sampler()
        s.sample(0.0, true, at, 0)
        s.sample(60.0, true, at + 1000, 1000, true)
        s.sample(61.0, false, at + 2000, 2000)
        s.sample(61.0, true, at + 12000, 12000)
        s.sample(62.0, true, at + 13000, 13000)
        s.sample(100.0, true, at + 51000, 51000)
        assertEquals(2.0, seconds(s), 1e-8)
    }

    @Test
    fun clockChangesResetBaseline() {
        val s = sampler()
        s.sample(0.0, true, at, 0)
        s.sample(1.0, true, at - 1000, 1000)
        s.sample(2.0, true, at + 300000, 2000)
        assertEquals(0.0, seconds(s), 0.0)
    }

    @Test
    fun snapshotsAreIndependentAndTerminalCannotReopen() {
        val s = sampler()
        s.sample(0.0, true, at, 0)
        val first = s.flush(at)!!
        assertNull(s.flush(at + 1000))
        s.sample(2.0, true, at + 2000, 2000)
        val done = s.finish("ended", at + 2000)
        s.sample(8.0, true, at + 8000, 8000)
        assertEquals(0, first.child("days").length())
        assertEquals(2, done.optLong("version"))
        assertEquals(2.0, seconds(s), 0.0)
    }
}
