package com.rmusic.android.development

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.*
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Foreground lifetime and system controls for local playback and the selected Core output. */
class AudioPlaybackService : Service() {
    interface Playback {
        val id: String
        fun snapshot(): Map<String, Any?>
        fun mediaCommand(action: String, expectedMediaId: String? = null)
        fun report()
        val queueControls: Boolean get() = false
        fun seek(positionMs: Long) {}
        val keepsAwake: Boolean get() = true
    }
    companion object {
        private const val CHANNEL = "rmusic-playback"
        private const val NOTIFICATION = 41
        private val main = Handler(Looper.getMainLooper())
        private val playbacks = java.util.concurrent.ConcurrentHashMap<String, Playback>()
        private val startups = java.util.concurrent.ConcurrentHashMap<String, CountDownLatch>()
        @Volatile private var selected: String? = null
        private var remotePlayback: RemotePlayback? = null
        private var startingRemote: RemotePlayback? = null
        private val playback: Playback? get() = remotePlayback ?: selected?.let { playbacks[it] } ?: playbacks.values.firstOrNull()
        private var service: AudioPlaybackService? = null
        private val artworkCache = object : android.util.LruCache<String, Bitmap>(2 * 1024 * 1024) {
            override fun sizeOf(key: String, value: Bitmap) = value.byteCount
        }
        fun artwork(trackId: String, data: String) {
            val bytes = android.util.Base64.decode(data.substringAfter(','), android.util.Base64.DEFAULT)
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            options.inSampleSize = 1
            while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 256) options.inSampleSize *= 2
            options.inJustDecodeBounds = false
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: return
            synchronized(artworkCache) { artworkCache.put(trackId, bitmap) }
            main.post { service?.update() }
        }
        private fun artwork(state: Map<String, Any?>): Bitmap? = (state["track_id"] as? String)?.let { key ->
            synchronized(artworkCache) { artworkCache.get(key) }
        }
        fun start(context: Context, next: Playback) {
            val ready = CountDownLatch(1)
            startups[next.id] = ready
            playbacks[next.id] = next
            selected = next.id
            try {
                context.startForegroundService(Intent(context, AudioPlaybackService::class.java).putExtra("session", next.id))
                check(ready.await(5, TimeUnit.SECONDS)) { "后台音频服务未能及时启动" }
            } catch (cause: Exception) {
                playbacks.remove(next.id, next)
                if (Build.VERSION.SDK_INT >= 31 && cause is ForegroundServiceStartNotAllowedException)
                    throw IllegalStateException("Android 系统不允许从后台新建播放服务，请回到应用后手动播放", cause)
                throw cause
            } finally {
                startups.remove(next.id, ready)
            }
        }
        fun finish(id: String) { main.post {
            playbacks.remove(id)
            if (playback == null) main.postDelayed({ if (playback == null) service?.stopSelf() }, 250)
            else service?.update()
        } }
        fun remote(context: Context, state: Map<String, Any?>?) { main.post {
            remotePlayback = state?.let { RemotePlayback(it) }
            val currentService = service
            if (currentService != null) {
                if (playback == null && startingRemote == null) currentService.stopSelf()
                else currentService.update()
            } else if (startingRemote == null && remotePlayback != null) {
                startingRemote = remotePlayback
                try {
                    context.startForegroundService(Intent(context, AudioPlaybackService::class.java))
                } catch (_: IllegalStateException) {
                    startingRemote = null
                    remotePlayback = null
                }
            }
        } }
        fun changed(id: String) { main.post { if (playbacks.containsKey(id)) service?.update() } }
        fun owns(id: String) = playbacks.containsKey(id)
        fun transport(id: String, state: Map<String, Any?>?): String? {
            if (state == null) { finish(id); return null }
            val remote = playbacks.computeIfAbsent(id) { Transport(id, state) } as Transport
            remote.state = state
            main.post { service?.ownedIds?.add(id) }
            changed(id)
            return remote.action.getAndSet(null)
        }
    }
    /** A coordinated pause releases the PCM worker, but keeps the Core transport controllable. */
    private class Transport(override val id: String, @Volatile var state: Map<String, Any?>) : Playback {
        val action = java.util.concurrent.atomic.AtomicReference<String?>(null)
        override val keepsAwake = false
        override fun snapshot() = state
        override fun report() {}
        override fun mediaCommand(action: String, expectedMediaId: String?) {
            if (expectedMediaId == null || expectedMediaId == state["media_id"]) this.action.set(action)
        }
    }
    private class RemotePlayback(private val state: Map<String, Any?>) : Playback {
        private val zone = state.getValue("zone") as String
        val updatedAt = SystemClock.elapsedRealtime()
        override val id = "remote:$zone"
        override val keepsAwake = false
        override val queueControls = true
        override fun snapshot() = state
        override fun report() {}
        override fun mediaCommand(action: String, expectedMediaId: String?) {
            val media = state.getValue("media_id") as String
            if (state["available"] == true && (expectedMediaId == null || expectedMediaId == media)
                && action in setOf("play", "pause", "next", "previous")) {
                AudioPlugin.remoteMediaCommand(zone, media, action)
            }
        }
    }
    private lateinit var media: MediaSession
    private lateinit var wake: PowerManager.WakeLock
    private var metadataId: String? = null
    private var metadataArtwork: Bitmap? = null
    private val ownedIds = mutableSetOf<String>()
    private var notificationState: Triple<String, Boolean, Boolean>? = null
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) playbacks.values.toList().forEach { it.mediaCommand("pause") }
        }
    }
    private val tick = object : Runnable {
        override fun run() { update(); main.postDelayed(this, if (playback?.snapshot()?.get("paused") == true) 30000 else 5000) }
    }
    override fun onCreate() {
        super.onCreate(); service = this
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, "音乐播放", NotificationManager.IMPORTANCE_LOW))
        wake = (getSystemService(Context.POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "R-Music:playback")
        wake.setReferenceCounted(false)
        media = MediaSession(this, "R-Music")
        media.setCallback(object : MediaSession.Callback() {
            override fun onPlay() { playback?.mediaCommand("play") }
            override fun onPause() { playback?.mediaCommand("pause") }
            override fun onStop() { playback?.mediaCommand("stop") }
            override fun onSkipToNext() { playback?.takeIf { it.queueControls }?.mediaCommand("next") }
            override fun onSkipToPrevious() { playback?.takeIf { it.queueControls }?.mediaCommand("previous") }
            override fun onSeekTo(pos: Long) { playback?.takeIf { it.queueControls }?.seek(pos) }
        }, main)
        media.isActive = true
        val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(noisy, filter, Context.RECEIVER_NOT_EXPORTED)
        else registerReceiver(noisy, filter)
        main.post(tick)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val current = playback ?: startingRemote
        startingRemote = null
        if (current == null) { stopSelf(); return START_NOT_STICKY }
        ownedIds.addAll(playbacks.keys)
        // Old notification PendingIntents must never control a replacement session.
        val expectedMedia = intent?.getStringExtra("media")
        if (expectedMedia != null && current === playback && current.id == intent.getStringExtra("session")) {
            intent.action?.let { current.mediaCommand(it, expectedMedia) }
        }
        val notification = notification(current, current.snapshot())
        if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        else startForeground(NOTIFICATION, notification)
        startups.values.forEach { it.countDown() }
        if (playback == null) stopSelf() else update()
        return START_NOT_STICKY
    }
    private fun pending(current: Playback, mediaId: String, action: String): PendingIntent = PendingIntent.getService(this, 0,
        Intent(this, AudioPlaybackService::class.java).setAction(action)
            .setData(Uri.parse("rmusic://playback/${current.id}/$mediaId/$action")).putExtra("session", current.id).putExtra("media",mediaId),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    private fun notification(current: Playback, state: Map<String, Any?>): Notification {
        val paused = state["paused"] == true
        val available = state["available"] != false
        val mediaId = state["media_id"] as String
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val builder = Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(state["title"] as String).setContentText(state["artist"] as String).setContentIntent(open)
            .setCategory(Notification.CATEGORY_TRANSPORT).setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOnlyAlertOnce(true).setOngoing(!paused).setShowWhen(false)
        (state["device"] as? String)?.let { builder.setSubText(it) }
        artwork(state)?.let { builder.setLargeIcon(it) }
        if (available) {
            if (current.queueControls) builder.addAction(Notification.Action.Builder(android.R.drawable.ic_media_previous, "上一首", pending(current, mediaId, "previous")).build())
            builder.addAction(Notification.Action.Builder(
                    if (paused) android.R.drawable.ic_media_play else android.R.drawable.ic_media_pause,
                    if (paused) "播放" else "暂停", pending(current, mediaId, if (paused) "play" else "pause")).build())
            if (current.queueControls) builder.addAction(Notification.Action.Builder(android.R.drawable.ic_media_next, "下一首", pending(current, mediaId, "next")).build())
            if (current !is RemotePlayback) builder.addAction(Notification.Action.Builder(android.R.drawable.ic_menu_close_clear_cancel, "停止", pending(current, mediaId, "stop")).build())
        }
        val compact = when {
            !available -> intArrayOf()
            current.queueControls -> intArrayOf(0, 1, 2)
            else -> intArrayOf(0, 1)
        }
        return builder.setStyle(Notification.MediaStyle().setMediaSession(media.sessionToken).setShowActionsInCompactView(*compact)).build()
    }
    private fun update() {
        val current = playback ?: return
        val state = current.snapshot()
        playbacks.values.toList().forEach { it.report() }
        val paused = state["paused"] == true
        val available = state["available"] != false
        val sourceId = "${current.id}:${state["media_id"]}"
        val artwork = artwork(state)
        if (metadataId != sourceId || metadataArtwork !== artwork) {
            val builder = MediaMetadata.Builder().putString(MediaMetadata.METADATA_KEY_TITLE, state["title"] as String)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, state["artist"] as String)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, (state["duration_ms"] as Number).toLong())
            (state["device"] as? String)?.let { device ->
                builder.putString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE, state["title"] as String)
                    .putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, listOf(state["artist"] as String, device).filter { it.isNotBlank() }.joinToString(" · "))
            }
            artwork?.let { builder.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it).putBitmap(MediaMetadata.METADATA_KEY_ART, it) }
            media.setMetadata(builder.build())
            metadataId = sourceId
            metadataArtwork = artwork
            notificationState = null
        }
        val position = (state["position_ms"] as? Number)?.toLong() ?: 0L
        val remote = current as? RemotePlayback
        val actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or
            (if (remote == null) PlaybackState.ACTION_STOP else 0L) or
            (if (current.queueControls) PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS else 0L) or
            (if (current.queueControls && remote == null) PlaybackState.ACTION_SEEK_TO else 0L)
        val playbackState = when {
            paused || !available -> PlaybackState.STATE_PAUSED
            state["buffering"] == true -> PlaybackState.STATE_BUFFERING
            else -> PlaybackState.STATE_PLAYING
        }
        media.setPlaybackState(PlaybackState.Builder().setActions(if (available) actions else 0L)
            .setState(playbackState, position, if (playbackState == PlaybackState.STATE_PLAYING) 1f else 0f, remote?.updatedAt ?: SystemClock.elapsedRealtime()).build())
        val running = playbacks.values.any { it.keepsAwake && it.snapshot()["paused"] != true }
        if (!running) { if (wake.isHeld) wake.release() } else if (!wake.isHeld) wake.acquire()
        val signature = Triple(sourceId, paused, available)
        if (notificationState != signature) {
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification(current, state))
            notificationState = signature
        }
    }
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() {
        main.removeCallbacks(tick)
        ownedIds.forEach { id -> playbacks.remove(id)?.mediaCommand("stop") }
        remotePlayback = null
        if (wake.isHeld) wake.release()
        unregisterReceiver(noisy); media.isActive = false; media.release()
        stopForeground(STOP_FOREGROUND_REMOVE); if (service === this) service = null
        super.onDestroy()
    }
}
