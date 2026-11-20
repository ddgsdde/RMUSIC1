@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.playback

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.media3.common.*
import androidx.media3.session.*
import com.rmusic.core.json
import com.rmusic.nativeapp.MainActivity
import com.rmusic.nativeapp.RMusicApplication
import java.lang.ref.WeakReference
import kotlinx.coroutines.launch

/** The same foreground owner keeps Core control available and later acquires media playback. */
class PlaybackService : MediaSessionService() {
    companion object {
        var current = WeakReference<PlaybackService>(null)
            private set
    }

    private var session: MediaSession? = null
    private var opening: PendingIntent? = null
    private val graph
        get() = (application as RMusicApplication).graph

    override fun onCreate() {
        super.onCreate()
        current = WeakReference(this)
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(
                NotificationChannel("music", "音乐与远程控制", NotificationManager.IMPORTANCE_LOW)
            )
        opening =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        showForeground(graph.runtime.state.value.buffering || graph.runtime.state.value.playing)
        setMediaNotificationProvider(
            DefaultMediaNotificationProvider.Builder(this)
                .setNotificationId(71)
                .setChannelId("music")
                .build()
        )
        val engine = graph.runtime
        fun forwarded(player: Player): Player =
            object : ForwardingPlayer(player) {
                private fun send(
                    action: String,
                    body: org.json.JSONObject = org.json.JSONObject(),
                ) {
                    graph.scope.launch {
                        runCatching { engine.localControl(action, body) }
                            .onFailure { graph.report(it.message ?: "操作失败") }
                    }
                }

                override fun getCurrentPosition() = (engine.state.value.position * 1000).toLong()

                override fun getContentPosition() = currentPosition

                override fun getDuration() =
                    engine.state.value.track
                        ?.duration
                        ?.takeIf { it > 0 }
                        ?.let { (it * 1000).toLong() } ?: super.getDuration()

                override fun getContentDuration() = duration

                override fun getAvailableCommands() =
                    super.getAvailableCommands()
                        .buildUpon()
                        .add(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM)
                        .add(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                        .add(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                        .build()

                override fun play() {
                    send("play")
                }

                override fun pause() {
                    send("pause")
                }

                override fun stop() {
                    send("stop")
                }

                override fun seekTo(positionMs: Long) {
                    send("seek", json("position" to positionMs / 1000.0))
                }

                override fun seekToNextMediaItem() {
                    send("next")
                }

                override fun seekToPreviousMediaItem() {
                    send("previous")
                }
            }
        session =
            MediaSession.Builder(this, forwarded(engine.ensurePlayer()))
                .setSessionActivity(opening!!)
                .build()
        engine.onPlayerChanged = { session?.setPlayer(forwarded(it)) }
    }

    fun beginPlaybackForeground() {
        showForeground(true)
    }

    private fun showForeground(media: Boolean) {
        val state = graph.runtime.state.value
        val builder =
            Notification.Builder(this, "music")
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentIntent(opening)
                .setOnlyAlertOnce(true)
                .setContentTitle(state.track?.title ?: "R-Music · 已连接 Core")
                .setContentText(state.track?.artist ?: "允许其他设备控制此播放端")
                .setOngoing(media || graph.preferences.backgroundControl)
        session?.let { s ->
            builder.setStyle(
                Notification.MediaStyle()
                    .setMediaSession(s.platformToken)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            fun action(name: String, icon: Int, description: String) {
                val intent = Intent(this, PlaybackService::class.java).setAction("rmusic.$name")
                val pending =
                    PendingIntent.getService(
                        this,
                        name.hashCode(),
                        intent,
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                    )
                builder.addAction(Notification.Action.Builder(icon, description, pending).build())
            }
            action("previous", android.R.drawable.ic_media_previous, "上一首")
            action(
                if (state.playing) "pause" else "play",
                if (state.playing) android.R.drawable.ic_media_pause
                else android.R.drawable.ic_media_play,
                if (state.playing) "暂停" else "播放",
            )
            action("next", android.R.drawable.ic_media_next, "下一首")
        }
        if (Build.VERSION.SDK_INT >= 29) {
            var type =
                if (media) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                else ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
            if (graph.preferences.backgroundControl)
                type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
            startForeground(71, builder.build(), type)
        } else startForeground(71, builder.build())
    }

    override fun onUpdateNotification(session: MediaSession, startInForegroundRequired: Boolean) {
        if (graph.preferences.backgroundControl)
            showForeground(
                startInForegroundRequired ||
                    graph.runtime.state.value.playing ||
                    graph.runtime.state.value.buffering
            )
        else super.onUpdateNotification(session, startInForegroundRequired)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        intent
            ?.action
            ?.takeIf { it.startsWith("rmusic.") }
            ?.let { action ->
                graph.scope.launch {
                    runCatching { graph.runtime.localControl(action.removePrefix("rmusic.")) }
                        .onFailure { graph.report(it.message ?: "操作失败") }
                }
                return START_NOT_STICKY
            }
        return super.onStartCommand(intent, flags, startId)
    }

    fun updateRemotePolicy() {
        if (graph.preferences.backgroundControl) showForeground(graph.runtime.state.value.playing)
        else if (!graph.runtime.state.value.playing && !graph.runtime.state.value.buffering) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        if (!graph.preferences.backgroundControl) super.onTaskRemoved(rootIntent)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo) = session

    override fun onDestroy() {
        if (current.get() === this) current.clear()
        graph.runtime.onPlayerChanged = null
        session?.release()
        session = null
        val state = graph.runtime.state.value
        if (!state.playing && !state.buffering) graph.runtime.releasePlayback()
        super.onDestroy()
    }
}
