@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class RecoveryUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun capture(name: String) {
        val bitmap = ui.onRoot().captureToImage().asAndroidBitmap()
        val folder = File(ui.activity.filesDir, "qa").apply { mkdirs() }
        File(folder, "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    @Test
    fun resumeOfferAndDownloadControlsHaveUsableVisibleTargets() {
        val api = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(
            api.session.username == "android-qa" && api.session.address == "http://127.0.0.1:18874"
        )
        val entry = graph.mediaCache.entries(api.session.cacheNamespace).first()
        runBlocking {
            withContext(Dispatchers.Main) {
                graph.recovery.record(entry.track, 4.0, "offline", entry.key, true, true)
                graph.recovery.select(api.session.cacheNamespace)
            }
        }
        ui.waitUntil(10000) { ui.onAllNodesWithText("继续上次聆听").fetchSemanticsNodes().isNotEmpty() }
        ui.onNodeWithContentDescription("恢复上次播放").assertHasClickAction()
        capture("round2-home-resume")
        ui.onNodeWithContentDescription("收起播放恢复").performClick()
        val journal = DownloadJournal(ui.activity)
        val original = journal.rows(api.session.cacheNamespace).first { it.status == "已缓存" }
        try {
            journal.put(original.copy(status = "已暂停"))
            runBlocking {
                withContext(Dispatchers.Main) {
                    graph.downloads.selectNamespace(api.session.cacheNamespace)
                }
            }
            ui.onNodeWithContentDescription("导航").performClick()
            ui.onNodeWithText("下载与缓存").performClick()
            ui.waitUntil(10000) { ui.onAllNodesWithText("继续下载").fetchSemanticsNodes().isNotEmpty() }
            ui.onAllNodesWithText("继续下载").onFirst().assertHasClickAction()
            ui.onNode(hasScrollToIndexAction()).performScrollToNode(hasText("继续下载"))
            capture("round2-download-paused")
            ui.onAllNodesWithText("继续下载").onFirst().performClick()
            ui.waitUntil(20000) {
                graph.downloads.items.value.firstOrNull { it.id == original.id }?.status == "已缓存"
            }
            assertFalse(graph.downloads.items.value.first { it.id == original.id }.resumable)
        } finally {
            journal.close()
        }
    }

    @Test
    fun cancelThenImmediateRequeueKeepsNewIntent() {
        val api = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(
            api.session.username == "android-qa" && api.session.address == "http://127.0.0.1:18874"
        )
        val t = runBlocking {
            api.request("library/tracks?limit=50").rows().map(::Track).first {
                it.title == "Android QA 1"
            }
        }
        repeat(2) {
            val old = graph.downloads.items.value.firstOrNull { it.id == t.id }?.request
            runBlocking {
                withContext(Dispatchers.Main) {
                    graph.downloads.cancel()
                    graph.downloads.enqueue(t)
                }
            }
            ui.waitUntil(15000) {
                graph.downloads.items.value.any {
                    it.id == t.id && it.request != old && it.status == "已缓存"
                }
            }
        }
    }
}
