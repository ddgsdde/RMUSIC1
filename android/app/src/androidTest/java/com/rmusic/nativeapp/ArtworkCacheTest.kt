@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import android.graphics.Bitmap
import androidx.test.platform.app.InstrumentationRegistry
import coil3.request.*
import java.io.ByteArrayOutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Actual HTTP 200 -> ETag/304, not a mocked image-loader return value. */
class ArtworkCacheTest {
    @Test
    fun expiredArtworkRevalidatesWithoutResendingPixelsOrLeakingCoreAuthorization() = runBlocking {
        val application =
            InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
                as RMusicApplication
        val graph = application.graph
        val bitmap =
            Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888).apply {
                eraseColor(0xff47d6af.toInt())
            }
        val pixels =
            ByteArrayOutputStream()
                .also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                .toByteArray()
        bitmap.recycle()
        val server =
            ServerSocket(0, 2, InetAddress.getByName("127.0.0.1")).apply { soTimeout = 10000 }
        val requests = CopyOnWriteArrayList<String>()
        var payloads = 0
        val responder =
            async(Dispatchers.IO) {
                repeat(2) {
                    server.accept().use { client ->
                        client.soTimeout = 10000
                        val reader = client.getInputStream().bufferedReader()
                        val lines = mutableListOf<String>()
                        while (true) {
                            val line = reader.readLine() ?: break
                            if (line.isEmpty()) break
                            lines += line
                        }
                        val request = lines.joinToString("\n")
                        requests += request
                        val conditional = request.contains("If-None-Match:", true)
                        val headers =
                            if (conditional) "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n"
                            else
                                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: ${pixels.size}\r\n"
                        client.getOutputStream().use { out ->
                            out.write(
                                (headers +
                                        "ETag: \"image-v1\"\r\nCache-Control: private, max-age=0\r\nConnection: close\r\n\r\n")
                                    .toByteArray()
                            )
                            if (!conditional) {
                                out.write(pixels)
                                payloads++
                            }
                        }
                    }
                }
            }
        try {
            val url = "http://127.0.0.1:${server.localPort}/artwork-qa"
            repeat(2) {
                val result =
                    graph.images.execute(
                        ImageRequest.Builder(application)
                            .data(url)
                            .memoryCachePolicy(CachePolicy.DISABLED)
                            .diskCacheKey(artworkCacheKey("qa", url))
                            .size(32)
                            .build()
                    )
                assertTrue("$result", result is SuccessResult)
            }
            withTimeout(12000) { responder.await() }
            assertEquals(2, requests.size)
            assertEquals(1, payloads)
            assertTrue(requests[1].contains("If-None-Match: \"image-v1\"", true))
            assertFalse(requests.any { it.contains("Authorization:", true) })
            server.close()
            val offline =
                graph.images.execute(
                    ImageRequest.Builder(application)
                        .data(url)
                        .memoryCachePolicy(CachePolicy.DISABLED)
                        .networkCachePolicy(CachePolicy.DISABLED)
                        .diskCacheKey(artworkCacheKey("qa", url))
                        .size(32)
                        .build()
                )
            assertTrue("Offline cached artwork: $offline", offline is SuccessResult)
        } finally {
            server.close()
            responder.cancel()
        }
    }
}
