package com.rmusic.media

import android.content.Context
import androidx.media3.common.util.UnstableApi
import androidx.media3.database.StandaloneDatabaseProvider
import androidx.media3.datasource.cache.*
import com.rmusic.core.*
import java.io.File
import org.json.JSONObject

/**
 * One cache owner per application. Cache metadata is advisory; missing spans never count as
 * complete.
 */
@UnstableApi
class MediaCache(context: Context, limit: Long) {
    private val root = File(context.cacheDir, "music-v1").apply { mkdirs() }
    private val db = context.getSharedPreferences("music-cache-index-v1", Context.MODE_PRIVATE)
    private val evictor = BudgetEvictor(limit)
    val store = SimpleCache(root, evictor, StandaloneDatabaseProvider(context))
    private val reading = java.util.concurrent.ConcurrentHashMap<String, Int>()

    fun resize(bytes: Long) {
        require(bytes >= 16L * 1024 * 1024)
        synchronized(store) { evictor.resize(store, bytes) }
    }

    @Synchronized
    fun protect(key: String) {
        reading[key] = (reading[key] ?: 0) + 1
        evictor.protected.add(key)
    }

    @Synchronized
    fun unprotect(key: String) {
        val count = reading[key] ?: 0
        if (count > 1) reading[key] = count - 1
        else {
            reading.remove(key)
            evictor.protected.remove(key)
        }
        synchronized(store) { evictor.trim(store) }
    }

    data class Entry(
        val key: String,
        val track: Track,
        val plan: StreamPlan,
        val bytes: Long,
        val namespace: String,
    )

    @Synchronized
    fun completed(key: String, track: Track, plan: StreamPlan, namespace: String, bytes: Long) {
        if (bytes <= 0 || !store.isCached(key, 0, bytes)) return
        db.edit()
            .putString(
                key,
                json(
                        "track" to track.cacheMetadata(),
                        "plan" to plan.json(),
                        "namespace" to namespace,
                        "bytes" to bytes,
                        "at" to System.currentTimeMillis(),
                    )
                    .toString(),
            )
            .apply()
        // Retire a weaker copy only after a complete, identity-equal improvement exists.
        entries(namespace)
            .filter {
                it.key != key &&
                    it.track.id == track.id &&
                    it.plan.sourceRevision == plan.sourceRevision &&
                    dominates(plan, it.plan) &&
                    !reading.containsKey(it.key)
            }
            .forEach { remove(it.key) }
    }

    @Synchronized
    fun entries(namespace: String): List<Entry> =
        db.all
            .mapNotNull { (key, value) ->
                runCatching {
                    val v = JSONObject(value as String)
                    val size = v.getLong("bytes")
                    if (size <= 0 || !store.isCached(key, 0, size)) {
                        db.edit().remove(key).apply()
                        return@runCatching null
                    }
                    if (v.getString("namespace") != namespace) return@runCatching null
                    Entry(
                        key,
                        Track(v.getJSONObject("track")),
                        StreamPlan.from(v.getJSONObject("plan")),
                        size,
                        namespace,
                    )
                }
                    .getOrNull()
            }
            .filterNotNull()

    fun exact(key: String, namespace: String) = entries(namespace).firstOrNull { it.key == key }

    @Synchronized
    fun remove(key: String) {
        if (!reading.containsKey(key)) {
            store.removeResource(key)
            db.edit().remove(key).apply()
        }
    }

    fun bytes() = store.cacheSpace

    fun release() = store.release()

    companion object {
        fun preferredComplete(
            entries: List<Entry>,
            namespace: String,
            track: Track,
            desired: StreamPlan,
            maxRate: Int,
            maxBits: Int,
            rawOnly: Boolean,
        ): Entry? {
            if (desired.codec != WireCodec.OPUS) return null
            return entries
                .filter {
                    it.namespace == namespace &&
                        it.track.id == track.id &&
                        it.plan.sourceRevision == desired.sourceRevision &&
                        it.plan.lossless &&
                        it.plan.rate <= maxRate &&
                        it.plan.bits <= maxBits &&
                        (!rawOnly || it.plan.bits <= 16 || it.plan.codec == WireCodec.PCM)
                }
                .maxWithOrNull(compareBy<Entry> { it.plan.bits }.thenBy { it.plan.rate })
        }

        fun dominates(a: StreamPlan, b: StreamPlan) =
            a.lossless &&
                (a.quality > b.quality ||
                    a.rate >= b.rate && a.bits >= b.bits && (a.rate > b.rate || a.bits > b.bits))
    }
}

/** All callbacks and resizing run under SimpleCache's monitor; no second lock order. */
@UnstableApi
private class BudgetEvictor(private var limit: Long) : CacheEvictor {
    val protected = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private val spans =
        java.util.TreeSet<CacheSpan>(
            compareBy<CacheSpan> { it.lastTouchTimestamp }.thenBy { it.key }.thenBy { it.position }
        )
    private var size = 0L

    override fun requiresCacheSpanTouches() = true

    override fun onCacheInitialized() {}

    override fun onStartFile(cache: Cache, key: String, position: Long, length: Long) {
        trim(cache, if (length > 0) length else 0)
        if (size + length.coerceAtLeast(1) > limit)
            throw Cache.CacheException("音乐缓存预算不足；正在播放的缓存暂时保留")
    }

    override fun onSpanAdded(cache: Cache, span: CacheSpan) {
        spans.add(span)
        size += span.length
        trim(cache)
    }

    override fun onSpanRemoved(cache: Cache, span: CacheSpan) {
        if (spans.remove(span)) size -= span.length
    }

    override fun onSpanTouched(cache: Cache, old: CacheSpan, new: CacheSpan) {
        spans.remove(old)
        spans.add(new)
    }

    fun resize(cache: Cache, bytes: Long) {
        limit = bytes
        trim(cache)
    }

    fun trim(cache: Cache, incoming: Long = 0) {
        while (size + incoming > limit) {
            val victim = spans.firstOrNull { it.key !in protected } ?: break
            cache.removeSpan(victim)
        }
    }
}
