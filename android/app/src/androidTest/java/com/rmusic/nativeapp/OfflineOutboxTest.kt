package com.rmusic.nativeapp

import androidx.test.platform.app.InstrumentationRegistry
import com.rmusic.core.*
import com.rmusic.nativeapp.playback.OfflineListenStore
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class OfflineOutboxTest {
    private val context
        get() = InstrumentationRegistry.getInstrumentation().targetContext

    private fun snapshot(version: Long = 1, reason: String = "active") =
        json(
            "id" to "fixed-session",
            "version" to version,
            "reason" to reason,
            "days" to JSONObject(),
        )

    private fun store() = OfflineListenStore(context, "qa-outbox-${java.util.UUID.randomUUID()}.db")

    @Test
    fun lateAcknowledgementNeverDeletesNewerTerminalSnapshot() {
        val s = store()
        try {
            val first = snapshot()
            s.save("owner", first)
            s.save("owner", snapshot(2, "ended"))
            s.acknowledge("owner", json("id" to "fixed-session", "version" to 1))
            assertEquals(2, s.pending("owner").single().optLong("version"))
            s.acknowledge("other", json("id" to "fixed-session", "version" to 2))
            assertEquals(1, s.pending("owner").size)
            s.acknowledge("owner", json("id" to "fixed-session", "version" to 2))
            assertTrue(s.pending("owner").isEmpty())
        } finally {
            s.close()
        }
    }

    @Test
    fun processRecoverySealsOnlyActiveRecordsOnce() {
        val s = store()
        try {
            s.save("owner", snapshot(5))
            s.acknowledge("owner", json("id" to "fixed-session", "version" to 5))
            assertTrue(s.pending("owner").isEmpty())
            s.recover()
            val next = s.pending("owner").single()
            assertEquals("interrupted", next.text("reason"))
            assertEquals(6, next.optLong("version"))
            s.recover()
            assertEquals(6, s.pending("owner").single().optLong("version"))
        } finally {
            s.close()
        }
    }

    @Test
    fun rejectionIsRetainedAndScopedInsteadOfHotLooping() {
        val s = store()
        try {
            s.save("owner", snapshot())
            s.acknowledge(
                "owner",
                json("id" to "fixed-session", "version" to 1, "code" to "invalid_snapshot"),
            )
            s.save("owner", snapshot(2, "ended"))
            assertTrue(s.pending("owner").isEmpty())
            assertEquals(0 to 1, s.counts("owner"))
            assertEquals(0 to 0, s.counts("other"))
        } finally {
            s.close()
        }
    }
}
