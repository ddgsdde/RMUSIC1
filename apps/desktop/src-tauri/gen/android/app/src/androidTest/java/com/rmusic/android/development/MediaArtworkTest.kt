package com.rmusic.android.development

import android.app.Notification
import android.content.Intent
import android.app.NotificationManager
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSession
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream

@RunWith(AndroidJUnit4::class)
class MediaArtworkTest {
    @Before fun launch() {
        val runner = InstrumentationRegistry.getInstrumentation()
        runner.startActivitySync(Intent(runner.targetContext,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    @Test fun notificationAndSystemSessionFollowArtworkWithoutReusingThePreviousSong() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        var track = "artwork-first"
        val playback = object : AudioPlaybackService.Playback {
            override val id = "artwork-qa"
            override val keepsAwake = false
            override fun snapshot() = mapOf("media_id" to track, "track_id" to track, "title" to track,
                "artist" to "QA", "duration_ms" to 10000L, "paused" to true)
            override fun report() {}
            override fun mediaCommand(action: String, expectedMediaId: String?) {}
        }
        fun metadata(): MediaMetadata? {
            val notification = context.getSystemService(NotificationManager::class.java).activeNotifications.firstOrNull { it.id == 41 }?.notification ?: return null
            val token = notification.extras.getParcelable(Notification.EXTRA_MEDIA_SESSION, MediaSession.Token::class.java) ?: return null
            return MediaController(context, token).metadata
        }
        fun await(check: () -> Boolean) {
            val deadline = System.nanoTime() + 3000000000L
            while (!check() && System.nanoTime() < deadline) Thread.sleep(20)
            assertTrue(check())
        }
        val bitmap = Bitmap.createBitmap(32,32,Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.GREEN) }
        val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG,100,it) }.toByteArray()
        val image = "data:image/png;base64," + android.util.Base64.encodeToString(bytes,android.util.Base64.NO_WRAP)
        try {
            AudioPlaybackService.start(context,playback)
            AudioPlaybackService.artwork(track,image)
            await { metadata()?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART) != null }
            assertEquals(android.graphics.Color.GREEN,metadata()!!.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)!!.getPixel(0,0))
            await { context.getSystemService(NotificationManager::class.java).activeNotifications.firstOrNull { it.id == 41 }?.notification?.getLargeIcon() != null }
            track = "artwork-second";AudioPlaybackService.changed(playback.id)
            await { metadata()?.getString(MediaMetadata.METADATA_KEY_TITLE) == track }
            assertNull(metadata()!!.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART))
            AudioPlaybackService.artwork("artwork-first",image)
            Thread.sleep(80)
            assertNull(metadata()!!.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART))
            AudioPlaybackService.artwork(track,image)
            await { metadata()?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART) != null }
        } finally { AudioPlaybackService.finish(playback.id) }
    }
}
