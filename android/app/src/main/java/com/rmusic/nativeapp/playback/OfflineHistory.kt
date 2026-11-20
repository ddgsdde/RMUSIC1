@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.playback

import com.rmusic.core.*
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject

/** Small durable outbox. Acknowledgements never erase newer versions or another account's data. */
class OfflineHistory(private val graph: AppGraph) {
    data class Status(
        val pending: Int = 0,
        val rejected: Int = 0,
        val syncing: Boolean = false,
        val error: String = "",
    )

    private val db = OfflineListenStore(graph.application)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val writes = Channel<suspend () -> Unit>(64)
    private val ready = CompletableDeferred<Unit>()
    private val gate = Mutex()
    private val mutable = MutableStateFlow(Status())
    val status = mutable.asStateFlow()
    private val prefs = graph.application.getSharedPreferences("offline-listen-client", 0)
    private val client =
        prefs.getString("id", null)
            ?: java.util.UUID.randomUUID().toString().also {
                prefs.edit().putString("id", it).commit()
            }
    @Volatile private var owner = ""
    private var sampler: ListenSampler? = null
    private var samplerOwner = ""
    private var retry: Job? = null
    private var attempt = 0
    private var lastSync = 0L
    @Volatile private var blocked = false

    init {
        scope.launch {
            try {
                db.recover()
                ready.complete(Unit)
            } catch (e: Exception) {
                ready.completeExceptionally(e)
                mutable.value = Status(error = "离线记录恢复失败：${e.message}")
                return@launch
            }
            for (write in writes) runCatching { write() }
                .onFailure { mutable.value = mutable.value.copy(error = "离线记录保存失败：${it.message}") }
        }
    }

    fun select(namespace: String) {
        finish("interrupted")
        owner = namespace
        retry?.cancel()
        attempt = 0
        blocked = false
        enqueue {
            publish()
            requestSync()
        }
    }

    fun begin(track: Track) {
        finish("replaced")
        if (owner.isBlank()) return
        samplerOwner = owner
        sampler = ListenSampler(track, System.currentTimeMillis())
    }

    fun sample(position: Double, playing: Boolean, reset: Boolean = false, force: Boolean = false) {
        val s = sampler ?: return
        val now = System.currentTimeMillis()
        s.sample(position, playing, now, android.os.SystemClock.elapsedRealtime(), reset)
        s.flush(now, force)?.let { save(samplerOwner, it) }
    }

    fun finish(reason: String) {
        sampler?.finish(reason, System.currentTimeMillis())?.let { save(samplerOwner, it) }
        sampler = null
    }

    private fun save(namespace: String, snapshot: JSONObject) = enqueue {
        db.save(namespace, snapshot)
        publish()
        if (
            snapshot.text("reason") != "active" ||
                android.os.SystemClock.elapsedRealtime() - lastSync >= 30000
        )
            requestSync()
    }

    private fun enqueue(block: suspend () -> Unit) {
        if (!writes.trySend(block).isSuccess)
            mutable.value = mutable.value.copy(error = "离线记录写入繁忙，请暂缓切歌")
    }

    suspend fun flushWrites() {
        ready.await()
        val done = CompletableDeferred<Unit>()
        writes.send { done.complete(Unit) }
        done.await()
    }

    private fun pending(namespace: String) = db.pending(namespace)

    private fun publish() {
        val (count, failed) = db.counts(owner)
        mutable.value = mutable.value.copy(pending = count, rejected = failed)
    }

    fun requestSync() {
        if (owner.isBlank() || blocked || retry?.isActive == true) return
        retry = scope.launch { sync() }
    }

    suspend fun sync() =
        withContext(Dispatchers.IO) {
            gate.withLock {
                ready.await()
                val a = graph.api.value ?: return@withLock
                val ns = a.session.cacheNamespace
                if (ns != owner || pending(ns).isEmpty()) return@withLock
                lastSync = android.os.SystemClock.elapsedRealtime()
                mutable.value = mutable.value.copy(syncing = true)
                try {
                    // Verify the public identity before sending credentials or persisted data.
                    require(a.request("status").text("core_id") == a.session.coreId) {
                        "此地址的Core身份已改变"
                    }
                    repeat(8) {
                        if (graph.api.value !== a || owner != ns) return@repeat
                        val batch = pending(ns)
                        if (batch.isEmpty()) return@repeat
                        val result =
                            a.request(
                                "stats/offline",
                                "POST",
                                json(
                                    "core_id" to a.session.coreId,
                                    "user_id" to a.session.userId,
                                    "client_id" to client,
                                    "sessions" to JSONArray(batch),
                                ),
                            )
                        for (ack in result.rows("accepted") + result.rows("rejected")) {
                            val expected =
                                batch.firstOrNull {
                                    it.text("id") == ack.text("id") &&
                                        it.optLong("version") == ack.optLong("version")
                                } ?: continue
                            db.acknowledge(ns, ack)
                        }
                    }
                    attempt = 0
                    mutable.value = mutable.value.copy(error = "")
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    mutable.value = mutable.value.copy(error = e.message ?: "稍后补传")
                    attempt++
                    blocked =
                        e is IllegalArgumentException ||
                            e is CoreFailure && e.status in setOf(401, 403)
                } finally {
                    publish()
                    mutable.value = mutable.value.copy(syncing = false)
                }
                if (
                    pending(ns).isNotEmpty() && (attempt == 0 || mutable.value.error.isNotBlank())
                ) {
                    // Backoff is suspended, never a CPU loop. Authentication/identity errors need
                    // user
                    // action.
                    val error = mutable.value.error
                    if (!blocked) {
                        delay((2000L shl attempt.coerceAtMost(5)).coerceAtMost(60000))
                        retry = null
                        requestSync()
                    }
                }
            }
        }
}
