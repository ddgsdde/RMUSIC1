@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import com.rmusic.media.OutputMode
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Run the two methods in separate adb instrumentation invocations with force-stop between. */
class ProcessRecoveryTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun main(block: suspend () -> Unit) = runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

    @Test
    fun prepareInterruptedTransfer() {
        val before = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(
            before.session.username == "android-qa" &&
                before.session.address in setOf("http://127.0.0.1:18874", "http://127.0.0.1:18875")
        )
        main { graph.login("http://127.0.0.1:18875", "android-qa", "RMusicAndroidQA-20261030") }
        val a = graph.api.value!!
        val t = runBlocking {
            a.request("library/tracks?limit=50").rows().map(::Track).first {
                it.title == "Download Lifecycle QA"
            }
        }
        runBlocking { delay(400) }
        val previous = graph.downloads.items.value.firstOrNull {it.id == t.id}?.request
        // Only this generated fixture's app cache is reset; never a user/Core cache.
        graph.mediaCache.entries(a.session.cacheNamespace).filter {it.track.id == t.id}.forEach {graph.mediaCache.remove(it.key)}
        main {
            graph.preferences.save(
                graph.preferences.policy.value.copy(
                    networkMode = "wifi",
                    outputMode = OutputMode.AUTO,
                )
            )
            graph.downloads.directory = null
            graph.downloads.enqueue(t)
        }
        ui.waitUntil(20000) {
            graph.downloads.items.value.any {
                it.id == t.id && it.request != previous && (it.status == "下载中" && it.bytes >= 512 * 1024 || it.status == "失败")
            }
        }
        val item = graph.downloads.items.value.first { it.id == t.id }
        assertEquals(item.error, "下载中", item.status)
        val saved = DownloadJournal(ui.activity).use { it.get(a.session.cacheNamespace, t.id)!! }
        assertTrue(saved.approval.isNotBlank())
        assertTrue(saved.cacheKey.isNotBlank())
        ui.activity
            .getSharedPreferences("qa-process-marker", 0)
            .edit()
            .putString("id", t.id)
            .putString("key", saved.cacheKey)
            .putString("approval", saved.approval)
            .commit()
    }

    @Test
    fun resumeInNewProcess() {
        val a = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(a.session.username == "android-qa" && a.session.address == "http://127.0.0.1:18875")
        val marker = ui.activity.getSharedPreferences("qa-process-marker", 0)
        val id = marker.getString("id", null)!!
        ui.waitUntil(30000) {
            graph.downloads.items.value.any { it.id == id && it.status in setOf("已缓存", "失败") }
        }
        val item = graph.downloads.items.value.first { it.id == id }
        assertEquals(item.error, "已缓存", item.status)
        val saved = DownloadJournal(ui.activity).use { it.get(a.session.cacheNamespace, id)!! }
        assertEquals(marker.getString("key", null), saved.cacheKey)
        assertEquals(marker.getString("approval", null), saved.approval)
        assertNotNull(graph.mediaCache.exact(saved.cacheKey, a.session.cacheNamespace))
        main { graph.login("http://127.0.0.1:18874", "android-qa", "RMusicAndroidQA-20261030") }
    }
}
