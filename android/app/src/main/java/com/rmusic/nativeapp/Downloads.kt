@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.content.Intent
import android.net.Uri
import com.rmusic.core.*
import java.io.IOException
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Serial orchestration only. The journal and byte transfer have separate ownership. */
class Downloads(private val graph: AppGraph) {
    private val journal = DownloadJournal(graph.application)
    private val transfer = DownloadTransfer(graph)
    private val io =
        CoroutineScope(
            SupervisorJob() +
                Dispatchers.IO.limitedParallelism(1) +
                CoroutineExceptionHandler { _, error -> graph.report("下载任务保存失败：${error.message}") }
        )
    private val ready = io.async { journal.recover() }
    private val prefs = graph.application.getSharedPreferences("download-journal-v1", 0)
    private val mutable = MutableStateFlow<List<DownloadItem>>(emptyList())
    val items = mutable.asStateFlow()
    private val queue = java.util.concurrent.ConcurrentLinkedQueue<Pair<String, String>>()
    @Volatile private var worker: Job? = null
    @Volatile private var current: Pair<String, String>? = null
    @Volatile private var epoch = 0L
    @Volatile private var namespace = ""
    var directory: Uri?
        get() = prefs.getString("directory", null)?.let(Uri::parse)
        set(v) {
            prefs.edit().putString("directory", v?.toString()).apply()
        }

    fun selectNamespace(value: String) {
        namespace = value
        mutable.value = emptyList()
        io.launch {
            ready.await()
            publish(value)
            if (
                namespace == value &&
                    value.isNotBlank() &&
                    graph.preferences.resumeDownloads &&
                    graph.runtime.foreground
            )
                resumeInterrupted()
        }
    }

    private fun publish(owner: String) {
        if (namespace == owner)
            mutable.value =
                journal.rows(owner).map { task ->
                    val item = task.item()
                    if (
                        task.status in setOf("已缓存", "已导出并缓存") &&
                            task.bytes > 0 &&
                            !graph.mediaCache.store.isCached(task.cacheKey, 0, task.bytes)
                    ) {
                        item.copy(
                            status = if (task.status == "已缓存") "缓存已回收" else "已导出",
                            resumable = true,
                        )
                    } else item
                }
    }

    private fun save(task: DownloadTask) {
        journal.put(task)
        publish(task.namespace)
    }

    fun refresh() {
        val ns = namespace
        io.launch {
            ready.await()
            publish(ns)
        }
    }

    private fun activeKey(id: String) = id to namespace

    fun enqueue(track: Track) {
        val api = graph.api.value ?: return
        val ns = api.session.cacheNamespace
        val target = directory?.toString().orEmpty()
        val submitted = epoch
        io.launch {
            ready.await()
            if (graph.api.value !== api || submitted != epoch) return@launch
            val old = journal.get(ns, track.id)
            if (
                old?.status in setOf("等待", "下载中") &&
                    (queue.contains(track.id to ns) ||
                        current == (track.id to ns) && worker?.isActive == true)
            )
                return@launch
            val task =
                DownloadTask(
                    track,
                    ns,
                    target,
                    approval = old?.approval.orEmpty(),
                    cacheKey = old?.cacheKey.orEmpty(),
                )
            save(task)
            offer(track.id, ns)
        }
    }

    fun resume(id: String, reapprove: Boolean = false) {
        val ns = namespace
        io.launch {
            ready.await()
            val t = journal.get(ns, id) ?: return@launch
            if (ns != namespace || current == (id to ns) || t.status in setOf("下载中", "等待"))
                return@launch
            save(
                t.copy(
                    status = "等待",
                    error = "",
                    approval = if (reapprove) "" else t.approval,
                    request = java.util.UUID.randomUUID().toString(),
                )
            )
            offer(id, ns)
        }
    }

    fun resumeInterrupted() {
        val ns = namespace
        io.launch {
            ready.await()
            journal
                .rows(ns)
                .filter { it.status == "已中断，点击继续" || it.status == "等待网络" }
                .reversed()
                .forEach {
                    if (ns == namespace) {
                        save(
                            it.copy(
                                status = "等待",
                                error = "",
                                request = java.util.UUID.randomUUID().toString(),
                            )
                        )
                        offer(it.id, ns)
                    }
                }
        }
    }

    private suspend fun offer(id: String, ns: String) {
        if (ns != namespace) return
        val key = id to ns
        if (!queue.contains(key) && (current != key || worker?.isActive != true)) queue.add(key)
        withContext(Dispatchers.Main) {
            runCatching {
                graph.application.startForegroundService(
                    Intent(graph.application, DownloadService::class.java)
                )
            }
                .onFailure { e ->
                    io.launch {
                        journal.get(ns, id)?.let {
                            save(it.copy(status = "已中断，点击继续", error = "请打开App后继续下载"))
                        }
                    }
                    queue.remove(key)
                }
        }
    }

    fun pause(id: String) {
        val key = activeKey(id)
        val request = mutable.value.firstOrNull { it.id == id }?.request
        queue.remove(key)
        if (current == key) interrupt(false, false, continuePending = true)
        io.launch {
            ready.await()
            journal.get(key.second, id)?.let {
                if (it.request == request) save(it.copy(status = "已暂停", error = ""))
            }
        }
    }

    fun cancel() = interrupt(true, true)

    fun serviceStopped() = interrupt(false, false)

    private fun interrupt(clear: Boolean, stopOwner: Boolean, continuePending: Boolean = false) {
        val ns = namespace
        val requests = mutable.value.associate { it.id to it.request }
        val old = worker
        val stamp = ++epoch
        val active = current
        if (clear) queue.clear()
        transfer.cancel()
        old?.cancel()
        io.launch {
            ready.await()
            old?.join()
            if (clear)
                journal
                    .rows(ns)
                    .filter {
                        it.request == requests[it.id] && it.status in setOf("下载中", "等待", "等待网络")
                    }
                    .forEach { save(it.copy(status = "已取消", error = "")) }
            else if (active != null)
                journal.get(active.second, active.first)?.let {
                    if (it.request == requests[it.id] && it.status == "下载中")
                        save(it.copy(status = "已中断，点击继续"))
                }
            if (stamp == epoch)
                withContext(Dispatchers.Main) {
                    worker = null
                    current = null
                    val host = DownloadService.current.get()
                    if (continuePending && queue.isNotEmpty() && host != null)
                        run { host.finishIfIdle() }
                    else if ((stopOwner || continuePending) && queue.isEmpty()) host?.finishIfIdle()
                }
        }
    }

    val idle
        get() = queue.isEmpty() && worker?.isActive != true

    fun run(done: () -> Unit) {
        if (worker?.isActive == true) return
        val stamp = ++epoch
        val previous = worker
        worker =
            graph.scope.launch {
                previous?.join()
                withContext(Dispatchers.IO) {
                    ready.await()
                    try {
                        while (isActive && stamp == epoch) {
                            val key = queue.poll() ?: break
                            current = key
                            val t = journal.get(key.second, key.first) ?: continue
                            if (t.status != "等待") continue
                            val a = graph.api.value
                            if (a == null || a.session.cacheNamespace != key.second) continue
                            try {
                                for (attempt in 0..2) {
                                    val latest = journal.get(key.second, key.first) ?: t
                                    try {
                                        require(
                                            a.request("status").text("core_id") == a.session.coreId
                                        ) {
                                            "此地址的Core身份已改变"
                                        }
                                        save(latest.copy(status = "下载中", error = ""))
                                        val result =
                                            transfer.execute(a, latest) { next ->
                                                if (stamp == epoch) save(next)
                                            }
                                        ensureActive()
                                        if (stamp == epoch) save(result)
                                        break
                                    } catch (e: Exception) {
                                        if (e is CancellationException) throw e
                                        ensureActive()
                                        val failed = journal.get(key.second, key.first) ?: latest
                                        val recoverable = e is IOException && e !is CoreFailure
                                        save(
                                            failed.copy(
                                                status = if (recoverable) "等待网络" else "失败",
                                                error = e.message ?: "下载失败",
                                            )
                                        )
                                        if (!recoverable || attempt == 2) break
                                        delay(2000L shl attempt)
                                    }
                                }
                            } finally {
                                current = null
                            }
                        }
                    } finally {
                        if (stamp == epoch) {
                            worker = null
                            current = null
                            publish(namespace)
                            withContext(NonCancellable + Dispatchers.Main) {
                                if (queue.isNotEmpty()) run(done) else done()
                            }
                        }
                    }
                }
            }
    }
}
