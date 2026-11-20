@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import com.rmusic.core.*
import com.rmusic.media.*
import java.io.File
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Bounded integration journey against the independently seeded QA Core, never a user library. */
class NativeJourneyTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun main(block: suspend () -> Unit) = runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

    private fun await(timeout: Long = 20000, predicate: () -> Boolean) {
        try {
            ui.waitUntil(timeout, predicate)
        } catch (e: Throwable) {
            capture("failure")
            val s = graph.runtime.state.value
            throw AssertionError(
                "play=${s.playing}, buffer=${s.buffering}, pos=${s.position}, transport=${s.transport}, title=${s.track?.title}, error=${s.error}",
                e,
            )
        }
    }

    private fun capture(name: String) {
        val bitmap = ui.onRoot().captureToImage().asAndroidBitmap()
        val dir = File(ui.activity.filesDir, "qa").apply { mkdirs() }
        File(dir, "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    @Test
    fun loginPlaybackSeekBackgroundLyricsDownloadAndOfflineCache() {
        if (graph.api.value == null) {
            ui.onNodeWithText("服务器地址").performTextClearance()
            ui.onNodeWithText("服务器地址").performTextInput("http://127.0.0.1:18874")
            ui.onNodeWithText("账号", useUnmergedTree = false).performTextInput("android-qa")
            ui.onNodeWithText("密码", useUnmergedTree = false)
                .performTextInput("RMusicAndroidQA-20261030")
            ui.onNodeWithText("进入音乐空间").performClick()
        }
        runBlocking { delay(1200) }
        capture("after-login")
        await { graph.api.value != null && graph.runtime.selectedZone.value.isNotBlank() }
        ui.onNodeWithText("聆听").assertIsDisplayed()
        capture("01-home")
        check(
            graph.api.value!!.session.address == "http://127.0.0.1:18874" &&
                graph.api.value!!.session.username == "android-qa"
        ) {
            "仅允许独立Android QA Core"
        }
        val tracks = runBlocking {
            graph.api.value!!.request("library/tracks?limit=50").rows().map(::Track)
        }
        assertTrue(tracks.size >= 4)
        val t = tracks.first { it.rate == 96000 }
        val image = runBlocking {
            graph.images.execute(
                coil3.request.ImageRequest.Builder(ui.activity)
                    .data(graph.api.value!!.image(t.cover))
                    .size(128)
                    .build()
            )
        }
        assertTrue("Artwork: $image", image is coil3.request.SuccessResult)
        main { graph.runtime.play(listOf(t, tracks.first { it.id != t.id })) }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position > 1 }
        val playing = graph.runtime.state.value
        assertTrue(playing.output.optInt("rate") > 0)
        assertFalse(playing.output.text("mixer_note").isBlank())
        assertTrue(playing.output.optInt("rate") <= graph.devices.caps().maxRate)
        main { graph.runtime.control("seek", json("position" to 8.0)) }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position >= 8 }
        assertEquals(t.id, graph.runtime.state.value.track!!.id)
        main { graph.runtime.control("pause") }
        await { !graph.runtime.state.value.playing }
        main { graph.runtime.control("play") }
        await { graph.runtime.state.value.playing }
        ui.onNodeWithContentDescription("播放或暂停").assertDoesNotExist()
        // The mini player is opened by its title, without any test-only production UI hooks.
        ui.onAllNodesWithText(t.title).onLast().performClick()
        ui.onNodeWithContentDescription("歌词").performClick()
        await { ui.onAllNodesWithText("Follow the music").fetchSemanticsNodes().isNotEmpty() }
        capture("02-lyrics")
        val before = graph.runtime.state.value.position
        ui.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        runBlocking { delay(2200) }
        ui.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        assertTrue(graph.runtime.state.value.position >= before)
        // Connected-device foreground service must accept a remote start while the UI is stopped.
        main { graph.runtime.control("pause") }
        await { !graph.runtime.state.value.playing }
        val localZone = graph.runtime.selectedZone.value
        ui.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        runBlocking { graph.api.value!!.command(localZone, "play") }
        await { graph.runtime.state.value.playing }
        ui.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        // Seek near the true end and observe Core advancing exactly once to the second queue item.
        main { graph.runtime.control("seek", json("position" to 16.0)) }
        await(15000) {
            graph.runtime.state.value.track?.id != t.id && graph.runtime.state.value.playing
        }
        assertEquals(tracks.first { it.id != t.id }.id, graph.runtime.state.value.track!!.id)
        main {
            graph.runtime.control("pause")
            graph.downloads.enqueue(t)
        }
        await(60000) {
            graph.downloads.items.value.any {
                it.id == t.id && it.status in setOf("已缓存", "已导出并缓存", "失败")
            }
        }
        val completed = graph.downloads.items.value.first { it.id == t.id }
        assertFalse(completed.error, completed.status == "失败")
        repeat(3) {
            main {
                graph.downloads.enqueue(t)
                graph.downloads.cancel()
            }
            runBlocking { delay(400) }
            await(5000) {
                ui.activity
                    .getSystemService(android.app.ActivityManager::class.java)
                    .getRunningServices(20)
                    .none { it.service.className == DownloadService::class.java.name }
            }
        }
        val entry =
            graph.mediaCache.entries(graph.api.value!!.session.cacheNamespace).first {
                it.track.id == t.id
            }
        main { graph.runtime.playCached(entry) }
        await {
            graph.runtime.state.value.playing &&
                graph.runtime.state.value.cached &&
                graph.runtime.state.value.position > 1
        }
        assertNotEquals("offline", graph.runtime.selectedZone.value)
        main { graph.runtime.control("seek", json("position" to 8.0)) }
        await {
            graph.runtime.state.value.playing &&
                graph.runtime.state.value.cached &&
                graph.runtime.state.value.position >= 8
        }
        main {
            graph.runtime.disconnect()
            graph.runtime.playCached(entry)
        }
        await { graph.runtime.state.value.playing && graph.runtime.state.value.position > 1 }
        assertTrue(graph.runtime.state.value.cached)
        main { graph.runtime.control("pause") }
        capture("03-cached")
        main {
            graph.runtime.disconnect()
            graph.runtime.connect(graph.api.value!!)
        }
        await { graph.runtime.selectedZone.value.isNotBlank() }
        val mobileTrack = tracks.first { it.rate == 48000 && it.format == "flac" }
        main {
            graph.preferences.save(
                graph.preferences.policy.value.copy(mobileOpus = true, networkMode = "mobile")
            )
            graph.runtime.play(listOf(mobileTrack))
        }
        if (graph.devices.caps().maxRate >= 48000) {
            await {
                graph.runtime.state.value.playing &&
                    graph.runtime.state.value.transport.startsWith("Opus")
            }
            assertTrue(graph.runtime.state.value.transport.contains("有损"))
        } else {
            await { graph.runtime.state.value.playing }
            assertFalse(graph.runtime.state.value.transport.startsWith("Opus"))
        }
        main {
            graph.runtime.control("pause")
            graph.preferences.save(
                graph.preferences.policy.value.copy(mobileOpus = false, networkMode = "auto")
            )
        }
        if (graph.devices.caps().maxRate >= 48000) {
            main {
                graph.preferences.save(
                    graph.preferences.policy.value.copy(
                        networkMode = "wifi",
                        mobileOpus = true,
                        bufferSeconds = 30,
                    )
                )
                graph.runtime.play(listOf(mobileTrack))
            }
            await {
                graph.runtime.state.value.playing &&
                    !graph.runtime.state.value.transport.startsWith("Opus")
            }
            await(15000) {
                graph.mediaCache.entries(graph.api.value!!.session.cacheNamespace).any {
                    it.track.id == mobileTrack.id && it.plan.lossless
                }
            }
            main {
                graph.preferences.save(
                    graph.preferences.policy.value.copy(networkMode = "mobile", mobileOpus = true)
                )
                graph.runtime.play(listOf(mobileTrack))
            }
            await { graph.runtime.state.value.playing && graph.runtime.state.value.cached }
            assertFalse(graph.runtime.state.value.transport.startsWith("Opus"))
            main {
                graph.runtime.control("pause")
                graph.preferences.save(
                    graph.preferences.policy.value.copy(
                        networkMode = "auto",
                        mobileOpus = false,
                        bufferSeconds = 15,
                    )
                )
            }
        }
        val dsd = tracks.first { it.dsd }
        main { graph.runtime.play(listOf(dsd)) }
        await {
            graph.runtime.state.value.playing &&
                graph.runtime.state.value.transport.startsWith("Core FLAC")
        }
        assertEquals("dsf", graph.runtime.state.value.track!!.format)
        main { graph.runtime.control("pause") }
    }
}
