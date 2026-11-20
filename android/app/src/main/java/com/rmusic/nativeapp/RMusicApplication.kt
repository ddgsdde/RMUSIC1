package com.rmusic.nativeapp

import android.app.Application
import androidx.media3.common.util.UnstableApi
import com.rmusic.core.*
import com.rmusic.media.*
import com.rmusic.nativeapp.playback.OfflineHistory
import com.rmusic.nativeapp.playback.PlaybackRecovery
import com.rmusic.nativeapp.playback.PlaybackRuntime
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

@UnstableApi
class RMusicApplication : Application() {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph(this)
    }
}

@UnstableApi
class AppGraph(val application: Application) {
    val scope =
        CoroutineScope(
            SupervisorJob() +
                Dispatchers.Main.immediate +
                CoroutineExceptionHandler { _, error -> report(error.message ?: "后台任务失败，请重试") }
        )
    val preferences = LocalPreferences(application)
    val connectivity = Connectivity(application)
    val vault = SessionVault(application)
    val mediaCache = MediaCache(application, preferences.musicBytes)
    val devices = OutputDevices(application, preferences)
    val repository = MusicRepository(application)
    private val active = MutableStateFlow<CoreApi?>(null)
    val api = active.asStateFlow()
    private val failure = MutableStateFlow("")
    val connectionError = failure.asStateFlow()
    val history = OfflineHistory(this)
    val recovery = PlaybackRecovery(this)
    val runtime = PlaybackRuntime(this)
    val downloads = Downloads(this)
    val images by lazy { artworkLoader() }
    private var reconnect: Job? = null
    @Volatile private var verified = false

    fun foreground() {
        runtime.foreground = true
        history.requestSync()
        if (!verified) recoverConnection()
        if (preferences.resumeDownloads) downloads.resumeInterrupted()
    }

    private fun recoverConnection() {
        if (verified || reconnect?.isActive == true) return
        val a = active.value ?: return
        reconnect = scope.launch {
            var attempt = 0
            while (isActive && active.value === a && !verified) {
                try {
                    require(a.request("status").text("core_id") == a.session.coreId) {
                        "Core身份已改变，请检查地址"
                    }
                    require(a.request("auth/me").text("id") == a.session.userId) { "登录身份已改变" }
                    verified = true
                    failure.value = ""
                    runtime.connect(a)
                    history.requestSync()
                    if (preferences.resumeDownloads && runtime.foreground)
                        downloads.resumeInterrupted()
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    failure.value = e.message ?: "Core暂时无法连接，缓存仍可用"
                    if (
                        e is IllegalArgumentException ||
                            e is CoreFailure && e.status in setOf(401, 403)
                    )
                        break
                    delay(
                        (2000L shl attempt.coerceAtMost(5)).coerceAtMost(
                            if (runtime.foreground) 30000 else 60000
                        )
                    )
                    attempt++
                }
            }
        }
    }

    init {
        scope.launch {
            connectivity.changes().collect {
                history.requestSync()
                recoverConnection()
                runtime.networkChanged()
                if (runtime.foreground) downloads.resumeInterrupted()
            }
        }
        scope.launch {
            vault.load()?.let { session ->
                val next = CoreApi(session)
                try {
                    attach(next, false)
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    if (e is java.io.IOException && e !is CoreFailure) {
                        repository.open(next)
                        active.value = next
                        downloads.selectNamespace(next.session.cacheNamespace)
                        history.select(next.session.cacheNamespace)
                        recovery.select(next.session.cacheNamespace)
                        runtime.accountOnly(next)
                        recoverConnection()
                    }
                    failure.value = e.message ?: "Core暂时无法连接"
                }
            }
        }
    }

    suspend fun login(address: String, user: String, password: String, totp: String = "") {
        val (api, _) = CoreApi.login(address, user, password, totp)
        attach(api, true)
    }

    private suspend fun attach(next: CoreApi, save: Boolean) {
        val status = next.request("status")
        require(status.text("core_id") == next.session.coreId) { "此地址指向另一台Core，请检查连接" }
        val me = next.request("auth/me")
        require(me.text("id") == next.session.userId) { "登录身份已变化，请重新登录" }
        if (save) withContext(Dispatchers.IO) { vault.save(next.session) }
        reconnect?.cancel()
        verified = true
        downloads.cancel()
        runtime.disconnect()
        active.value?.close()
        repository.open(next)
        active.value = next
        downloads.selectNamespace(next.session.cacheNamespace)
        history.select(next.session.cacheNamespace)
        recovery.select(next.session.cacheNamespace)
        failure.value = ""
        runtime.connect(next)
    }

    fun logout() {
        scope.launch {
            reconnect?.cancel()
            verified = false
            downloads.cancel()
            runtime.disconnect()
            active.value?.let {
                runCatching { it.request("auth/logout", "POST") }
                it.close()
            }
            history.select("")
            recovery.select("")
            active.value = null
            downloads.selectNamespace("")
            withContext(Dispatchers.IO) { vault.clear() }
            repository.clearMemory()
        }
    }

    fun report(message: String) {
        failure.value = message
    }
}
