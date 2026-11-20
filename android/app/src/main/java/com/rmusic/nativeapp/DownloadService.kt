@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build

class DownloadService : Service() {
    companion object {
        var current = java.lang.ref.WeakReference<DownloadService>(null)
            private set
    }

    private var lastStartId = 0
    private val graph
        get() = (application as RMusicApplication).graph

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(
                NotificationChannel("downloads", "歌曲下载", NotificationManager.IMPORTANCE_LOW)
            )
        showForeground()
        current = java.lang.ref.WeakReference(this)
    }

    private fun showForeground() {
        val n =
            Notification.Builder(this, "downloads")
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle("R-Music 正在下载")
                .setContentText("串行下载，避免占满音源接口")
                .setOnlyAlertOnce(true)
                .build()
        if (Build.VERSION.SDK_INT >= 29)
            startForeground(72, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        else startForeground(72, n)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStartId = startId
        android.util.Log.i("RMusicDownloads", "service start=$startId")
        // A reused instance may have been demoted while a newer start was in flight.
        showForeground()
        graph.downloads.run { finishIfIdle() }
        return START_NOT_STICKY
    }

    fun finishIfIdle() {
        val idle = graph.downloads.idle
        val stopped = lastStartId > 0 && idle && stopSelfResult(lastStartId)
        android.util.Log.i(
            "RMusicDownloads",
            "finish start=$lastStartId idle=$idle stopped=$stopped",
        )
        if (stopped) stopForeground(STOP_FOREGROUND_REMOVE)
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        graph.downloads.cancel()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf(startId)
    }

    override fun onBind(intent: Intent?) = null

    override fun onDestroy() {
        android.util.Log.i("RMusicDownloads", "service destroy=$lastStartId")
        // Preserve a newly queued start that may arrive while the old service is retiring.
        if (current.get() === this) {
            current.clear()
            graph.downloads.serviceStopped()
        }
        super.onDestroy()
    }
}
