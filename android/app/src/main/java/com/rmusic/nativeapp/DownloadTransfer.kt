@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import androidx.media3.datasource.*
import androidx.media3.datasource.cache.*
import androidx.media3.datasource.okhttp.OkHttpDataSource
import com.rmusic.core.*
import com.rmusic.media.*
import com.rmusic.nativeapp.playback.ConfirmedSource
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import okhttp3.HttpUrl.Companion.toHttpUrl

/**
 * One bounded transfer. Byte resumes require the same strong entity; conversions restart safely.
 */
class DownloadTransfer(private val graph: AppGraph) {
    @Volatile private var writer: CacheWriter? = null
    @Volatile private var activeSource: DataSource? = null

    fun cancel() {
        writer?.cancel()
        val s = activeSource
        graph.scope.launch(Dispatchers.IO) { runCatching { s?.close() } }
    }

    suspend fun execute(
        a: CoreApi,
        initial: DownloadTask,
        save: (DownloadTask) -> Unit,
    ): DownloadTask {
        var task = initial
        if (task.approval.isBlank()) {
            val approved = a.request("downloads", "POST", json("track_id" to task.id)).text("id")
            require(approved.isNotBlank()) { "Core未创建下载" }
            task = task.copy(approval = approved)
            save(task)
        }
        var approved = org.json.JSONObject()
        for (i in 0 until 180) {
            currentCoroutineContext().ensureActive()
            val state = a.request("downloads/${pathId(task.approval)}")
            if (state.text("status") == "failed") error(state.text("error", "音源拒绝下载"))
            if (state.text("status") == "complete") {
                approved = state
                break
            }
            delay(1000)
        }
        require(approved.text("status") == "complete") { "Core下载尚未完成，可稍后继续" }
        val audio = approved.child("source_audio")
        require(audio.optBoolean("verified")) { "Core未报告实际批准下载格式" }
        val metadata = task.track.cacheMetadata()
        for (k in
            listOf("format", "sample_rate", "bit_depth", "channels", "bitrate", "duration")) if (
            audio.has(k)
        )
            metadata.put(k, audio.get(k))
        metadata.child("provider_data").put("source_audio", audio)
        val track = Track(metadata)
        var plan =
            PlaybackPolicy.choose(
                track,
                graph.connectivity.unmeteredWifi(),
                graph.devices.caps(),
                graph.preferences.policy.value,
            )
        if (graph.preferences.rawPcmCompatibility && plan.bits > 16 && plan.codec != WireCodec.OPUS)
            plan = plan.copy(codec = WireCodec.PCM)
        val original = a.url("api/v1/downloads/${pathId(task.approval)}/file")
        val client =
            a.http
                .newBuilder()
                .callTimeout(0, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)
                .build()
        var validator = ""
        var total = -1L
        if (plan.codec == WireCodec.ORIGINAL) {
            client
                .newCall(
                    okhttp3.Request.Builder()
                        .url(original)
                        .head()
                        .header("Authorization", "Bearer ${a.session.token}")
                        .build()
                )
                .await()
                .use { head ->
                    if (!head.isSuccessful)
                        throw CoreFailure(
                            head.code,
                            "download_unavailable",
                            "批准下载文件不可用（${head.code}），可重新申请",
                        )
                    validator =
                        head
                            .header("ETag")
                            .orEmpty()
                            .takeIf { it.startsWith('"') && it.endsWith('"') }
                            .orEmpty()
                    total = head.header("Content-Length")?.toLongOrNull() ?: -1
                }
        }
        // Stateless encoders do not promise byte-identical new responses. Never splice those.
        val entity =
            if (validator.isNotBlank()) "$validator:$total"
            else java.util.UUID.randomUUID().toString()
        plan =
            plan.copy(
                sourceRevision = stableHash("${track.revision}:approved:${task.approval}:$entity")
            )
        val key = plan.cacheKey(task.namespace, task.id)
        val url =
            if (plan.codec == WireCodec.ORIGINAL) original
            else
                plan
                    .uri(a.url("api/v1/media/${pathId(task.id)}"))
                    .toHttpUrl()
                    .newBuilder()
                    .addQueryParameter("download_id", task.approval)
                    .build()
                    .toString()
        task =
            task.copy(
                cacheKey = key,
                validator = validator,
                total = total,
                status = "下载中",
                error = "",
            )
        save(task)
        val network =
            OkHttpDataSource.Factory(client)
                .setDefaultRequestProperties(mapOf("Authorization" to "Bearer ${a.session.token}"))
        val factory =
            CacheDataSource.Factory()
                .setCache(graph.mediaCache.store)
                .setFlags(CacheDataSource.FLAG_BLOCK_ON_CACHE)
                .setUpstreamDataSourceFactory {
                    ConfirmedDownloadSource(
                        ConfirmedSource(network.createDataSource(), plan),
                        validator,
                    )
                }
        if (task.directory.isNotBlank())
            factory.setCacheWriteDataSinkFactory { PlaybackCacheSink(graph.mediaCache.store) }
        else
            factory.setCacheWriteDataSinkFactory(
                CacheDataSink.Factory().setCache(graph.mediaCache.store).setFragmentSize(256 * 1024)
            )
        val source = factory.createDataSource()
        activeSource = source
        val spec =
            DataSpec.Builder()
                .setUri(url)
                .setKey(key)
                .setFlags(DataSpec.FLAG_ALLOW_CACHE_FRAGMENTATION)
                .build()
        graph.mediaCache.protect(key)
        try {
            var length = 0L
            var last = 0L
            fun progress(bytes: Long, expected: Long) {
                length = bytes
                val now = android.os.SystemClock.elapsedRealtime()
                if (now - last >= 1000) {
                    last = now
                    save(task.copy(bytes = bytes, total = expected))
                }
            }
            if (task.directory.isBlank()) {
                writer =
                    CacheWriter(source, spec, ByteArray(128 * 1024)) { expected, bytes, _ ->
                        progress(bytes, expected)
                    }
                writer!!.cache()
                writer = null
            } else {
                val folder =
                    DocumentFile.fromTreeUri(graph.application, Uri.parse(task.directory))
                        ?: error("下载目录不可用，请重新选择")
                val ext =
                    when (plan.codec) {
                        WireCodec.OPUS -> "opus"
                        WireCodec.FLAC -> "oga"
                        WireCodec.PCM -> "wav"
                        else -> track.format
                    }
                val name =
                    ("${track.artist} - ${track.title}")
                        .replace(Regex("[\\\\/:*?\"<>|]"), "_")
                        .take(150) + ".$ext"
                val mime =
                    when (ext) {
                        "opus",
                        "oga",
                        "ogg" -> "audio/ogg"
                        "mp3" -> "audio/mpeg"
                        else -> "audio/$ext"
                    }
                // Persist the document before writing: a restarted task replaces only its own
                // partial.
                if (task.exported.isNotBlank()) {
                    val stale =
                        DocumentFile.fromSingleUri(graph.application, Uri.parse(task.exported))
                    require(stale == null || !stale.exists() || stale.delete()) {
                        "无法清理上次未完成导出，请检查目录权限"
                    }
                }
                val file = folder.createFile(mime, name) ?: error("无法创建导出文件")
                task = task.copy(exported = file.uri.toString())
                save(task)
                var complete = false
                try {
                    DataSourceInputStream(source, spec).use { input ->
                        graph.application.contentResolver.openOutputStream(file.uri, "w")!!.use {
                            out ->
                            val buffer = ByteArray(128 * 1024)
                            var bytes = 0L
                            while (true) {
                                currentCoroutineContext().ensureActive()
                                val n = input.read(buffer)
                                if (n < 0) break
                                out.write(buffer, 0, n)
                                bytes += n
                                progress(bytes, total)
                            }
                        }
                    }
                    complete = true
                } finally {
                    if (!complete) runCatching { file.delete() }
                }
            }
            currentCoroutineContext().ensureActive()
            graph.mediaCache.completed(key, track, plan, task.namespace, length)
            val cached = length > 0 && graph.mediaCache.store.isCached(key, 0, length)
            require(task.directory.isNotBlank() || cached) { "缓存未完整保存，请检查预算和存储空间" }
            return task.copy(
                status = if (task.directory.isBlank()) "已缓存" else if (cached) "已导出并缓存" else "已导出",
                bytes = length,
                total = length,
                error = "",
            )
        } finally {
            writer = null
            activeSource = null
            runCatching { source.close() }
            graph.mediaCache.unprotect(key)
        }
    }
}

private class ConfirmedDownloadSource(
    private val delegate: DataSource,
    private val validator: String,
) : DataSource by delegate {
    override fun open(spec: DataSpec): Long {
        val headers =
            if (validator.isNotBlank()) spec.httpRequestHeaders + mapOf("If-Range" to validator)
            else spec.httpRequestHeaders
        val size = delegate.open(spec.buildUpon().setHttpRequestHeaders(headers).build())
        if (validator.isNotBlank()) {
            val actual =
                delegate.responseHeaders.entries
                    .firstOrNull { it.key.equals("ETag", true) }
                    ?.value
                    ?.firstOrNull()
            if (actual != validator) {
                delegate.close()
                throw IOException("下载来源已变化，已停止拼接；请继续以重新校验")
            }
        }
        return size
    }
}
