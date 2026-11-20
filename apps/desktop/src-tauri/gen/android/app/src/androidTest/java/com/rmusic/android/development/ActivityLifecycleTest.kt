package com.rmusic.android.development

import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ActivityLifecycleTest {
    private fun webview(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) for (index in 0 until view.childCount) webview(view.getChildAt(index))?.let { return it }
        return null
    }
    private fun verifyNativeIpc(activity: android.app.Activity) {
        val runner = InstrumentationRegistry.getInstrumentation()
        val deadline = System.nanoTime() + 15000000000L
        var view: WebView? = null
        while (view == null && System.nanoTime() < deadline) {
            runner.runOnMainSync { view = webview(activity.window.decorView) }
            if (view == null) Thread.sleep(20)
        }
        assertNotNull("Reopened Activity has no WebView", view)
        var started = false
        while (System.nanoTime() < deadline) {
            val result = java.util.concurrent.atomic.AtomicReference<String>()
            val ready = java.util.concurrent.CountDownLatch(1)
            runner.runOnMainSync {
                val code = if (!started) "(() => { if (!window.__TAURI_INTERNALS__) return false; window.__rmusicLifecycle = null; window.__TAURI_INTERNALS__.invoke('native_devices').then(devices => window.__rmusicLifecycle = {ok:Array.isArray(devices)}, error => window.__rmusicLifecycle = {error:String(error)}); return true; })()" else "window.__rmusicLifecycle"
                view!!.evaluateJavascript(code) { result.set(it); ready.countDown() }
            }
            assertTrue("WebView script callback timed out", ready.await(2, java.util.concurrent.TimeUnit.SECONDS))
            if (!started) started = result.get() == "true"
            else if (result.get() != "null") {
                assertTrue("Native IPC failed after reopen: ${result.get()}", org.json.JSONObject(result.get()).optBoolean("ok"))
                return
            }
            Thread.sleep(20)
        }
        fail("Native IPC was not ready after Activity reopen")
    }
    @Test fun closingAndReopeningTheMainActivityKeepsTheNativeRuntimeAlive() {
        val runner = InstrumentationRegistry.getInstrumentation()
        val frames = 96000 * 30; val bytes = frames * 4
        val header = java.nio.ByteBuffer.allocate(44).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        header.put("RIFF".toByteArray()).putInt(36 + bytes).put("WAVEfmt ".toByteArray()).putInt(16)
            .putShort(1).putShort(2).putInt(96000).putInt(96000 * 4).putShort(4).putShort(16).put("data".toByteArray()).putInt(bytes)
        val file = java.io.File(runner.targetContext.cacheDir, "lifecycle-zero.wav")
        file.outputStream().use { out -> out.write(header.array()); val zeros = ByteArray(96000 * 4); repeat(30) { out.write(zeros) } }
        var playing = false
        try { repeat(2) { index ->
            val activity = runner.startActivitySync(Intent(runner.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            val deadline = System.nanoTime() + 15000000000L
            while (!DecoderDiagnostics.ready() && System.nanoTime() < deadline) Thread.sleep(20)
            assertTrue("Native runtime did not initialize", DecoderDiagnostics.ready())
            verifyNativeIpc(activity)
            if (index == 0) {
                LifecycleDiagnostics.start(file.absolutePath); playing = true
                val started = System.nanoTime() + 10000000000L
                while (LifecycleDiagnostics.frames() < 9600 && System.nanoTime() < started) Thread.sleep(20)
                assertTrue("AudioTrack did not consume programme frames", LifecycleDiagnostics.frames() >= 9600)
            }
            val before = LifecycleDiagnostics.frames()
            runner.runOnMainSync { activity.finish() }
            runner.waitForIdleSync()
            val closed = System.nanoTime() + 15000000000L
            while (!activity.isDestroyed && System.nanoTime() < closed) Thread.sleep(20)
            assertTrue("Main Activity did not finish", activity.isDestroyed)
            Thread.sleep(2000)
            assertTrue("Audio stopped after Activity destruction", LifecycleDiagnostics.frames() > before + 96000)
        } } finally { if (playing) LifecycleDiagnostics.stop() }
        val manager = runner.targetContext.getSystemService(android.content.Context.ACTIVITY_SERVICE) as android.app.ActivityManager
        @Suppress("DEPRECATION")
        fun serviceRunning() = manager.getRunningServices(100).any { it.service.className == AudioPlaybackService::class.java.name }
        val stopped = System.nanoTime() + 5000000000L
        while (serviceRunning() && System.nanoTime() < stopped) Thread.sleep(20)
        assertFalse("Foreground playback service leaked", serviceRunning())
    }
}
