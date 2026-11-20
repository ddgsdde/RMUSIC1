@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Real native search, menus and mutations, checked against the independently seeded Core. */
class LibraryActionsTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    @Test
    fun searchFavoritesPlaylistCreateAppendRemoveAndDiscovery() {
        ui.waitUntil(15000) { graph.api.value != null }
        val api = graph.api.value!!
        check(
            api.session.address == "http://127.0.0.1:18874" && api.session.username == "android-qa"
        )
        val name = "Android UI playlist QA"
        runBlocking {
            api.request("playlists")
                .rows()
                .filter { it.text("name") == name }
                .forEach { api.request("playlists/${pathId(it.text("id"))}", "DELETE") }
        }
        ui.onNodeWithContentDescription("播放列表", useUnmergedTree = true).performClick()
        ui.onNodeWithContentDescription("创建播放列表").performClick()
        ui.onNodeWithText("名称").performTextInput(name)
        ui.onNodeWithText("保存").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty() }
        val id = runBlocking {
            api.request("playlists").rows().first { it.text("name") == name }.text("id")
        }
        try {
            ui.onNodeWithContentDescription("首页", useUnmergedTree = true).performClick()
            ui.onNodeWithText("搜索歌曲、艺术家、专辑").performTextInput("Android")
            ui.onNode(hasSetTextAction()).performImeAction()
            ui.waitUntil(10000) { ui.onAllNodesWithText("综合").fetchSemanticsNodes().isNotEmpty() }
            ui.onNode(hasText("歌曲") and hasClickAction()).performClick()
            val target = runBlocking {
                api.request("search?q=Android&limit=60").rows().first()
            }
                .let(::Track)
            ui.waitUntil(10000) {
                runCatching {
                        ui.onNode(hasScrollToIndexAction())
                            .performScrollToNode(hasText(target.title))
                        true
                    }
                    .getOrDefault(false)
            }
            ui.onAllNodesWithContentDescription("歌曲操作").onFirst().performClick()
            ui.onNodeWithText("加入播放列表").performClick()
            ui.waitUntil(10000) { ui.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText(name).performClick()
            ui.waitUntil(10000) { ui.onAllNodesWithText("综合").fetchSemanticsNodes().isNotEmpty() }
            ui.waitUntil(10000) {
                runCatching {
                        ui.onNode(hasScrollToIndexAction())
                            .performScrollToNode(hasText(target.title))
                        true
                    }
                    .getOrDefault(false)
            }
            val stored = runBlocking { api.request("playlists/${pathId(id)}") }
            assertTrue(stored.optJSONArray("track_ids").toString().contains(target.id))
            ui.onAllNodesWithContentDescription("歌曲操作").onFirst().performClick()
            ui.waitUntil(5000) {
                ui.onAllNodesWithText("取消喜欢").fetchSemanticsNodes().isNotEmpty() ||
                    ui.onAllNodesWithText("加入我喜欢").fetchSemanticsNodes().isNotEmpty()
            }
            val wasLiked = ui.onAllNodesWithText("取消喜欢").fetchSemanticsNodes().isNotEmpty()
            ui.onNodeWithText(if (wasLiked) "取消喜欢" else "加入我喜欢").performClick()
            ui.waitUntil(10000) {
                runBlocking {
                    api.request("favorites")
                        .optJSONArray("track_ids")
                        .toString()
                        .contains(target.id) != wasLiked
                }
            }
            ui.onNodeWithContentDescription("播放列表", useUnmergedTree = true).performClick()
            ui.waitUntil(10000) { ui.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText(name).performClick()
            ui.waitUntil(10000) {
                ui.onAllNodesWithText(target.title).fetchSemanticsNodes().isNotEmpty()
            }
            ui.onNodeWithText("播放全部").performClick()
            ui.waitUntil(15000) {
                graph.runtime.state.value.playing &&
                    graph.runtime.state.value.track?.id == target.id
            }
            ui.onAllNodesWithContentDescription("歌曲操作").onFirst().performClick()
            ui.onNodeWithText("移出播放列表").performClick()
            ui.waitUntil(10000) {
                runBlocking {
                    api.request("playlists/${pathId(id)}").optJSONArray("track_ids").length() == 0
                }
            }
            ui.onNodeWithContentDescription("首页", useUnmergedTree = true).performClick()
            ui.onNodeWithContentDescription("导航").performClick()
            ui.onNodeWithText("发现音乐").performClick()
            ui.onNodeWithContentDescription("换一批").assertIsDisplayed()
            ui.onNodeWithContentDescription("导航").performClick()
            ui.onNodeWithText("聆听统计").performClick()
            ui.onNodeWithText("最近 30 天").assertIsDisplayed()
        } catch (e: Throwable) {
            val dir=File(ui.activity.filesDir,"qa").apply {mkdirs()}
            runCatching { java.io.File(dir,"library-failure-tree.txt").writeText(ui.onRoot().printToString()) }
            runCatching { androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().let {bitmap -> java.io.File(dir,"library-failure.png").outputStream().use {bitmap.compress(Bitmap.CompressFormat.PNG,100,it)} } }
            throw e
        } finally {
            runBlocking { api.request("playlists/${pathId(id)}", "DELETE") }
        }
    }
}
