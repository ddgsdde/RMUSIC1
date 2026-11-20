package com.rmusic.android.development

import android.app.Activity
import android.content.Context
import android.media.*
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import app.tauri.plugin.Channel
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.max
import kotlin.math.min
import java.util.concurrent.TimeUnit

@InvokeArg
class AudioCallbacks { lateinit var updates: Channel }

/** Independent Rust workers own sessions; late calls must carry the exact session identity. */
@TauriPlugin
@androidx.annotation.Keep
class AudioPlugin(private val host: Activity) : Plugin(host) {
    companion object {
        @JvmStatic external fun registerNativeBridge(plugin: AudioPlugin)
        @JvmStatic external fun playbackChanged()
        @JvmStatic external fun remoteMediaCommand(zone: String, media: String, action: String)
        private val sessions = java.util.concurrent.ConcurrentHashMap<String, AudioPlugin.Session>()
        private val outputLock = Any()
        private val routeOwners = mutableMapOf<Int, String>()
        private val focusOwners = mutableSetOf<String>()
        private var sharedFocus: AudioFocusRequest? = null
    }
    init { registerNativeBridge(this) }
    fun nativeDecoder(command: String, payload: String, output: ByteArray): String = NativeDecoder.command(command,payload,output)
    private val manager = host.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val offlineStorage by lazy { OfflineStorage(host.applicationContext) }
    @Command fun chooseOfflineDirectory(invoke: Invoke) {startActivityForResult(invoke,offlineStorage.chooseIntent(),"offlineDirectoryResult")}
    @app.tauri.annotation.ActivityCallback
    private fun offlineDirectoryResult(invoke: Invoke,result: androidx.activity.result.ActivityResult) {
        val uri=result.data?.data
        if(result.resultCode!=Activity.RESULT_OK||uri==null){invoke.resolveObject(mapOf("cancelled" to true));return}
        Thread {
            try {val value=offlineStorage.accept(uri,result.data!!.flags);invoke.resolve(app.tauri.plugin.JSObject(value.toString()))}
            catch(error:Exception){invoke.reject(error.message?:"无法使用该离线目录")}
        }.start()
    }
    private val coreDiscovery by lazy { CoreDiscovery(host.applicationContext) }
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
    private fun acquireFocus(id: String) = synchronized(outputLock) {
        if (focusOwners.isEmpty()) {
            val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN).setAudioAttributes(attributes)
                .setOnAudioFocusChangeListener({ change -> if (change < 0) sessions.values.toList().forEach {
                    it.fail(DeviceError("音频焦点已丢失，请手动重新播放"))
                } }, Handler(Looper.getMainLooper())).build()
            if (manager.requestAudioFocus(request) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) throw DeviceError("系统拒绝音频焦点")
            sharedFocus = request
        }
        focusOwners.add(id)
    }
    private fun releaseOutput(id: String) = synchronized(outputLock) {
        routeOwners.entries.removeAll { it.value == id }
        focusOwners.remove(id)
        if (focusOwners.isEmpty()) { sharedFocus?.let { manager.abandonAudioFocusRequest(it) }; sharedFocus = null }
    }
    private fun claimRoute(id: String, route: Int) = synchronized(outputLock) {
        if (routeOwners[route]?.let { it != id } == true) throw DeviceError("此音频输出已被另一个音区占用，请选择其他设备")
        routeOwners[route] = id
    }
    private class DeviceError(message: String) : Exception(message)
    private fun usb(device: AudioDeviceInfo) = device.type == AudioDeviceInfo.TYPE_USB_DEVICE ||
        device.type == AudioDeviceInfo.TYPE_USB_HEADSET || device.type == AudioDeviceInfo.TYPE_USB_ACCESSORY
    private fun outputs() = manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
    private fun deviceKey(device: AudioDeviceInfo): String {
        val value = "${device.type}:${device.address}:${device.productName}"
        val digest = java.security.MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        val ambiguous = outputs().count { "${it.type}:${it.address}:${it.productName}" == value } > 1
        return "android:" + digest.take(12).joinToString("") { "%02x".format(it.toInt() and 255) } + if (ambiguous) "-${device.id}" else ""
    }
    private fun bits(encoding: Int) = when (encoding) {
        AudioFormat.ENCODING_PCM_8BIT -> 8
        AudioFormat.ENCODING_PCM_16BIT -> 16
        AudioFormat.ENCODING_PCM_24BIT_PACKED -> 24
        AudioFormat.ENCODING_PCM_32BIT, AudioFormat.ENCODING_PCM_FLOAT -> 32
        else -> 0
    }
    private fun integer(format: MediaFormat, key: String, fallback: Int = 0) =
        if (format.containsKey(key)) format.getInteger(key) else fallback
    private fun audio(extractor: MediaExtractor): MediaFormat {
        for (index in 0 until extractor.trackCount) {
            val format = extractor.getTrackFormat(index)
            if (format.getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true) {
                extractor.selectTrack(index)
                return format
            }
        }
        throw Exception("系统解码器未找到可播放音轨")
    }
    @Command fun saveDownload(invoke: Invoke) {
        val args=invoke.getArgs()
        Thread {
            var created: android.net.Uri?=null
            try {
                val file=java.io.File(args.getString("path")).canonicalFile
                require(file.path.startsWith(host.cacheDir.canonicalPath+java.io.File.separator)&&file.name.startsWith("rmusic-export-")) { "Invalid export staging path" }
                val name=args.getString("name").replace(Regex("[\\\\/:*?\"<>|]"),"_")
                if(Build.VERSION.SDK_INT>=29){
                    val values=android.content.ContentValues().apply { put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME,name);put(android.provider.MediaStore.MediaColumns.MIME_TYPE,args.getString("mime"));put(android.provider.MediaStore.MediaColumns.RELATIVE_PATH,android.os.Environment.DIRECTORY_DOWNLOADS+"/R-Music");put(android.provider.MediaStore.MediaColumns.IS_PENDING,1) }
                    val uri=host.contentResolver.insert(android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI,values)?:error("Cannot create download")
                    created=uri
                    host.contentResolver.openOutputStream(uri,"w")!!.use { output -> file.inputStream().use { it.copyTo(output,1024*1024) } }
                    values.clear();values.put(android.provider.MediaStore.MediaColumns.IS_PENDING,0);host.contentResolver.update(uri,values,null,null)
                    invoke.resolveObject(mapOf("uri" to uri.toString()))
                }else{
                    val directory=java.io.File(host.getExternalFilesDir(android.os.Environment.DIRECTORY_MUSIC),"R-Music").apply { mkdirs() }
                    val target=java.io.File(directory,"${System.currentTimeMillis()}-$name")
                    file.copyTo(target,false);invoke.resolveObject(mapOf("uri" to target.path))
                }
            }catch(error:Throwable){created?.let { try { host.contentResolver.delete(it,null,null) }catch(_:Throwable){} };invoke.reject(error.message?:"Export failed")}
        }.start()
    }
    @Command fun devices(invoke: Invoke) {
        invoke.resolveObject(outputs().map { device ->
            val strict = if (Build.VERSION.SDK_INT >= 34 && usb(device))
                manager.getSupportedMixerAttributes(device).any { it.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT }
                else false
            mapOf("id" to deviceKey(device), "name" to "${device.productName} · ${if (usb(device)) "USB" else "系统设备"}${if (strict) " · 支持原样请求" else ""}")
        })
    }
    private fun extractorInput(extractor: MediaExtractor, input: String) {
        val fd = input.removePrefix("/proc/self/fd/").takeIf { input.startsWith("/proc/self/fd/") }?.toIntOrNull()
        if (fd != null) android.os.ParcelFileDescriptor.fromFd(fd).use { extractor.setDataSource(it.fileDescriptor) }
        else extractor.setDataSource(input)
    }
    @Command fun probe(invoke: Invoke) {
        val input = invoke.getArgs().getString("input")
        Thread({
            val extractor = MediaExtractor()
            try {
                extractorInput(extractor, input)
                val format = audio(extractor)
                val mime = format.getString(MediaFormat.KEY_MIME)!!
                val encoding = integer(format, MediaFormat.KEY_PCM_ENCODING)
                val precision = integer(format, "bits-per-sample", bits(encoding))
                val rate = integer(format, MediaFormat.KEY_SAMPLE_RATE)
                val channels = integer(format, MediaFormat.KEY_CHANNEL_COUNT)
                require(rate in 1..768000 && channels in 1..8) { "音轨采样率或声道不受支持" }
                val codec = when (mime) {
                    "audio/raw" -> "wav"; "audio/flac" -> "flac"; "audio/alac" -> "alac"
                    "audio/mpeg" -> "mp3"; "audio/mp4a-latm" -> "aac"; "audio/opus" -> "opus"
                    else -> mime.removePrefix("audio/")
                }
                invoke.resolveObject(mapOf("duration" to if (format.containsKey(MediaFormat.KEY_DURATION)) format.getLong(MediaFormat.KEY_DURATION) / 1000000.0 else null,
                    "format" to codec, "sample_rate" to rate, "channels" to channels, "bit_depth" to precision,
                    "bitrate" to integer(format, MediaFormat.KEY_BIT_RATE), "quality_rank" to 0,
                    "source_integer_lossless" to (codec in listOf("wav", "flac", "alac") && precision > 0 && encoding != AudioFormat.ENCODING_PCM_FLOAT)))
            } catch (error: Exception) { invoke.reject("Android 音轨探测失败：${error.message}") }
            finally { extractor.release() }
        }, "rmusic-probe").start()
    }
    private fun session(invoke: Invoke): Session {
        val id = invoke.getArgs().getString("id")
        return sessions[id] ?: throw Exception("播放实例已经失效")
    }
    @Command fun play(invoke: Invoke) {
        try {
            val next = Session(invoke.getArgs(), invoke.parseArgs(AudioCallbacks::class.java).updates)
            synchronized(outputLock) {
                require(sessions.size < 32 && !sessions.containsKey(next.id)) { "输出会话已存在或已达到并发上限" }
                sessions[next.id] = next
            }
            Thread({ next.run() }, "rmusic-audio").start()
            invoke.resolve()
        } catch (error: Exception) { invoke.reject(error.message) }
    }
    @Command fun controls(invoke: Invoke) {
        try { val session = session(invoke); session.controls(invoke.getArgs()); session.report(); invoke.resolveObject(session.snapshot()) }
        catch (error: Exception) { invoke.reject(error.message) }
    }
    @Command fun status(invoke: Invoke) {
        try { invoke.resolveObject(session(invoke).snapshot()) }
        catch (error: Exception) { invoke.reject(error.message) }
    }
    @Command fun stop(invoke: Invoke) {
        try { val session = session(invoke); session.cancel(); session.report(); invoke.resolveObject(session.snapshot()) }
        catch (error: Exception) { invoke.reject(error.message) }
    }
    // Session methods serialize state; PCM feeding must not wait for the UI Looper.
    fun nativeControls(command: String, payload: String): String {
        if(command.startsWith("storage_"))return offlineStorage.command(command,JSONObject(payload))
        val args = JSONObject(payload)
        if (command == "discovery") return coreDiscovery.snapshot()
        if (command == "session") return SessionVault.command(host.applicationContext, args)
        if (command == "download_notice") return DownloadService.update(host.applicationContext,args)
        if (command == "quality") {
            val prefs = host.applicationContext.getSharedPreferences("rmusic-network-quality", Context.MODE_PRIVATE)
            val values = args.optJSONObject("values")
            val allowed = setOf("original", "lossless", "hires", "opus96", "opus160", "opus320")
            if (values != null) {
                val wifi = values.getString("wifi"); val mobile = values.getString("mobile")
                require(wifi in allowed && mobile in allowed) { "音质选择无效" }
                check(prefs.edit().putString("wifi", wifi).putString("mobile", mobile).commit()) { "音质设置保存失败" }
            }
            val cm = host.getSystemService(Context.CONNECTIVITY_SERVICE) as android.net.ConnectivityManager
            val caps = cm.getNetworkCapabilities(cm.activeNetwork)
            val wifi = caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true
            val vpn = caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_VPN) == true
            val subnets = org.json.JSONArray()
            if (!vpn) cm.getLinkProperties(cm.activeNetwork)?.linkAddresses?.forEach { address ->
                subnets.put("${address.address.hostAddress}/${address.prefixLength}")
            }
            val selected = args.optString("device_id").takeIf { it.isNotBlank() }?.let { key -> outputs().firstOrNull { deviceKey(it) == key } }
            val maxRate = selected?.sampleRates?.maxOrNull() ?: manager.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)?.toIntOrNull() ?: 48000
            val maxBits = selected?.encodings?.map { bits(it) }?.maxOrNull()?.takeIf { it > 0 } ?: 24
            return JSONObject().put("max_sample_rate", maxRate).put("max_bit_depth", maxBits)
                .put("wifi", prefs.getString("wifi", "original"))
                .put("mobile", prefs.getString("mobile", "original"))
                .put("network", if(wifi) "wifi" else if(caps != null) "mobile" else "unknown")
                .put("unmetered_wifi", wifi && !cm.isActiveNetworkMetered)
                .put("vpn", vpn).put("metered", cm.isActiveNetworkMetered).put("local_subnets", subnets)
                .put("reason", if(wifi) "当前为 Wi-Fi" else "当前为移动流量或未确认网络")
                .toString()
        }
        if (command == "artwork") {
            AudioPlaybackService.artwork(args.getString("track_id"), args.getString("image"))
            return "{}"
        }
        if (command == "remote_media") {
            val state = args.optJSONObject("state")
            val values = state?.keys()?.asSequence()?.associateWith { key -> state.get(key) }
            AudioPlaybackService.remote(host.applicationContext, values)
            return "{}"
        }
        if (command == "transport") {
            val state = args.optJSONObject("state")
            val values = state?.keys()?.asSequence()?.associateWith { key -> state.get(key).let { if (it == JSONObject.NULL) null else it } }
            return JSONObject().put("action", AudioPlaybackService.transport(args.getString("id"), values) ?: JSONObject.NULL).toString()
        }
        val session = sessions[args.getString("id")] ?: throw Exception("播放实例已经失效")
        when (command) { "controls" -> session.controls(args); "stop" -> session.cancel(); "status" -> {}; "start" -> session.startPcm(); "drain" -> session.finishPcm(); "metadata" -> session.metadata(args); "release" -> check(session.isComplete()) { "输出线程尚未退出" }; else -> throw Exception("未知原生音频控制") }
        if (command != "status") session.report()
        val state = JSONObject(session.snapshot()).toString()
        if (command == "release") sessions.remove(session.id, session)
        return state
    }
    fun writePcm(id: String, bytes: ByteArray): Int =
        (sessions[id] ?: throw Exception("播放实例已经失效")).offerPcm(bytes)
    override fun onDestroy(activity: AppCompatActivity) { sessions.values.forEach { if (!AudioPlaybackService.owns(it.id)) it.cancel() } }

    private inner class Session(private val args: JSONObject, private val updates: Channel) : AudioPlaybackService.Playback {
        override val id = args.getString("id")
        private var title = args.optString("title", "R-Music")
        private var artist = args.optString("artist", "")
        private var durationMs = (args.optDouble("duration", 0.0) * 1000).toLong()
        private var mediaId = args.optString("media_id", id)
        private var trackId = args.optString("track_id", "")
        private var mediaPosition = args.getDouble("position")
        private var mediaPlayed = 0L
        private var actionTarget = ""
        private var actionPosition = 0.0
        private val mode = args.optString("mode", "system")
        private val lock = java.lang.Object()
        @Volatile private var cancelled = false
        @Volatile private var paused = false
        @Volatile private var volume = 1.0
        @Volatile private var muted = false
        private var track: AudioTrack? = null
        private var written = 0L
        private var played = 0L
        private var head = 0L
        private var wraps = 0L
        private var prime = 0L
        private var rate = 0
        private var channels = 0
        private var encoding = 0
        private var ended = false
        private var released = false
        private var error: String? = null
        private var kind = "source"
        private var output = "Android AudioTrack"
        private var preferred: AudioDeviceInfo? = null
        private var strict = false
        private var mixerSet = false
        private var mediaRevision = 0L
        private var mediaAction = ""
        private val externalPcm = args.optBoolean("external_pcm")
        private val pcm = java.util.concurrent.ArrayBlockingQueue<ByteArray>(2)
        @Volatile private var inputFinished = false
        @Volatile private var complete = false
        private val scheduled = args.optBoolean("scheduled")
        private var started = !scheduled
        private var prepared = false
        private val timestamp = AudioTimestamp()
        private var clock: Map<String, Any>? = null
        private var lastClockRead = 0L
        fun isComplete() = complete
        private var underrunBase: Int? = null
        private var underruns = 0
        private var continuityLost = false
        private var terminalUnderruns = 0
        private var preparationUnderruns = 0
        init { controls(args.getJSONObject("controls")) }
        fun controls(value: JSONObject) = synchronized(lock) {
            val gain = value.getDouble("volume")
            require(gain.isFinite() && gain in 0.0..1.0) { "音量必须为 0..1" }
            if (mode == "usb_bitperfect" && (gain != 1.0 || value.getBoolean("muted"))) {
                fail(DeviceError("USB 原样模式不支持软件音量和静音，请使用 DAC 调节音量"))
            } else {
                paused = value.getBoolean("paused"); volume = gain; muted = value.getBoolean("muted")
                track?.let { applyControls(it) }
                lock.notifyAll()
                AudioPlaybackService.changed(id)
            }
        }
        private fun applyControls(audio: AudioTrack) {
            audio.setVolume(if (externalPcm) 1f else if (muted) 0f else volume.toFloat())
            if (paused || !started) { if (audio.playState == AudioTrack.PLAYSTATE_PLAYING) audio.pause() }
            else if (audio.playState != AudioTrack.PLAYSTATE_PLAYING) audio.play()
        }
        private fun count() {
            if (released) return
            if (prepared && !started) {
                if (track?.playState == AudioTrack.PLAYSTATE_PLAYING) invalidateClock("计划提交前 AudioTrack 意外启动，已停止")
                return
            }
            val raw = track?.playbackHeadPosition?.toLong()?.and(0xffffffffL) ?: return
            if (raw < head) {
                if (head >= 0xf0000000L && raw <= 0x0fffffffL) wraps += 1L shl 32
                else { invalidateClock("AudioTrack 播放头意外重置，请重新播放"); return }
            }
            head = raw
            played = min(written, max(played, max(0L, wraps + raw - prime)))
            val now = System.nanoTime()
            if (now - lastClockRead >= 250000000L) {
                lastClockRead = now
                val before = System.nanoTime()
                val valid = track?.getTimestamp(timestamp) == true
                val after = System.nanoTime()
                clock = if (valid && started && !paused && timestamp.framePosition >= prime && timestamp.framePosition - prime <= written &&
                    timestamp.nanoTime in (after - 500000000L)..after) mapOf(
                    "counter" to timestamp.framePosition, "programme_frames" to timestamp.framePosition - prime,
                    "time_ns" to timestamp.nanoTime, "before_ns" to before, "after_ns" to after, "running" to true) else null
            }
            underrunBase?.let { base -> track?.let {
                val observed = max(underruns, it.underrunCount - base)
                if (observed > underruns) {
                    if (inputFinished && played == written && pcm.isEmpty()) terminalUnderruns += observed - underruns
                    else continuityLost = true
                }
                if (BuildConfig.DEBUG && observed > underruns) android.util.Log.i("RMusicUnderrun", JSONObject()
                    .put("count",observed).put("written",written).put("played",played).put("queued",pcm.size)
                    .put("input_finished",inputFinished).put("paused",paused).put("ended",ended).put("complete",complete).toString())
                underruns = observed
            } }
        }
        private fun invalidateClock(message: String) {
            error = message; kind = "device"; cancelled = true
            track?.pause(); released = true; track?.flush()
        }
        override fun snapshot(): Map<String, Any?> = synchronized(lock) {
            count()
            mapOf("id" to id, "media_id" to mediaId, "track_id" to trackId, "title" to title, "artist" to artist, "duration_ms" to durationMs,
                "cancelled" to cancelled, "written" to written, "played" to played, "sample_rate" to rate, "channels" to channels,
                "bits" to if (mode == "compatibility") 16 else bits(encoding), "sample_type" to if (mode != "compatibility" && encoding == AudioFormat.ENCODING_PCM_FLOAT) "float" else "int",
                "output" to output, "exclusive" to strict, "eligible" to (strict && args.optBoolean("lossless")),
                "processing" to listOf(when (mode) {
                    "usb_bitperfect" -> "Android 已接受 USB 原样混音属性；硬件逐位结果待验"
                    "compatibility" -> "显式 PCM16 兼容输出；系统可能混音或重采样"
                    else -> "Android 系统输出；系统可能混音或重采样，蓝牙编解码器由系统协商"
                }) + if (args.optBoolean("lossless") && args.optInt("bits") > bits(encoding)) listOf("系统解码精度 ${args.optInt("bits")} → ${bits(encoding)} bit") else emptyList(),
                "ended" to ended, "error" to error, "kind" to kind, "paused" to paused,
                "media_revision" to mediaRevision, "media_action" to mediaAction, "media_target" to actionTarget, "media_position" to actionPosition,
                "complete" to complete, "prepared" to prepared, "started" to started, "underruns" to underruns,
                "continuity_lost" to continuityLost, "terminal_underruns" to terminalUnderruns, "preparation_underruns" to preparationUnderruns,
                "clock" to if (paused || !started || released) null else clock,
                "prefill_frames" to track?.bufferCapacityInFrames,
                "position_ms" to ((mediaPosition + if (rate > 0) max(0L, played - mediaPlayed).toDouble() / rate else 0.0) * 1000).toLong())
        }
        override fun report() { updates.sendObject(snapshot()) }
        override val queueControls get() = externalPcm
        override fun seek(positionMs: Long) { command("seek", null, positionMs) }
        override fun mediaCommand(action: String, expectedMediaId: String?) { command(action, expectedMediaId, null) }
        private fun command(action: String, expectedMediaId: String?, positionMs: Long?) = synchronized(lock) {
            val supported = action in listOf("play", "pause", "stop") || externalPcm && action in listOf("next", "previous", "seek")
            if (!cancelled && !ended && (expectedMediaId == null || expectedMediaId == mediaId) && supported) {
                count(); actionTarget = mediaId
                actionPosition = if (action == "seek") (positionMs ?: 0L).coerceAtLeast(0).toDouble() / 1000.0 else mediaPosition + if (rate > 0) max(0L,played - mediaPlayed).toDouble() / rate else 0.0
                if (action == "stop" && !externalPcm) cancel() else if (action in listOf("play", "pause", "stop")) { paused = action != "play"; track?.let { applyControls(it) } }
                mediaRevision++; mediaAction = action
                lock.notifyAll(); playbackChanged()
                report()
                AudioPlaybackService.changed(id)
            }
        }
        fun cancel() = synchronized(lock) {
            cancelled = true; count()
            lock.notifyAll(); playbackChanged()
            track?.let { if (!released) { it.pause(); it.flush() } }
            released = true // Preserve the confirmed cursor before flush resets the device head.
            clearPreferences()
        }
        private fun clearPreferences() {
            if (Build.VERSION.SDK_INT >= 34 && mixerSet) preferred?.let { manager.clearPreferredMixerAttributes(attributes, it) }
            mixerSet = false
        }
        fun fail(cause: Exception) = synchronized(lock) {
            if (!ended && !cancelled) { error = cause.message ?: "Android 音频失败"; kind = if (cause is DeviceError) "device" else "source"; cancel() }
        }
        fun offerPcm(bytes: ByteArray): Int {
            require(externalPcm && bytes.isNotEmpty() && bytes.size <= 65536 && bytes.size % (args.getInt("channels") * 4) == 0) { "无效的有界 PCM 数据" }
            check(!inputFinished && !cancelled) { "PCM 输出已关闭" }
            return if (pcm.offer(bytes)) bytes.size else 0
        }
        fun finishPcm() { check(externalPcm) { "不是外部 PCM 会话" }; inputFinished = true }
        fun startPcm() = synchronized(lock) {
            check(externalPcm && scheduled && prepared && written > 0 && !cancelled) { "计划输出尚未预缓冲或已经结束" }
            if (!started) { track?.let {
                if (Build.VERSION.SDK_INT < 31 && written < it.bufferCapacityInFrames) throw DeviceError("此 Android 版本无法以短于输出缓冲的内容计划起播，请减小缓冲")
                val baseline=it.underrunCount;preparationUnderruns += max(0,baseline-(underrunBase ?: baseline));underrunBase=baseline
                if (Build.VERSION.SDK_INT >= 31) it.setStartThresholdInFrames(min(written,it.bufferCapacityInFrames.toLong()).coerceAtLeast(1).toInt())
                started = true;applyControls(it)
            } }
        }
        fun metadata(value: JSONObject) = synchronized(lock) {
            check(externalPcm) { "不是外部 PCM 会话" }
            trackId = value.optString("track_id", ""); mediaId = value.getString("playback_id"); title = value.getString("title"); artist = value.getString("artist")
            durationMs = (value.getDouble("duration") * 1000).toLong()
            mediaPosition = value.getDouble("position"); mediaPlayed = value.getLong("played")
            AudioPlaybackService.changed(id)
        }
        private fun open(format: MediaFormat) = synchronized(lock) {
            if (cancelled) return@synchronized
            rate = integer(format, MediaFormat.KEY_SAMPLE_RATE)
            channels = integer(format, MediaFormat.KEY_CHANNEL_COUNT)
            encoding = integer(format, MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            require(rate == args.getInt("sample_rate") && channels == args.getInt("channels")) { "解码器改变了采样率或声道，已拒绝自动转换" }
            require(rate in 1..768000 && channels in 1..8 && bits(encoding) > 0) { "解码 PCM 格式不受支持" }
            val outputEncoding = if (mode == "compatibility") AudioFormat.ENCODING_PCM_16BIT else encoding
            val mask = integer(format, MediaFormat.KEY_CHANNEL_MASK).takeIf { it != 0 }
                ?: when (channels) { 1 -> AudioFormat.CHANNEL_OUT_MONO; 2 -> AudioFormat.CHANNEL_OUT_STEREO; else -> 0 }
            require(mask != 0) { "多声道音轨缺少可靠的声道布局" }
            val audioFormat = AudioFormat.Builder().setSampleRate(rate).setChannelMask(mask).setEncoding(outputEncoding).build()
            val deviceId = args.optString("device").takeUnless { it.isEmpty() || it == "null" }
            preferred = deviceId?.let { value -> outputs().firstOrNull { deviceKey(it) == value || "android:${it.id}" == value } ?: throw DeviceError("所选音频设备已断开") }
            claimRoute(id, preferred?.id ?: 0)
            if (mode == "usb_bitperfect") {
                val device = preferred ?: throw DeviceError("USB 原样模式需要选择 USB 设备")
                if (Build.VERSION.SDK_INT < 34 || !usb(device)) throw DeviceError("USB 原样模式需要 Android 14 及兼容 USB 设备")
                require(args.optBoolean("lossless") && args.getInt("bits") > 0 && bits(encoding) >= args.getInt("bits") && encoding != AudioFormat.ENCODING_PCM_FLOAT) { "解码器未证明保留整数源精度，已拒绝原样模式" }
                val mixer = manager.getSupportedMixerAttributes(device).firstOrNull {
                    it.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT && it.format == audioFormat
                } ?: throw DeviceError("USB 设备不支持此格式的系统原样模式；可手动选择系统或兼容方式")
                if (!manager.setPreferredMixerAttributes(attributes, device, mixer)) throw DeviceError("系统拒绝 USB 原样混音属性")
                strict = true
                mixerSet = true
            }
            val minimum = AudioTrack.getMinBufferSize(rate, mask, outputEncoding)
            if (minimum <= 0) throw DeviceError("系统不支持此 PCM 输出格式；可手动选择兼容方式")
            val millis = args.getInt("buffer_ms")
            require(millis in 20..2000) { "缓冲时长必须为 20–2000 ms" }
            val frameBytes = channels * bits(outputEncoding) / 8
            val buffer = max(minimum.toLong(), rate.toLong() * frameBytes * max(millis, if (mode == "compatibility") 250 else 20) / 1000).toInt()
            val audio = AudioTrack.Builder().setAudioAttributes(attributes).setAudioFormat(audioFormat)
                .setTransferMode(AudioTrack.MODE_STREAM).setBufferSizeInBytes(buffer).build()
            track = audio
            if (audio.state != AudioTrack.STATE_INITIALIZED) throw DeviceError("AudioTrack 初始化失败")
            preferred?.let { if (!audio.setPreferredDevice(it)) throw DeviceError("系统拒绝所选音频路由") }
            acquireFocus(id)
            if (scheduled && Build.VERSION.SDK_INT >= 31) audio.setStartThresholdInFrames(1)
            // Resolve the route with silence before any programme data can reach a different device.
            val warmupFrames = if (scheduled && Build.VERSION.SDK_INT < 31) audio.bufferCapacityInFrames else max(1, rate / 100)
            val zeros = ByteBuffer.allocateDirect(warmupFrames * frameBytes)
            val bytes = audio.write(zeros, zeros.remaining(), AudioTrack.WRITE_NON_BLOCKING)
            if (bytes < 0 || bytes % frameBytes != 0) throw DeviceError("音频设备拒绝预热缓冲")
            prime = bytes.toLong() / frameBytes
            audio.play()
        }
        private fun route() {
            val audio = track ?: return
            val device = audio.routedDevice
            preferred?.let { if (device?.id != it.id) throw DeviceError("所选音频路由已改变，已停止输出") }
            if (device != null) { claimRoute(id, device.id); output = "${device.productName} · AudioTrack" }
        }
        private fun compatible(input: ByteBuffer): ByteBuffer {
            if (mode != "compatibility" || encoding == AudioFormat.ENCODING_PCM_16BIT) return input
            input.order(ByteOrder.LITTLE_ENDIAN)
            val result = ByteBuffer.allocateDirect(input.remaining() / (bits(encoding) / 8) * 2).order(ByteOrder.LITTLE_ENDIAN)
            while (input.hasRemaining()) {
                val sample = when (encoding) {
                    AudioFormat.ENCODING_PCM_8BIT -> ((input.get().toInt() and 255) - 128) shl 8
                    AudioFormat.ENCODING_PCM_24BIT_PACKED -> { input.get(); (input.get().toInt() and 255) or (input.get().toInt() shl 8) }
                    AudioFormat.ENCODING_PCM_32BIT -> input.int shr 16
                    AudioFormat.ENCODING_PCM_FLOAT -> (input.float.coerceIn(-1f, 1f) * 32768f).toInt().coerceIn(-32768, 32767)
                    else -> throw Exception("无法转换此 PCM 编码")
                }
                result.putShort(sample.toShort())
            }
            result.flip(); return result
        }
        private fun awaitResume() = synchronized(lock) { while (paused && !cancelled) lock.wait() }
        private fun write(input: ByteBuffer, timestamp: Long, target: Long) {
            val frameBytes = channels * bits(encoding) / 8
            require(input.remaining() % frameBytes == 0) { "解码器返回了不完整的 PCM 帧" }
            val skip = min(input.remaining() / frameBytes.toLong(), max(0L, (target - timestamp) * rate + 500000) / 1000000)
            input.position(input.position() + skip.toInt() * frameBytes)
            val data = compatible(input)
            val outputFrame = channels * (if (mode == "compatibility") 2 else bits(encoding) / 8)
            while (data.hasRemaining() && !cancelled) {
                awaitResume()
                if (cancelled) break
                val bytes = synchronized(lock) {
                    if (cancelled || paused) 0 else {
                        if (started) route()
                        if (externalPcm && underrunBase == null) underrunBase = track!!.underrunCount
                        val count = track!!.write(data, data.remaining(), AudioTrack.WRITE_NON_BLOCKING)
                        if (count < 0 || count % outputFrame != 0) throw DeviceError("AudioTrack 写入失败：$count")
                        written += count / outputFrame
                        count
                    }
                }
                if (bytes == 0) Thread.sleep(5)
            }
        }
        fun run() {
            val extractor = MediaExtractor()
            var decoder: MediaCodec? = null
            try {
                if (cancelled) return
                AudioPlaybackService.start(host.applicationContext, this)
                if (externalPcm) {
                    val format = MediaFormat.createAudioFormat("audio/raw", args.getInt("sample_rate"), args.getInt("channels"))
                    format.setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_32BIT)
                    open(format); readyRoute()
                    while (!cancelled) {
                        awaitResume()
                        if (cancelled) break
                        val bytes = pcm.poll(10, TimeUnit.MILLISECONDS)
                        if (bytes != null) write(ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN), 0, 0)
                        else if (inputFinished) break
                    }
                } else {
                extractorInput(extractor, args.getString("input"))
                val source = audio(extractor)
                val target = (args.getDouble("position") * 1000000).toLong()
                require(target >= 0) { "播放位置无效" }
                if (target > 0) extractor.seekTo(target, MediaExtractor.SEEK_TO_PREVIOUS_SYNC)
                val mime = source.getString(MediaFormat.KEY_MIME)!!
                if (mime == "audio/raw") {
                    open(source)
                    readyRoute()
                    val buffer = ByteBuffer.allocateDirect(1024 * 1024)
                    while (!cancelled) {
                        buffer.clear(); val count = extractor.readSampleData(buffer, 0)
                        if (count < 0) break
                        buffer.position(0); buffer.limit(count); write(buffer, extractor.sampleTime, target)
                        extractor.advance()
                    }
                } else {
                    val codec = MediaCodec.createDecoderByType(mime); decoder = codec
                    if (args.getInt("bits") > 16 && Build.VERSION.SDK_INT >= 31)
                        source.setInteger(MediaFormat.KEY_PCM_ENCODING, if (args.getInt("bits") > 24) AudioFormat.ENCODING_PCM_32BIT else AudioFormat.ENCODING_PCM_24BIT_PACKED)
                    codec.configure(source, null, null, 0); codec.start()
                    var inputEnded = false; var outputEnded = false
                    val info = MediaCodec.BufferInfo()
                    while (!cancelled && !outputEnded) {
                        awaitResume()
                        if (cancelled) break
                        if (!inputEnded) {
                            val index = codec.dequeueInputBuffer(10000)
                            if (index >= 0) {
                                val buffer = codec.getInputBuffer(index)!!; val count = extractor.readSampleData(buffer, 0)
                                if (count < 0) { codec.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM); inputEnded = true }
                                else { codec.queueInputBuffer(index, 0, count, extractor.sampleTime, 0); extractor.advance() }
                            }
                        }
                        val index = codec.dequeueOutputBuffer(info, 10000)
                        if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                            require(track == null) { "播放中 PCM 格式变化，已停止" }; open(codec.outputFormat); readyRoute()
                        } else if (index >= 0) {
                            try {
                                if (info.size > 0) {
                                    require(track != null) { "解码器尚未提供输出格式" }
                                    val buffer = codec.getOutputBuffer(index)!!; buffer.position(info.offset); buffer.limit(info.offset + info.size)
                                    write(buffer, info.presentationTimeUs, target)
                                }
                                outputEnded = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                            } finally { codec.releaseOutputBuffer(index, false) }
                        }
                    }
                }
                }
                while (!cancelled) {
                    awaitResume()
                    if (cancelled) break
                    val drained = synchronized(lock) { count(); route(); played == written }
                    if (drained) break
                    Thread.sleep(5)
                }
                synchronized(lock) { if (!cancelled) { count(); ended = true } }
            } catch (cause: Exception) { fail(cause) }
            finally {
                synchronized(lock) { count(); released = true; track?.release(); track = null }
                synchronized(lock) { clearPreferences() }
                releaseOutput(id)
                decoder?.let { try { it.stop() } finally { it.release() } }
                extractor.release()
                synchronized(lock) { complete = true; pcm.clear() }
                report()
                AudioPlaybackService.finish(id)
            }
        }
        private fun readyRoute() {
            val deadline = System.nanoTime() + max(1000000000L, prime * 1000000000L / max(1, rate) + 500000000L)
            while (!cancelled && track?.routedDevice == null && System.nanoTime() < deadline) Thread.sleep(5)
            if (scheduled) {
                // Only routing silence is consumed before commit. Drain it before pausing so
                // every endpoint starts its programme at the prepared buffer's first frame.
                while (!cancelled && System.nanoTime() < deadline) {
                    val primed = synchronized(lock) { count(); wraps + head >= prime }
                    if (primed) break
                    Thread.sleep(2)
                }
            }
            synchronized(lock) { if (!cancelled) {
                route()
                if (scheduled && wraps + head < prime) throw DeviceError("计划输出的路由预热未完成")
                track?.let {
                    if (scheduled) {
                        it.pause(); it.flush()
                        prime = 0; head = 0; wraps = 0; played = 0; clock = null; lastClockRead = 0
                        if (Build.VERSION.SDK_INT >= 31) it.setStartThresholdInFrames(1)
                    }
                    applyControls(it)
                }
                prepared = true
            } }
        }
    }
}
