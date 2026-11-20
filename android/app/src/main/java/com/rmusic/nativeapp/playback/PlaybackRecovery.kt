@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.playback

import android.content.Context
import com.rmusic.core.*
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

/** A bookmark is an offer to resume, never a command to autoplay or to replace a remote queue. */
class PlaybackRecovery(private val graph: AppGraph) {
    data class Bookmark(
        val track: Track,
        val position: Double,
        val zone: String,
        val cacheKey: String,
        val offline: Boolean,
        val updated: Long,
    )

    private val disk =
        graph.application.getSharedPreferences("playback-bookmarks-v1", Context.MODE_PRIVATE)
    private val mutable = MutableStateFlow<Bookmark?>(null)
    val offered = mutable.asStateFlow()
    private var namespace = ""
    private var savedAt = 0L
    private var lastTrack = ""

    fun select(value: String) {
        namespace = value
        savedAt = 0
        lastTrack = ""
        mutable.value =
            if (value.isBlank()) null
            else
                runCatching {
                    val v = JSONObject(disk.getString(value, null) ?: return@runCatching null)
                    val t = Track(v.child("track"))
                    val position = v.optDouble("position")
                    if (
                        t.id.isBlank() ||
                            !position.isFinite() ||
                            position < 0 ||
                            t.duration > 0 && position >= t.duration - 1
                    )
                        null
                    else
                        Bookmark(
                            t,
                            position,
                            v.text("zone"),
                            v.text("cache"),
                            v.optBoolean("offline"),
                            v.optLong("updated"),
                        )
                }
                    .getOrNull()
    }

    fun record(
        track: Track,
        position: Double,
        zone: String,
        key: String,
        offline: Boolean,
        force: Boolean = false,
    ) {
        if (namespace.isBlank() || !position.isFinite() || position < 0) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (!force && track.id == lastTrack && now - savedAt < 5000) return
        savedAt = now
        lastTrack = track.id
        // apply() serializes disk writes in Android; no media URL, ticket or password is stored.
        disk
            .edit()
            .putString(
                namespace,
                json(
                        "track" to track.cacheMetadata(),
                        "position" to position,
                        "zone" to zone,
                        "cache" to key,
                        "offline" to offline,
                        "updated" to System.currentTimeMillis(),
                    )
                    .toString(),
            )
            .apply()
    }

    fun dismiss() {
        mutable.value = null
    }

    fun clear() {
        if (namespace.isNotBlank()) disk.edit().remove(namespace).apply()
        mutable.value = null
    }

    suspend fun resume() {
        val mark = mutable.value ?: return
        val a = graph.api.value ?: error("请先连接缓存所属的Core账号")
        require(a.session.cacheNamespace == namespace) { "播放记录属于另一Core账号" }
        val cache = withContext(Dispatchers.IO) { graph.mediaCache.exact(mark.cacheKey, namespace) }
        if (mark.offline) {
            require(cache != null) { "这首歌的完整缓存已被回收，请联网重新播放" }
            graph.runtime.playCached(cache, mark.position)
        } else {
            // A fresh Core snapshot wins over stale device bookmarks, including other controllers.
            val local =
                a.request("zones").rows().firstOrNull {
                    it.text("endpoint_id") == graph.preferences.endpointId
                } ?: error("本机音区尚未准备好")
            require(local.child("track").text("id") == mark.track.id) { "Core播放列表已变化，请从当前列表继续" }
            require(local.text("status") != "playing") { "Core已在播放，请使用当前播放栏" }
            graph.runtime.selectedZone.value = local.text("id")
            if (cache != null) graph.runtime.offerCache(cache)
            require(local.text("playback_id").isNotBlank()) { "该播放实例已结束，请从当前列表重新播放" }
            // Core validates identity and applies position under one lock. A second controller
            // changing tracks between the read and this command cannot seek the wrong song.
            a.command(
                local.text("id"),
                "play",
                json(
                    "position" to mark.position,
                    "control" to
                        json(
                            "endpoint_id" to graph.preferences.endpointId,
                            "track_id" to mark.track.id,
                            "playback_id" to local.text("playback_id"),
                        ),
                ),
            )
        }
        dismiss()
    }
}
