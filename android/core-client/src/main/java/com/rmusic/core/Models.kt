package com.rmusic.core

import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject

fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }

fun JSONObject.rows(key: String = "items") = optJSONArray(key)?.objects().orEmpty()

fun JSONObject.text(key: String, fallback: String = "") =
    optString(key).takeUnless { it == "null" || it.isEmpty() } ?: fallback

fun JSONObject.child(key: String) = optJSONObject(key) ?: JSONObject()

fun json(vararg values: Pair<String, Any?>) =
    JSONObject().apply { values.forEach { (k, v) -> put(k, v ?: JSONObject.NULL) } }

fun stableHash(value: String) =
    MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") {
        "%02x".format(it)
    }

fun pathId(value: String) = java.net.URLEncoder.encode(value, "UTF-8").replace("+", "%20")

data class CoreSession(
    val address: String,
    val token: String,
    val coreId: String,
    val userId: String,
    val username: String,
    val admin: Boolean,
) {
    val cacheNamespace
        get() = stableHash("$coreId\u0000$userId")
}

data class Track(val raw: JSONObject) {
    val id
        get() = raw.text("id")

    val title
        get() = raw.text("title", "未命名曲目")

    val artist
        get() = raw.text("artist", "未提供艺术家")

    val album
        get() = raw.text("album")

    val cover
        get() = raw.text("cover_url")

    val duration
        get() = raw.optDouble("duration", 0.0).coerceAtLeast(0.0)

    val data
        get() = raw.child("provider_data")

    private val verified
        get() = data.child("source_audio").takeIf { it.optBoolean("verified") } ?: JSONObject()

    val rate
        get() = verified.optInt("sample_rate").takeIf { it > 0 } ?: raw.optInt("sample_rate")

    val bits
        get() = verified.optInt("bit_depth").takeIf { it > 0 } ?: raw.optInt("bit_depth")

    val format
        get() = verified.text("format", raw.text("format")).lowercase()

    val channels
        get() = raw.optInt("channels", 2).coerceIn(1, 64)

    val lossless
        get() = format in setOf("flac", "alac", "wav", "aiff", "pcm", "dsf", "dff", "dsd")

    val dsd
        get() = format in setOf("dsf", "dff", "dsd")

    val revision
        get() =
            stableHash(
                listOf(
                        id,
                        raw.text("recording_id"),
                        data.text("album_mid"),
                        data.text("source_revision"),
                        format,
                        rate,
                        bits,
                        raw.optInt("bitrate"),
                    )
                    .joinToString("\u0000")
            )

    val credits
        get() = data.rows("artist_credits")
}

/** Keep playable identity and presentation, never whole provider biographies/temporary URLs. */
fun Track.cacheMetadata(): JSONObject {
    val result = JSONObject()
    for (key in
        listOf(
            "id",
            "title",
            "artist",
            "album",
            "album_artist",
            "duration",
            "track_number",
            "disc_number",
            "source",
            "format",
            "sample_rate",
            "bit_depth",
            "channels",
            "bitrate",
            "cover_url",
            "recording_id",
            "available",
            "created_at",
        )) if (raw.has(key)) result.put(key, raw.get(key))
    val provider = JSONObject()
    for (key in
        listOf(
            "album_mid",
            "release_id",
            "artist_mids",
            "artist_credits",
            "source_audio",
            "source_revision",
            "release_date",
        )) if (data.has(key)) provider.put(key, data.get(key))
    val cached = data.child("cached_from")
    if (cached.length() > 0)
        provider.put(
            "cached_from",
            json(
                "track_id" to cached.text("track_id"),
                "source" to cached.text("source"),
                "cache_id" to cached.text("cache_id"),
            ),
        )
    return result.put("provider_data", provider)
}

data class LyricLine(val millis: Long, val text: String, val translation: String = "")

object Lyrics {
    private val stamp = Regex("\\[(\\d{1,3}):(\\d{2})(?:[.:](\\d{1,3}))?]")

    fun parse(text: String, translation: String = ""): List<LyricLine> {
        fun lines(value: String) =
            value
                .lineSequence()
                .flatMap { line ->
                    val words =
                        line.replace(stamp, "").replace(Regex("<\\d+:\\d+(?:\\.\\d+)?>"), "").trim()
                    stamp.findAll(line).map { m ->
                        val fraction = m.groupValues[3].padEnd(3, '0').take(3).toLongOrNull() ?: 0
                        (m.groupValues[1].toLong() * 60000 +
                            m.groupValues[2].toLong() * 1000 +
                            fraction) to words
                    }
                }
                .filter { it.second.isNotEmpty() }
                .toList()
        val translated = lines(translation).toMap()
        return lines(text)
            .sortedBy { it.first }
            .map { LyricLine(it.first, it.second, translated[it.first].orEmpty()) }
    }
}
