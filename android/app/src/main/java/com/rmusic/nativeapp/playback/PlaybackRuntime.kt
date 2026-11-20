package com.rmusic.nativeapp.playback

import android.content.Intent
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import androidx.media3.common.*
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.*
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.*
import androidx.media3.exoplayer.audio.*
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import com.rmusic.core.*
import com.rmusic.media.*
import com.rmusic.nativeapp.AppGraph
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import org.json.JSONObject

data class Playing(
    val track: Track? = null,
    val position: Double = 0.0,
    val playing: Boolean = false,
    val buffering: Boolean = false,
    val error: String = "",
    val transport: String = "",
    val cached: Boolean = false,
    val decoder: String = "",
    val output: JSONObject = JSONObject(),
)

@UnstableApi
class PlaybackRuntime(private val graph: AppGraph) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val mutable = MutableStateFlow(Playing())
    val state = mutable.asStateFlow()
    private val zoneState = MutableStateFlow<List<JSONObject>>(emptyList())
    val zones = zoneState.asStateFlow()
    val selectedZone = MutableStateFlow("")
    private var socket: EndpointSocket? = null
    private var monitor: EndpointSocket? = null
    private var creatingZone = false
    private var localRequests = 0
    var foreground = false
        set(value) {
            field = value
            if (
                value &&
                    api != null &&
                    graph.preferences.backgroundControl &&
                    PlaybackService.current.get() == null
            )
                runCatching {
                    graph.application.startForegroundService(
                        Intent(graph.application, PlaybackService::class.java)
                    )
                }
                    .onFailure { graph.report("后台控制尚未启动，请重试") }
        }

    private var lastSignal = ""
    private var lastReport = 0L
    private var readyToReport = false
    private var playerSignature = ""
    var onPlayerChanged: ((ExoPlayer) -> Unit)? = null
    private var api: CoreApi? = null
    private var preparing: Job? = null
    private var idleRelease: Job? = null
    private var lastStateSignature = ""
    private var reporting: Job? = null
    private var generation = 0L
    private var command = JSONObject()
    private var origin = 0.0
    private var cacheKey: String? = null
    private var livePlan: StreamPlan? = null
    private var activeMedia = ""
    private var ended = ""
    private var offline: MediaCache.Entry? = null
    private var requestedCache: MediaCache.Entry? = null
    private var currentCache: MediaCache.Entry? = null
    private var absoluteTimeline = false
    private var retry = 0
    @Volatile private var pcmFallback = false
    var player: ExoPlayer? = null
        private set

    private val manager = graph.application.getSystemService(AudioManager::class.java)
    private val deviceChanges =
        object : AudioDeviceCallback() {
            override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) {
                if (
                    removed.any { it.id == graph.preferences.policy.value.deviceId } &&
                        player != null
                ) {
                    player?.stop()
                    mutable.value = mutable.value.copy(playing = false, error = "所选输出已断开，请选择设备后继续")
                    scope.launch { runCatching { localControl("pause") } }
                }
            }
        }

    init {
        manager.registerAudioDeviceCallback(deviceChanges, Handler(Looper.getMainLooper()))
    }

    fun accountOnly(next: CoreApi) {
        api = next
    }

    fun connect(next: CoreApi) {
        socket?.close()
        monitor?.close()
        api = next
        if (graph.preferences.backgroundControl)
            runCatching {
                graph.application.startForegroundService(
                    Intent(graph.application, PlaybackService::class.java)
                )
            }
                .onFailure { graph.report("请打开 App 以启用后台远程播放") }
        val endpoint =
            json(
                "id" to graph.preferences.endpointId,
                "name" to graph.preferences.clientName,
                "kind" to "android",
                "platform" to "android",
                "online" to true,
                "capabilities" to
                    json(
                        "pcm" to true,
                        "playback_instance" to true,
                        "media_url_api_relative" to true,
                        "continuous_pcm" to false,
                        "continuous_recovery" to false,
                        "group_start_v1" to false,
                        "group_output_v2" to false,
                        "strict_sync" to false,
                        "dsp" to false,
                        "host_clock_v1" to false,
                    ),
            )
        socket =
            EndpointSocket(
                    next,
                    endpoint,
                    onCommand = { v ->
                        when (v.text("type")) {
                            "registered" -> {
                                val stamp = generation
                                val local = runCatching {
                                    next.request("zones").rows().firstOrNull {
                                        it.text("endpoint_id") == graph.preferences.endpointId
                                    }
                                }
                                    .getOrNull()
                                if (
                                    stamp == generation &&
                                        localRequests == 0 &&
                                        offline == null &&
                                        local != null &&
                                        local.text("status") == "playing" &&
                                        local.text("playback_id") != activeMedia
                                ) {
                                    start(
                                        json(
                                            "type" to "play",
                                            "zone_id" to local.text("id"),
                                            "playback_id" to local.text("playback_id"),
                                            "track" to local.child("track"),
                                            "position" to local.optDouble("position"),
                                        )
                                    )
                                }
                            }
                            "play",
                            "seek" -> start(v)
                            "pause" -> {
                                command.put("paused", true)
                                player?.pause()
                                tick()
                                releaseWhenIdle()
                            }
                            "stop" -> {
                                stopPlayer()
                                mutable.value =
                                    mutable.value.copy(playing = false, buffering = false)
                                tick()
                            }
                            "volume",
                            "mute" -> {
                                graph.devices.volume(
                                    if (v.optBoolean("muted")) 0f
                                    else v.optDouble("volume", 1.0).toFloat()
                                )
                                tick()
                            }
                            "session_expired" -> {
                                stopPlayer()
                                graph.report("Core会话已失效，请重新登录")
                            }
                            "remove_endpoint" -> {
                                disconnect()
                                graph.report("此播放设备已被Core移除")
                            }
                        }
                    },
                )
                .also { it.connect() }
        monitor =
            EndpointSocket(
                    next,
                    JSONObject(),
                    { event ->
                        when (event.text("type")) {
                            "zones",
                            "resync" -> {
                                val rows =
                                    event.optJSONArray("data")?.objects()
                                        ?: event.child("data").rows()
                                zoneState.value = rows
                                val local = rows.firstOrNull {
                                    it.text("endpoint_id") == graph.preferences.endpointId
                                }
                                if (selectedZone.value.isBlank() && local != null)
                                    selectedZone.value = local.text("id")
                            }
                            "zone_progress" -> {
                                val data = event.child("data")
                                zoneState.value =
                                    zoneState.value.map { z ->
                                        if (
                                            z.text("id") == data.text("id") &&
                                                z.text("playback_id") == data.text("playback_id")
                                        )
                                            JSONObject(z.toString())
                                                .put("position", data.optDouble("position"))
                                                .put("status", data.text("status"))
                                        else z
                                    }
                            }
                            "endpoints" -> {
                                if (
                                    !creatingZone &&
                                        zoneState.value.none {
                                            it.text("endpoint_id") == graph.preferences.endpointId
                                        }
                                )
                                    ensureZone(next)
                            }
                            "session_expired" -> graph.report("Core会话已失效，请重新登录")
                        }
                    },
                    eventsOnly = true,
                )
                .also { it.connect() }
        ensureZone(next)
        startTicker()
    }

    private fun startTicker() {
        if (reporting?.isActive == true) return
        reporting = scope.launch {
            while (isActive) {
                tick()
                delay(if (foreground) 1000 else 3000)
            }
        }
    }

    private fun ensureZone(next: CoreApi) {
        if (creatingZone) return
        creatingZone = true
        scope.launch {
            try {
                repeat(12) {
                    if (api !== next) return@launch
                    val list = next.request("zones").rows()
                    zoneState.value = list
                    val local = list.firstOrNull {
                        it.text("endpoint_id") == graph.preferences.endpointId
                    }
                    if (local != null) {
                        if (selectedZone.value.isBlank()) selectedZone.value = local.text("id")
                        return@launch
                    }
                    val endpoints = next.request("endpoints").rows()
                    if (endpoints.any { it.text("id") == graph.preferences.endpointId }) {
                        val z =
                            next.request(
                                "zones",
                                "POST",
                                json(
                                    "name" to graph.preferences.clientName,
                                    "endpoint_id" to graph.preferences.endpointId,
                                ),
                            )
                        if (selectedZone.value.isBlank()) selectedZone.value = z.text("id")
                        return@launch
                    }
                    delay(500)
                }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
            } finally {
                creatingZone = false
            }
        }
    }

    fun offerCache(entry: MediaCache.Entry) {
        requestedCache = entry
    }

    suspend fun playCached(
        entry: MediaCache.Entry,
        position: Double = 0.0,
        forceOffline: Boolean = false,
    ) {
        val account = graph.api.value ?: error("请先登录缓存所属的Core账号")
        require(entry.namespace == account.session.cacheNamespace) { "此缓存属于其他Core账号" }
        if (api == null) api = account
        startTicker()
        val local =
            zones.value.firstOrNull { it.text("endpoint_id") == graph.preferences.endpointId }
        if (!forceOffline && socket?.online?.value == true && local != null) {
            selectedZone.value = local.text("id")
            requestedCache = entry
            try {
                if (position <= 0.0) play(listOf(entry.track))
                else {
                    localRequests++
                    try {
                        account.command(local.text("id"), "pause")
                        account.request(
                            "zones/${pathId(local.text("id"))}/queue",
                            "PUT",
                            json(
                                "track_ids" to org.json.JSONArray(listOf(entry.track.id)),
                                "queue_index" to 0,
                            ),
                        )
                        account.command(local.text("id"), "seek", json("position" to position))
                        account.command(local.text("id"), "play")
                    } finally {
                        localRequests--
                    }
                }
            } catch (e: Exception) {
                requestedCache = null
                throw e
            }
            return
        }
        val zone =
            json(
                "id" to "offline",
                "name" to "本机缓存",
                "endpoint_id" to graph.preferences.endpointId,
                "track" to entry.track.raw,
                "queue" to org.json.JSONArray(listOf(entry.track.raw)),
                "status" to "playing",
            )
        zoneState.value = zones.value.filter { it.text("id") != "offline" } + zone
        selectedZone.value = "offline"
        start(
            json(
                "track" to entry.track.raw,
                "zone_id" to "offline",
                "playback_id" to "offline-${java.util.UUID.randomUUID()}",
                "position" to position,
            ),
            cachedEntry = entry,
        )
    }

    fun rename(name: String) {
        socket?.rename(name)
        lastSignal = ""
    }

    suspend fun localControl(action: String, body: JSONObject = JSONObject()) {
        if (offline != null) {
            offlineControl(action, body)
            return
        }
        val zone =
            command.text("zone_id").ifBlank {
                zones.value
                    .firstOrNull { it.text("endpoint_id") == graph.preferences.endpointId }
                    ?.text("id")
                    .orEmpty()
            }
        require(zone.isNotBlank()) { "本机音区尚未准备好" }
        api?.command(zone, action, body) ?: error("Core未连接")
    }

    suspend fun control(action: String, body: JSONObject = JSONObject()) {
        if (selectedZone.value == "offline") {
            offlineControl(action, body)
            return
        }
        val target = selectedZone.value.takeIf { it.isNotBlank() } ?: error("正在准备播放设备，请稍候")
        if (action in setOf("play", "seek")) awaitLocalPipe()
        api?.command(target, action, body) ?: error("Core未连接")
    }

    private fun offlineControl(action: String, body: JSONObject) {
        when (action) {
            "play" -> {
                if (
                    player == null ||
                        activeMedia.isBlank() ||
                        player?.playbackState == Player.STATE_ENDED
                )
                    start(
                        JSONObject(command.toString())
                            .put(
                                "position",
                                if (player?.playbackState == Player.STATE_ENDED) 0.0 else origin,
                            ),
                        cachedEntry = offline,
                    )
                else player?.play()
            }
            "pause" -> {
                player?.pause()
                releaseWhenIdle()
            }
            "stop" -> {
                graph.history.sample(position(), player?.isPlaying == true, force = true)
                graph.history.finish("stop")
                graph.recovery.clear()
                stopPlayer()
            }
            "seek" -> {
                graph.history.sample(position(), player?.isPlaying == true, force = true)
                val position = body.optDouble("position").coerceAtLeast(0.0)
                if (player == null) {
                    origin = position
                    mutable.value = mutable.value.copy(position = position)
                } else player?.seekTo((position * 1000).toLong())
                graph.history.sample(
                    position,
                    player?.isPlaying == true,
                    reset = true,
                    force = true,
                )
            }
            else -> error("离线模式请从本机缓存选择歌曲")
        }
        tick()
        zoneState.value =
            zoneState.value.map {
                if (it.text("id") == "offline")
                    JSONObject(it.toString())
                        .put("status", if (player?.playWhenReady == true) "playing" else "paused")
                else it
            }
    }

    private suspend fun awaitLocalPipe() {
        val local =
            zones.value.firstOrNull { it.text("id") == selectedZone.value }?.text("endpoint_id") ==
                graph.preferences.endpointId
        if (local) {
            val link = socket ?: error("本机播放端尚未连接")
            withTimeout(10000) { link.online.filter { it }.first() }
        }
    }

    suspend fun play(tracks: List<Track>, index: Int = 0) {
        currentCache = null
        localRequests++
        try {
            awaitLocalPipe()
            api?.queue(selectedZone.value, tracks, index) ?: error("Core未连接")
        } finally {
            localRequests--
        }
    }

    suspend fun playAll(path: String) {
        val a = api ?: error("Core未连接")
        val result = mutableListOf<Track>()
        var offset = 0
        while (true) {
            val separator = if ('?' in path) '&' else '?'
            val page = a.request("$path${separator}limit=100&offset=$offset")
            val rows = page.rows().map(::Track)
            // The queue request needs IDs only; do not retain biographies/artwork metadata for a
            // whole library.
            result.addAll(rows.map { Track(json("id" to it.id)) })
            require(result.size <= 10000) { "播放列表超过一万首，请先筛选" }
            if (rows.isEmpty() || result.size >= page.optInt("total", result.size)) break
            offset += rows.size
        }
        require(result.isNotEmpty()) { "没有可播放的曲目" }
        play(result)
    }

    private fun start(
        value: JSONObject,
        resetRetry: Boolean = true,
        cachedEntry: MediaCache.Entry? = null,
    ) {
        val a = api ?: return
        val track = Track(value.child("track"))
        if (track.id.isBlank()) return
        val epoch = ++generation
        preparing?.cancel()
        idleRelease?.cancel()
        if (resetRetry) {
            retry = 0
            pcmFallback = false
        }
        if (offline != null) {
            graph.history.sample(position(), player?.isPlaying == true, force = true)
            graph.history.finish("replaced")
        }
        // Close old reads before planning the next source; a cancelled prepare never emits EOF.
        stopPlayer()
        val offeredCache =
            cachedEntry
                ?: requestedCache?.takeIf { it.track.id == track.id }
                ?: currentCache?.takeIf { it.track.id == track.id }
        requestedCache = null
        offline = offeredCache?.takeIf { value.text("zone_id") == "offline" }
        if (offline != null) graph.history.begin(track)
        graph.recovery.dismiss()
        command = value
        activeMedia = value.text("playback_id")
        readyToReport = false
        origin = value.optDouble("position", 0.0).coerceAtLeast(0.0)
        mutable.value = Playing(track, origin, buffering = true)
        preparing = scope.launch {
            try {
                val host = PlaybackService.current.get()
                if (host != null) host.beginPlaybackForeground()
                else
                    graph.application.startForegroundService(
                        Intent(graph.application, PlaybackService::class.java)
                    )
                val coreTrack =
                    if (offline != null) offline!!.track
                    else
                        runCatching { Track(a.request("library/tracks/${pathId(track.id)}")) }
                            .getOrDefault(track)
                val policy = graph.preferences.policy.value
                val wifi =
                    when (policy.networkMode) {
                        "wifi" -> true
                        "mobile" -> false
                        else -> graph.connectivity.unmeteredWifi()
                    }
                val caps = graph.devices.caps()
                val maxRate =
                    minOf(caps.maxRate, if (wifi) policy.wifiMaxRate else policy.mobileMaxRate)
                val maxBits =
                    minOf(caps.maxBits, if (wifi) policy.wifiMaxBits else policy.mobileMaxBits)
                fun compatible(e: MediaCache.Entry): Boolean {
                    val sameIdentity =
                        e.namespace == a.session.cacheNamespace &&
                            e.track.id == coreTrack.id &&
                            e.track.raw.text("recording_id") ==
                                coreTrack.raw.text("recording_id") &&
                            e.track.data.text("source_revision") ==
                                coreTrack.data.text("source_revision")
                    return sameIdentity &&
                        e.plan.rate <= maxRate &&
                        e.plan.bits <= maxBits &&
                        (!graph.preferences.rawPcmCompatibility ||
                            e.plan.bits <= 16 ||
                            e.plan.codec == WireCodec.PCM) &&
                        (policy.outputMode != OutputMode.USB_BITPERFECT ||
                            e.plan.codec == WireCodec.PCM)
                }
                if (offline != null)
                    require(offeredCache != null && compatible(offeredCache)) {
                        "缓存格式超出当前输出或用户上限，请联网转换后再播放"
                    }
                var cache = offeredCache?.takeIf { compatible(it) }
                var plan = cache?.plan ?: PlaybackPolicy.choose(coreTrack, wifi, caps, policy)
                if (cache == null && plan.codec == WireCodec.OPUS) {
                    cache =
                        withContext(Dispatchers.IO) {
                            MediaCache.preferredComplete(
                                graph.mediaCache.entries(a.session.cacheNamespace),
                                a.session.cacheNamespace,
                                coreTrack,
                                plan,
                                maxRate,
                                maxBits,
                                graph.preferences.rawPcmCompatibility,
                            )
                        }
                    cache?.let { plan = it.plan }
                }
                val selectedCache = cache
                val refreshed = selectedCache?.track ?: coreTrack
                if (
                    selectedCache == null &&
                        graph.preferences.rawPcmCompatibility &&
                        plan.bits > 16 &&
                        plan.codec != WireCodec.OPUS
                )
                    plan = plan.copy(codec = WireCodec.PCM)
                if (
                    selectedCache == null &&
                        plan.codec == WireCodec.OPUS &&
                        !a.request("status").child("capabilities").optBoolean("mobile_opus_enabled")
                )
                    throw IOException("Core未允许Opus，请在Core设置启用，或选择不转码")
                android.util.Log.i(
                    "RMusicAudio",
                    "prepare codec=${plan.codec} rate=${plan.rate} bits=${plan.bits} network=${graph.preferences.policy.value.networkMode} opus=${graph.preferences.policy.value.mobileOpus}",
                )
                val existing =
                    if (selectedCache != null) "https://cached.invalid/${pathId(track.id)}"
                    else
                        a.url(
                            value.text("url").takeIf { it.startsWith("api/v1/media/") }
                                ?: a.mediaUrl(refreshed)
                        )
                if (epoch != generation) return@launch
                currentCache = selectedCache
                absoluteTimeline = plan.codec == WireCodec.ORIGINAL || selectedCache != null
                graph.devices.prepare(plan)
                val begin =
                    if (plan.codec == WireCodec.ORIGINAL || selectedCache != null) 0.0 else origin
                val key =
                    selectedCache?.key
                        ?: plan.cacheKey(a.session.cacheNamespace, refreshed.id, begin)
                val cached = graph.mediaCache.exact(key, a.session.cacheNamespace) != null
                cacheKey = key
                graph.mediaCache.protect(key)
                livePlan = plan
                val upstream =
                    OkHttpDataSource.Factory(
                        a.http
                            .newBuilder()
                            .callTimeout(0, TimeUnit.SECONDS)
                            .readTimeout(30, TimeUnit.SECONDS)
                            .build()
                    )
                val factory = DataSource.Factory {
                    val cache =
                        CacheDataSource.Factory()
                            .setCache(graph.mediaCache.store)
                            .setCacheWriteDataSinkFactory {
                                PlaybackCacheSink(graph.mediaCache.store)
                            }
                            .setFlags(
                                CacheDataSource.FLAG_BLOCK_ON_CACHE or
                                    CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR
                            )
                            .setUpstreamDataSourceFactory(
                                if (selectedCache != null) null
                                else
                                    DataSource.Factory {
                                        ConfirmedSource(upstream.createDataSource(), plan)
                                    }
                            )
                            .createDataSource()
                    CompletedSource(cache) { length ->
                        if (begin == 0.0)
                            graph.mediaCache.completed(
                                key,
                                refreshed,
                                plan,
                                a.session.cacheNamespace,
                                length,
                            )
                    }
                }
                val p = ensurePlayer()
                activeMedia = value.text("playback_id")
                readyToReport = false
                ended = ""
                lastSignal = ""
                lastReport = 0
                val item =
                    MediaItem.Builder()
                        .setMediaId(activeMedia)
                        .setUri(plan.uri(existing, begin))
                        .setCustomCacheKey(key)
                        .setMediaMetadata(
                            MediaMetadata.Builder()
                                .setTitle(refreshed.title)
                                .setArtist(refreshed.artist)
                                .setAlbumTitle(refreshed.album)
                                .build()
                        )
                        .build()
                p.setMediaSource(
                    DefaultMediaSourceFactory(factory).createMediaSource(item),
                    if (plan.codec == WireCodec.ORIGINAL || selectedCache != null)
                        (origin * 1000).toLong()
                    else 0L,
                )
                p.prepare()
                p.playWhenReady = !value.optBoolean("paused")
                readyToReport = true
                mutable.value =
                    mutable.value.copy(
                        track = refreshed,
                        cached = cached,
                        transport =
                            when (plan.codec) {
                                WireCodec.OPUS -> "Opus · ${plan.bitrate} kbps · 有损"
                                WireCodec.PCM -> "Core PCM · ${plan.rate} Hz · ${plan.bits} bit"
                                WireCodec.FLAC -> "Core FLAC · ${plan.rate} Hz · ${plan.bits} bit"
                                WireCodec.ORIGINAL -> "原格式直传"
                            },
                        error = "",
                    )
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                if (epoch == generation)
                    fail(e.message ?: "音源暂时无法加载", e is IOException && e !is CoreFailure)
            }
        }
    }

    fun ensurePlayer(): ExoPlayer {
        val settings = graph.preferences.policy.value
        val signature = "${settings.outputMode}:${settings.bufferSeconds}:${settings.deviceId}"
        player?.let {
            if (playerSignature == signature) return it
            it.release()
            player = null
        }
        playerSignature = signature
        val renderers =
            object : DefaultRenderersFactory(graph.application) {
                    override fun buildAudioSink(
                        context: android.content.Context,
                        enableFloatOutput: Boolean,
                        enableAudioOutputPlaybackParams: Boolean,
                    ): AudioSink {
                        val sink =
                            DefaultAudioSink.Builder(context)
                                .setEnableFloatOutput(
                                    graph.preferences.policy.value.outputMode !=
                                        OutputMode.TRACK_INTEGER
                                )
                                .setAudioTrackProvider(graph.devices.provider)
                                .build()
                        val routed =
                            if (
                                graph.preferences.policy.value.outputMode ==
                                    OutputMode.USB_BITPERFECT
                            )
                                ExactPcmSink(sink, graph.devices)
                            else sink
                        return object : ForwardingAudioSink(routed) {
                            override fun configure(
                                format: Format,
                                specifiedBufferSize: Int,
                                outputChannels: IntArray?,
                            ) {
                                val target = livePlan
                                if (
                                    target != null &&
                                        target.bits > OutputDevices.bits(format.pcmEncoding)
                                ) {
                                    pcmFallback = target.codec != WireCodec.PCM && offline == null
                                    throw AudioSink.ConfigurationException(
                                        "decoder_precision",
                                        format,
                                    )
                                }
                                scope.launch {
                                    mutable.value =
                                        mutable.value.copy(
                                            decoder =
                                                "PCM · ${format.sampleRate} Hz · ${OutputDevices.bits(format.pcmEncoding)} bit${if(format.pcmEncoding==C.ENCODING_PCM_FLOAT)"（Float32容器）" else ""}"
                                        )
                                }
                                super.configure(format, specifiedBufferSize, outputChannels)
                            }
                        }
                    }
                }
                .setEnableAudioFloatOutput(true)
        val budget = graph.preferences.policy.value.bufferSeconds * 1000
        val p =
            ExoPlayer.Builder(graph.application, renderers)
                .setLoadControl(
                    DefaultLoadControl.Builder()
                        .setBufferDurationsMs(3000, budget.coerceIn(5000, 60000), 1500, 2500)
                        .setTargetBufferBytes(16 * 1024 * 1024)
                        .build()
                )
                .build()
        p.setWakeMode(C.WAKE_MODE_NETWORK)
        p.setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                .build(),
            true,
        )
        p.setHandleAudioBecomingNoisy(true)
        p.addListener(
            object : Player.Listener {
                override fun onEvents(player: Player, events: Player.Events) {
                    tick()
                }

                override fun onPlaybackStateChanged(playbackState: Int) {
                    if (
                        playbackState == Player.STATE_ENDED &&
                            p.playbackState == Player.STATE_ENDED &&
                            readyToReport &&
                            activeMedia.isNotEmpty() &&
                            ended != activeMedia &&
                            p.currentMediaItem?.mediaId == activeMedia
                    ) {
                        val expected = mutable.value.track?.duration ?: 0.0
                        if (expected > 0 && position() + 1.5 < expected) {
                            if (retry++ < 2 && offline == null) {
                                val retryFrom =
                                    JSONObject(command.toString()).put("position", position())
                                scope.launch { start(retryFrom, false) }
                            } else fail("音源在预期曲尾前中断，已保留播放位置", true)
                            return
                        }
                        ended = activeMedia
                        if (offline != null) {
                            graph.history.sample(position(), false, force = true)
                            graph.history.finish("ended")
                        }
                        graph.recovery.clear()
                        report("ended")
                        mutable.value = mutable.value.copy(playing = false, buffering = false)
                    }
                }

                override fun onPlayerError(error: PlaybackException) {
                    if (p.currentMediaItem?.mediaId != activeMedia) return
                    if (pcmFallback && offline == null && livePlan?.codec != WireCodec.PCM) {
                        graph.preferences.rawPcmCompatibility = true
                        val old = JSONObject(command.toString()).put("position", position())
                        scope.launch { start(old, false) }
                        return
                    }
                    if (retry++ < 2 && error.cause is IOException) {
                        val epoch = generation
                        val old = JSONObject(command.toString()).put("position", position())
                        scope.launch {
                            delay(750)
                            if (epoch == generation)
                                start(old.put("paused", !p.playWhenReady), false, offline)
                        }
                    } else fail(error.localizedMessage ?: "音频播放失败", error.errorCode in 2000..2999)
                }
            }
        )
        player = p
        onPlayerChanged?.invoke(p)
        return p
    }

    private fun releaseWhenIdle() {
        checkpoint()
        idleRelease?.cancel()
        val epoch = generation
        idleRelease = scope.launch {
            delay(30000)
            if (epoch == generation && player?.playWhenReady != true) {
                origin = position()
                activeMedia = ""
                stopPlayer()
                mutable.value =
                    mutable.value.copy(position = origin, playing = false, buffering = false)
            }
        }
    }

    private fun position(): Double =
        if (activeMedia.isBlank() || !readyToReport) origin
        else if (absoluteTimeline) (player?.currentPosition ?: 0) / 1000.0
        else origin + (player?.currentPosition ?: 0) / 1000.0

    private fun tick() {
        val p = player ?: return
        if (activeMedia.isNotBlank() && !readyToReport) return
        mutable.value =
            mutable.value.copy(
                position = position(),
                playing = p.isPlaying,
                buffering = p.playbackState == Player.STATE_BUFFERING,
                output = graph.devices.snapshot(),
            )
        if (activeMedia.isNotBlank()) {
            if (offline != null) graph.history.sample(position(), p.isPlaying)
            if (p.playbackState != Player.STATE_ENDED)
                mutable.value.track?.let {
                    graph.recovery.record(
                        it,
                        position(),
                        command.text("zone_id"),
                        cacheKey.orEmpty(),
                        offline != null,
                    )
                }
        }
        if (activeMedia.isNotBlank() && p.playbackState != Player.STATE_ENDED)
            report(if (p.playWhenReady) "playing" else "paused")
    }

    private fun report(status: String) {
        if (activeMedia.isBlank() || offline != null) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (status != "ended" && now - lastReport < 900) return
        lastReport = now
        val output = graph.devices.snapshot()
        val t = mutable.value.track
        val processing = mutableListOf<String>()
        livePlan?.let { p ->
            if (p.codec != WireCodec.ORIGINAL)
                processing.add(
                    if (p.codec == WireCodec.OPUS) "Core Opus有损编码 ${p.bitrate} kbps"
                    else "Core按输出上限转换 ${p.rate} Hz / ${p.bits} bit"
                )
        }
        if (output.isNull("mixer")) processing.add("Android共享输出；系统混音器实际格式未公开")
        val signal =
            json(
                "output" to output.text("device"),
                "sample_rate" to output.optInt("rate"),
                "channels" to output.optInt("channels"),
                "bit_depth" to output.optInt("bits"),
                "sample_type" to
                    if (output.optInt("encoding") == android.media.AudioFormat.ENCODING_PCM_FLOAT)
                        "f32"
                    else "integer",
                "exclusive" to !output.isNull("mixer"),
                "mixer_sample_rate" to output.optJSONObject("mixer")?.optInt("rate"),
                "bit_perfect_eligible" to
                    (!output.isNull("mixer") &&
                        t?.lossless == true &&
                        !t.dsd &&
                        t.rate == livePlan?.rate &&
                        t.bits == livePlan?.bits &&
                        t.channels == livePlan?.channels),
                "processing" to org.json.JSONArray(processing),
                "source" to
                    json("format" to t?.format, "sample_rate" to t?.rate, "bit_depth" to t?.bits),
                "transport" to
                    (mutable.value.transport + if (mutable.value.cached) " · 本机缓存" else ""),
                "decoder" to mutable.value.decoder,
                "android_output" to output,
            )
        val report =
            json(
                "type" to "state",
                "zone_id" to command.text("zone_id"),
                "endpoint_id" to graph.preferences.endpointId,
                "core_playback_id" to activeMedia,
                "track_id" to t?.id,
                "status" to status,
                "position" to position(),
            )
        val chain = signal.toString()
        val signature = "$status:${(position()*1000).toLong()}:$chain"
        if (status != "playing" && signature == lastStateSignature) return
        lastStateSignature = signature
        if (chain != lastSignal) {
            report.put("signal_path", signal)
            lastSignal = chain
        }
        socket?.send(report)
    }

    private fun fail(
        message: String,
        sourceFailure: Boolean = false,
        deviceFailure: Boolean = false,
    ) {
        mutable.value = mutable.value.copy(error = message, playing = false, buffering = false)
        socket?.send(
            json(
                "type" to "error",
                "zone_id" to command.text("zone_id"),
                "endpoint_id" to graph.preferences.endpointId,
                "core_playback_id" to activeMedia,
                "track_id" to mutable.value.track?.id,
                "message" to message,
                "source_failed" to sourceFailure,
                "device_failed" to deviceFailure,
                "recoverable" to false,
            )
        )
        val at = position()
        if (offline != null) {
            graph.history.sample(at, false, force = true)
            graph.history.finish("interrupted")
        }
        stopPlayer()
        currentCache = null
        origin = at
        mutable.value = mutable.value.copy(position = at)
    }

    private fun stopPlayer() {
        activeMedia = ""
        player?.stop()
        player?.clearMediaItems()
        cacheKey?.let { graph.mediaCache.unprotect(it) }
        cacheKey = null
        graph.devices.clearMixer()
    }

    fun checkpoint() {
        if (activeMedia.isNotBlank())
            mutable.value.track?.let {
                graph.recovery.record(
                    it,
                    position(),
                    command.text("zone_id"),
                    cacheKey.orEmpty(),
                    offline != null,
                    true,
                )
                if (offline != null)
                    graph.history.sample(position(), player?.isPlaying == true, force = true)
            }
    }

    fun networkChanged() {
        // Keep a healthy buffered source and its precision. Reconnect control separately; the
        // next play chooses Wi-Fi/mobile quality. Never replace mid-song just to change codec.
        socket?.reconnectIfNeeded()
        monitor?.reconnectIfNeeded()
        if (foreground) graph.history.requestSync()
    }

    fun releasePlayback() {
        ++generation
        preparing?.cancel()
        idleRelease?.cancel()
        stopPlayer()
        player?.release()
        player = null
        graph.devices.release()
    }

    fun disconnect() {
        checkpoint()
        graph.history.finish("interrupted")
        ++generation
        preparing?.cancel()
        reporting?.cancel()
        socket?.close()
        socket = null
        monitor?.close()
        monitor = null
        releasePlayback()
        api = null
        requestedCache = null
        currentCache = null
        offline = null
        zoneState.value = emptyList()
        selectedZone.value = ""
        mutable.value = Playing()
    }
}

@UnstableApi
private class CompletedSource(
    private val delegate: DataSource,
    private val finished: (Long) -> Unit,
) : DataSource by delegate {
    private var bytes = 0L
    private var eof = false
    private var zero = false

    override fun open(dataSpec: DataSpec): Long {
        bytes = 0
        eof = false
        zero = dataSpec.position == 0L
        return delegate.open(dataSpec)
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
        delegate.read(buffer, offset, length).also {
            if (it == C.RESULT_END_OF_INPUT) eof = true else if (it > 0) bytes += it
        }

    override fun close() {
        delegate.close()
        if (eof && zero) finished(bytes)
    }
}
