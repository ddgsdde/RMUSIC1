@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import androidx.test.platform.app.InstrumentationRegistry
import com.rmusic.core.*
import com.rmusic.media.*
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Actual HTTP/cache bytes, interrupted reader, new transfer object, strong identity and ranges. */
class DownloadResumeTest {
    @Test fun interruptedOriginalResumesOnlyWithUnchangedEntity() = exercise(false)

    @Test fun changedApprovalRestartsWithoutSplicingOldBytes() = exercise(true)

    private fun exercise(changed: Boolean) = runBlocking {
        val graph =
            (InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
                    as RMusicApplication)
                .graph
        val size = 2 * 1024 * 1024
        val data = ByteArray(size) { (it % 241).toByte() }
        val replacement = ByteArray(size) { (data[it].toInt() xor 0x5a).toByte() }
        val server = ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"))
        val requests = CopyOnWriteArrayList<String>()
        val sent = AtomicLong()
        val wire = java.util.concurrent.ConcurrentHashMap<Int, AtomicLong>()
        val tag = java.util.concurrent.atomic.AtomicReference("resume-v1")
        val serverJob =
            launch(Dispatchers.IO) {
                while (isActive) {
                    val client =
                        try {
                            server.accept()
                        } catch (e: Exception) {
                            break
                        }
                    launch {
                        client.use { c ->
                            runCatching {
                                c.soTimeout = 10000
                                val r = c.getInputStream().bufferedReader()
                                val lines = mutableListOf<String>()
                                while (true) {
                                    val l = r.readLine() ?: break
                                    if (l.isEmpty()) break
                                    lines += l
                                }
                                val first = lines.first()
                                val requestIndex = requests.size
                                requests += lines.joinToString("\n")
                                val out = c.getOutputStream()
                                if (first.contains("/file ")) {
                                    val range =
                                        lines
                                            .firstOrNull { it.startsWith("Range:", true) }
                                            ?.substringAfter("bytes=")
                                            ?.substringBefore('-')
                                            ?.toIntOrNull() ?: 0
                                    val head = first.startsWith("HEAD ")
                                    val payload =
                                        if (tag.get() == "resume-v2") replacement else data
                                    val header =
                                        "HTTP/1.1 ${if(range>0)"206 Partial Content" else "200 OK"}\r\nContent-Length: ${size-range}\r\nETag: \"${tag.get()}\"\r\nAccept-Ranges: bytes\r\n" +
                                            (if (range > 0)
                                                "Content-Range: bytes $range-${size-1}/$size\r\n"
                                            else "") +
                                            "Connection: close\r\n\r\n"
                                    out.write(header.toByteArray())
                                    if (!head) {
                                        var offset = range
                                        while (offset < size) {
                                            val n = minOf(32768, size - offset)
                                            out.write(payload, offset, n)
                                            out.flush()
                                            offset += n
                                            sent.addAndGet(n.toLong())
                                            wire
                                                .computeIfAbsent(requestIndex) { AtomicLong() }
                                                .addAndGet(n.toLong())
                                            delay(8)
                                        }
                                    }
                                } else {
                                    val bytes =
                                        json(
                                                "id" to "approved",
                                                "status" to "complete",
                                                "source_audio" to
                                                    json(
                                                        "verified" to true,
                                                        "format" to "wav",
                                                        "sample_rate" to 48000,
                                                        "bit_depth" to 16,
                                                        "channels" to 2,
                                                        "duration" to 10,
                                                    ),
                                            )
                                            .toString()
                                            .toByteArray()
                                    out.write(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n"
                                            .toByteArray()
                                    )
                                    out.write(bytes)
                                }
                            }
                        }
                    }
                }
            }
        val api =
            CoreApi(
                CoreSession(
                    "http://127.0.0.1:${server.localPort}",
                    "qa-only-token",
                    "fixture-core",
                    "fixture-user",
                    "qa",
                    false,
                )
            )
        val track =
            Track(
                json(
                    "id" to "resume-${java.util.UUID.randomUUID()}",
                    "title" to "Resume fixture",
                    "format" to "wav",
                    "sample_rate" to 48000,
                    "bit_depth" to 16,
                    "channels" to 2,
                )
            )
        val policy = graph.preferences.policy.value
        var latest = DownloadTask(track, api.session.cacheNamespace, approval = "approved")
        val keys = mutableSetOf<String>()
        try {
            withContext(Dispatchers.Main) {
                graph.preferences.save(
                    policy.copy(networkMode = "wifi", outputMode = OutputMode.AUTO)
                )
            }
            val first = DownloadTransfer(graph)
            val failure = java.util.concurrent.atomic.AtomicReference<Throwable>()
            val job =
                launch(Dispatchers.IO) {
                    runCatching {
                        first.execute(api, latest) {
                            latest = it
                            if (it.cacheKey.isNotBlank()) keys += it.cacheKey
                        }
                    }
                        .onFailure { failure.set(it) }
                }
            withTimeout(15000) {
                while (sent.get() < 700000) {
                    check(failure.get() == null) {
                        "Transfer failure: ${failure.get()} requests=${requests.map { it.lineSequence().first() }}"
                    }
                    delay(10)
                }
            }
            first.cancel()
            job.cancelAndJoin()
            assertTrue(latest.cacheKey.isNotBlank())
            val key = latest.cacheKey
            assertTrue(graph.mediaCache.store.getCachedBytes(key, 0, size.toLong()) > 0)
            if (changed) tag.set("resume-v2")
            val requestsBefore = requests.size
            val done =
                withContext(Dispatchers.IO) {
                    DownloadTransfer(graph).execute(api, latest) {
                        latest = it
                        if (it.cacheKey.isNotBlank()) keys += it.cacheKey
                    }
                }
            assertEquals("已缓存", done.status)
            if (changed) assertNotEquals(key, done.cacheKey) else assertEquals(key, done.cacheKey)
            assertEquals(size.toLong(), done.bytes)
            val reads =
                requests.drop(requestsBefore).filter {
                    it.startsWith("GET ") && it.contains("/file ")
                }
            val resumed = reads.any {
                Regex("Range: bytes=[1-9][0-9]*-", RegexOption.IGNORE_CASE).containsMatchIn(it)
            }
            assertEquals("Range requests: $reads", !changed, resumed)
            if (!changed)
                assertTrue(
                    wire.entries.filter { it.key >= requestsBefore }.sumOf { it.value.get() } < size
                )
            assertTrue(graph.mediaCache.store.isCached(done.cacheKey, 0, size.toLong()))
            val cacheRead =
                androidx.media3.datasource.cache.CacheDataSource.Factory()
                    .setCache(graph.mediaCache.store)
                    .createDataSource()
            val spec =
                androidx.media3.datasource.DataSpec.Builder()
                    .setUri("https://cache.invalid/verify")
                    .setKey(done.cacheKey)
                    .build()
            val actual =
                androidx.media3.datasource.DataSourceInputStream(cacheRead, spec).use {
                    it.readBytes()
                }
            assertArrayEquals(if (changed) replacement else data, actual)
        } finally {
            withContext(Dispatchers.Main) { graph.preferences.save(policy) }
            api.close()
            server.close()
            serverJob.cancelAndJoin()
            keys.forEach { graph.mediaCache.remove(it) }
        }
    }
}
