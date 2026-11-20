@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.net.TrafficStats
import android.os.Process
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import com.rmusic.core.*
import com.rmusic.media.OutputMode
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class LanAcceptanceTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun main(block: suspend () -> Unit) = runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

    private fun waitFor(block: () -> Boolean) = ui.waitUntil(20000, block)

    @Test
    fun privateWifiCarriesAudioAndTravelPolicyIsExplicit() {
        val before = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(
            before.session.username == "android-qa" &&
                before.session.address == "http://127.0.0.1:18874"
        )
        val address =
            InstrumentationRegistry.getArguments().getString("qa_lan")
                ?: error("Private QA LAN address required")
        val host = java.net.URI(address).host
        require(CoreApi.isPrivateHost(host) && host != "127.0.0.1" && host != "localhost")
        assertTrue("Connect the phone to unmetered Wi-Fi first", graph.connectivity.unmeteredWifi())
        val old = graph.preferences.policy.value
        val out = File(ui.activity.filesDir, "device-qa").apply { mkdirs() }
        val report = json("address" to address, "actual_wifi" to true)
        try {
            main { graph.login(address, "android-qa", "RMusicAndroidQA-20261030") }
            val a = graph.api.value!!
            assertEquals(before.session.coreId, a.session.coreId)
            waitFor { graph.runtime.selectedZone.value.isNotBlank() }
            val tracks = runBlocking { a.request("library/tracks?limit=50").rows().map(::Track) }
            val t = tracks.first { it.title == "Android QA 3" }
            graph.mediaCache
                .entries(a.session.cacheNamespace)
                .filter { it.track.id == t.id }
                .forEach { graph.mediaCache.remove(it.key) }
            val rx = TrafficStats.getUidRxBytes(Process.myUid())
            main {
                graph.preferences.save(
                    old.copy(
                        networkMode = "auto",
                        mobileOpus = true,
                        outputMode = OutputMode.AUTO,
                        deviceId = 0,
                    )
                )
                graph.runtime.play(listOf(t))
            }
            waitFor { graph.runtime.state.value.playing && graph.runtime.state.value.position > 1 }
            assertFalse(graph.runtime.state.value.cached)
            assertFalse(graph.runtime.state.value.transport.startsWith("Opus"))
            report.put(
                "wifi",
                json(
                    "transport" to graph.runtime.state.value.transport,
                    "output" to graph.devices.snapshot(),
                    "received_bytes" to TrafficStats.getUidRxBytes(Process.myUid()) - rx,
                ),
            )
            assertTrue(report.child("wifi").optLong("received_bytes") > 65536)
            val mobile = tracks.first { it.title == "Android QA 1" }
            graph.mediaCache
                .entries(a.session.cacheNamespace)
                .filter { it.track.id == mobile.id }
                .forEach { graph.mediaCache.remove(it.key) }
            main {
                graph.preferences.save(
                    graph.preferences.policy.value.copy(networkMode = "mobile", mobileOpus = true)
                )
                graph.runtime.play(listOf(mobile))
            }
            waitFor { graph.runtime.state.value.playing && graph.runtime.state.value.position > 1 }
            if (graph.devices.caps().maxRate >= 48000)
                assertTrue(graph.runtime.state.value.transport.startsWith("Opus"))
            report.put(
                "manual_travel_policy_over_wifi",
                json(
                    "transport" to graph.runtime.state.value.transport,
                    "output" to graph.devices.snapshot(),
                ),
            )
            File(out, "lan.json").writeText(report.toString(2))
        } finally {
            main {
                runCatching { graph.runtime.control("pause") }
                graph.preferences.save(old)
                graph.login("http://127.0.0.1:18874", "android-qa", "RMusicAndroidQA-20261030")
            }
        }
    }
}
