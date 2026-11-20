@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class OfflineRecoveryJourneyTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun main(block: suspend () -> Unit) = runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

    private fun await(block: () -> Boolean) = ui.waitUntil(20000, block)

    @Test
    fun offlineProgressUploadsOnceAndBookmarkRestoresPausedPosition() {
        val a = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(a.session.address == "http://127.0.0.1:18874" && a.session.username == "android-qa")
        val target = runBlocking {
            a.request("library/tracks?limit=50").rows().map(::Track).first {
                it.title == "Android QA 1"
            }
        }
        if (graph.mediaCache.entries(a.session.cacheNamespace).none { it.track.id == target.id }) {
            main { graph.downloads.enqueue(target) }
            await {
                graph.mediaCache.entries(a.session.cacheNamespace).any { it.track.id == target.id }
            }
        }
        val entry =
            graph.mediaCache.entries(a.session.cacheNamespace).first { it.track.id == target.id }
        val t = entry.track
        val before = runBlocking {
            a.request("library/tracks/${pathId(t.id)}/stats").optDouble("listened_seconds")
        }
        main {
            val local =
                graph.runtime.zones.value.first {
                    it.text("endpoint_id") == graph.preferences.endpointId
                }
            a.command(local.text("id"), "pause")
            graph.runtime.playCached(entry, forceOffline = true)
        }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position > 1 }
        runBlocking { delay(2300) }
        main {
            graph.runtime.control("pause")
            graph.runtime.checkpoint()
            graph.recovery.select(a.session.cacheNamespace)
        }
        val bookmark = graph.recovery.offered.value!!
        assertTrue(bookmark.position > 1)
        main {
            graph.runtime.control("seek", json("position" to 12.0))
            graph.runtime.control("stop")
        }
        runBlocking {
            graph.history.flushWrites()
            graph.history.sync()
        }
        val after = runBlocking {
            a.request("library/tracks/${pathId(t.id)}/stats").optDouble("listened_seconds")
        }
        assertTrue("Measured progress ${after-before}", after - before in 2.0..8.0)
        runBlocking { graph.history.sync() }
        assertEquals(
            after,
            runBlocking {
                a.request("library/tracks/${pathId(t.id)}/stats").optDouble("listened_seconds")
            },
            0.01,
        )
        // Restore from disk while paused; the stored position is not counted as listening.
        main {
            graph.recovery.record(t, bookmark.position, "offline", entry.key, true, true)
            graph.recovery.select(a.session.cacheNamespace)
            graph.recovery.resume()
        }
        await {
            graph.runtime.state.value.playing &&
                graph.runtime.state.value.position >= bookmark.position
        }
        assertEquals(t.id, graph.runtime.state.value.track!!.id)
        main { graph.runtime.control("pause") }
    }

    @Test
    fun onlineBookmarkIsGuardedAndNeverReplacesAnotherTrack() {
        val a = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(a.session.username == "android-qa" && a.session.address == "http://127.0.0.1:18874")
        val tracks = runBlocking { a.request("library/tracks?limit=50").rows().map(::Track) }
        val t = tracks.first { it.title == "Android QA 1" }
        val other = tracks.first { it.title == "Android QA 2" }
        main {
            val local =
                graph.runtime.zones.value.first {
                    it.text("endpoint_id") == graph.preferences.endpointId
                }
            graph.runtime.selectedZone.value = local.text("id")
            graph.runtime.play(listOf(t))
        }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.track?.id == t.id }
        main { graph.runtime.control("seek", json("position" to 6.0)) }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position >= 6 }
        main { graph.runtime.control("pause") }
        await { !graph.runtime.state.value.playing }
        main {
            graph.runtime.checkpoint()
            graph.recovery.select(a.session.cacheNamespace)
            graph.recovery.resume()
        }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position >= 6 }
        main { graph.runtime.play(listOf(other)) }
        await {
            graph.runtime.state.value.playing && graph.runtime.state.value.track?.id == other.id
        }
        main {
            graph.runtime.control("pause")
            graph.recovery.record(t, 6.0, "", "", false, true)
            graph.recovery.select(a.session.cacheNamespace)
            val failure = runCatching { graph.recovery.resume() }.exceptionOrNull()
            assertTrue(failure is IllegalArgumentException)
        }
        assertEquals(other.id, graph.runtime.state.value.track!!.id)
    }
}
