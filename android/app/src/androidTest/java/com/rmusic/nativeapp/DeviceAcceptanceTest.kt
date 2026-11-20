@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.os.Build
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.rmusic.core.*
import com.rmusic.media.OutputMode
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Host observes phase.json, sends screen-off and remote pause/play, then wakes the device. */
class DeviceAcceptanceTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val graph
        get() = (ui.activity.application as RMusicApplication).graph

    private fun main(block: suspend () -> Unit) = runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

    @Test
    fun screenOffKeepsRealOutputAndRemoteControl() {
        val a = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(a.session.username == "android-qa" && a.session.address == "http://127.0.0.1:18874")
        check(Build.MANUFACTURER != "unknown")
        val dir = File(ui.activity.filesDir, "device-qa").apply { mkdirs() }
        val old = graph.preferences.policy.value
        val local = runBlocking {
            a.request("zones").rows().first {
                it.text("endpoint_id") == graph.preferences.endpointId
            }
        }
        val zone = local.text("id")
        val tracks = runBlocking {
            a.request("library/tracks?limit=50").rows().map(::Track).filter {
                it.title in setOf("Android QA 1", "Android QA 2")
            }
        }
        assertEquals(2, tracks.size)
        val samples = JSONArray()
        val details =
            json(
                "manufacturer" to Build.MANUFACTURER,
                "model" to Build.MODEL,
                "api" to Build.VERSION.SDK_INT,
                "endpoint_id" to graph.preferences.endpointId,
                "zone_id" to zone,
            )
        try {
            main {
                graph.preferences.save(
                    old.copy(
                        networkMode = "auto",
                        mobileOpus = false,
                        outputMode = OutputMode.AUTO,
                        deviceId = 0,
                    )
                )
                graph.runtime.selectedZone.value = zone
                a.request(
                    "zones/${pathId(zone)}",
                    "PATCH",
                    json("repeat" to "all", "shuffle" to false),
                )
                graph.runtime.play(tracks)
            }
            ui.waitUntil(20000) {
                graph.runtime.state.value.playing && graph.runtime.state.value.position > 1
            }
            File(dir, "phase.json")
                .writeText(json("phase" to "ready", "zone_id" to zone).toString())
            repeat(15) { i ->
                runBlocking { delay(5000) }
                val s = graph.runtime.state.value
                samples.put(
                    json(
                        "seconds" to (i + 1) * 5,
                        "foreground" to graph.runtime.foreground,
                        "playing" to s.playing,
                        "buffering" to s.buffering,
                        "position" to s.position,
                        "track_id" to s.track?.id,
                        "transport" to s.transport,
                        "decoder" to s.decoder,
                        "error" to s.error,
                        "output" to graph.devices.snapshot(),
                    )
                )
                File(dir, "report.json")
                    .writeText(JSONObject(details.toString()).put("samples", samples).toString(2))
                assertTrue("Unexpected playback error: ${s.error}", s.error.isBlank())
            }
            val rows = samples.objects()
            assertTrue(
                "Host must really turn the screen off",
                rows.count { !it.optBoolean("foreground") } >= 8,
            )
            assertTrue(
                "Playback continues in background",
                rows.count { !it.optBoolean("foreground") && it.optBoolean("playing") } >= 7,
            )
            assertTrue(
                "Core must advance real tracks",
                rows.map { it.text("track_id") }.toSet().size >= 2,
            )
            assertTrue(rows.any { it.child("output").optInt("rate") > 0 })
            File(dir, "phase.json")
                .writeText(json("phase" to "complete", "zone_id" to zone).toString())
        } finally {
            main {
                runCatching { a.command(zone, "pause") }
                runCatching {
                    a.request(
                        "zones/${pathId(zone)}",
                        "PATCH",
                        json(
                            "repeat" to local.text("repeat", "none"),
                            "shuffle" to local.optBoolean("shuffle"),
                        ),
                    )
                }
                graph.preferences.save(old)
            }
        }
    }
}
