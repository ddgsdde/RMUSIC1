package com.rmusic.android.development

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug only: retain and invoke an actual old notification action for the identity guard audit. */
class QueueControlReceiver : BroadcastReceiver() {
    companion object { private var oldAction: PendingIntent? = null }
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            "com.rmusic.android.development.QA_CAPTURE" -> {
                oldAction = context.getSystemService(NotificationManager::class.java).activeNotifications
                    .first { it.notification.channelId == "rmusic-playback" }.notification.actions.first().actionIntent
                resultData = "captured"
            }
            "com.rmusic.android.development.QA_INVOKE_OLD" -> {
                checkNotNull(oldAction).send(); oldAction = null; resultData = "sent"
            }
        }
    }
}
