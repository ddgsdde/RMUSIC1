package com.rmusic.core

import java.time.Instant
import java.time.ZoneOffset
import java.util.UUID
import kotlin.math.abs
import kotlin.math.min
import org.json.JSONObject

/** Progress observations, never a hardware-frame or DAC measurement. Seek/stalls earn no time. */
class ListenSampler(track: Track, at: Long) {
    val snapshot =
        json(
            "id" to UUID.randomUUID().toString(),
            "version" to 0,
            "track_id" to track.id,
            "title" to track.title.take(500),
            "artist" to track.artist.take(500),
            "duration" to track.duration.coerceIn(0.0, 604800.0),
            "started_at" to at,
            "updated_at" to at,
            "position" to 0.0,
            "days" to JSONObject(),
            "reason" to "active",
        )

    private data class Baseline(
        val wall: Long,
        val mono: Long,
        val position: Double,
        val playing: Boolean,
    )

    private var prior: Baseline? = null
    private var saved = Long.MIN_VALUE

    fun sample(position: Double, playing: Boolean, wall: Long, mono: Long, reset: Boolean = false) {
        if (snapshot.text("reason") != "active" || !position.isFinite() || position < 0) return
        val duration = snapshot.optDouble("duration")
        val pos = if (duration > 0) position.coerceAtMost(duration) else position
        val old = prior
        if (!reset && old?.playing == true) {
            val elapsed = (mono - old.mono) / 1000.0
            val delta = pos - old.position
            if (
                elapsed > 0 &&
                    elapsed <= 30 &&
                    delta > 0 &&
                    delta <= elapsed + 1 &&
                    wall > old.wall &&
                    abs((wall - old.wall) / 1000.0 - elapsed) < 2 &&
                    old.wall >= snapshot.optLong("started_at")
            ) {
                var begin = old.wall
                val days = snapshot.child("days")
                while (begin < wall) {
                    val day = Instant.ofEpochMilli(begin).atZone(ZoneOffset.UTC).toLocalDate()
                    val midnight =
                        day.plusDays(1).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
                    val end = min(wall, midnight)
                    val value = days.optJSONObject(day.toString()) ?: json("seconds" to 0.0)
                    value.put(
                        "seconds",
                        value.optDouble("seconds") + delta * (end - begin) / (wall - old.wall),
                    )
                    value.put("last_at", if (end == midnight) end - 1 else end)
                    days.put(day.toString(), value)
                    begin = end
                }
            }
        }
        snapshot.put("position", pos).put("updated_at", maxOf(wall, snapshot.optLong("updated_at")))
        prior = Baseline(wall, mono, pos, playing)
    }

    fun flush(wall: Long, force: Boolean = false): JSONObject? {
        if (!force && saved != Long.MIN_VALUE && wall - saved in 0..4999) return null
        saved = wall
        snapshot.put("version", snapshot.optLong("version") + 1)
        return JSONObject(snapshot.toString())
    }

    fun finish(reason: String, wall: Long): JSONObject {
        require(reason in setOf("ended", "stop", "replaced", "replay", "interrupted"))
        snapshot.put("reason", reason)
        return flush(wall, true)!!
    }
}
