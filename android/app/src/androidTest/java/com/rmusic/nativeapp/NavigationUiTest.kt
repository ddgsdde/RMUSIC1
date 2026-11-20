@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class NavigationUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun capture(name: String) {
        val folder = File(ui.activity.filesDir, "qa").apply { mkdirs() }
        val bitmap =
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
                .uiAutomation
                .takeScreenshot()
        File(folder, "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }

    @Test
    fun librarySortSelectionArtistAlbumsMetadataAndDeviceSettings() {
        ui.waitUntil(15000) { graph.api.value != null }
        check(
            graph.api.value!!.session.address == "http://127.0.0.1:18874" &&
                graph.api.value!!.session.username == "android-qa"
        ) {
            "仅允许独立Android QA Core"
        }
        ui.onAllNodesWithText("音乐空间").onFirst().performClick()
        ui.onNodeWithText("排序 · 默认顺序").performClick()
        ui.onNodeWithText("最新发行").performClick()
        ui.onNode(hasScrollToIndexAction()).performScrollToNode(hasText("Android QA 1"))
        ui.onNode(hasScrollToIndexAction()).performScrollToIndex(0)
        ui.onNodeWithText("多选").performClick()
        ui.onNodeWithText("全选已加载").performClick()
        val total = runBlocking { graph.api.value!!.request("library/tracks?limit=50").rows().size }
        ui.onNodeWithText("已选 $total").assertIsDisplayed()
        capture("04-library-selection")
        ui.onNodeWithContentDescription("结束多选").performClick()
        ui.onNodeWithText("艺术家").performClick()
        ui.waitUntil(10000) {
            ui.onAllNodesWithText("R-Music Test").fetchSemanticsNodes().isNotEmpty()
        }
        ui.onAllNodesWithText("R-Music Test").onFirst().performClick()
        ui.onNode(hasScrollToIndexAction()).performScrollToNode(hasContentDescription("编辑资料"))
        ui.onNodeWithContentDescription("编辑资料").performScrollTo().performClick()
        val description = "Native Android metadata QA ${System.currentTimeMillis()}"
        ui.onNodeWithText("简介").performTextClearance()
        ui.onNodeWithText("简介").performTextInput(description)
        ui.onNodeWithText("保存修改").performScrollTo().performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithText("保存修改").fetchSemanticsNodes().isEmpty() }
        ui.onNodeWithText("专辑").performClick()
        ui.waitUntil(10000) {
            runCatching {
                    ui.onNode(hasScrollToIndexAction())
                        .performScrollToNode(hasText("Native Journey"))
                    true
                }
                .getOrDefault(false)
        }
        capture("05-artist-albums")
        val api = graph.api.value!!
        val artist = runBlocking {
            api.request("library/artists?limit=50").rows().first {
                it.text("name") == "R-Music Test"
            }
        }
        val profile = runBlocking {
            api.request("library/catalog/artist/${pathId(artist.text("id"))}/profile")
        }
        assertEquals(description, profile.child("profile").text("description"))
        ui.onNodeWithContentDescription("声音设备").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithText("输出方式与设备").fetchSemanticsNodes().isNotEmpty() }
        ui.onNodeWithText("输出方式与设备").performClick()
        ui.onNodeWithText("Android 输出").assertIsDisplayed()
        capture("06-output-settings")
        ui.activity.onBackPressedDispatcher.onBackPressed()
    }
}
