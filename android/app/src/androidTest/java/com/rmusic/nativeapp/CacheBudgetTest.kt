@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp

import androidx.media3.common.C
import androidx.media3.datasource.*
import androidx.media3.datasource.cache.*
import androidx.test.platform.app.InstrumentationRegistry
import com.rmusic.media.PlaybackCacheSink
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

class CacheBudgetTest {
    private fun requireQa(graph: AppGraph) {
        val api = runBlocking { withTimeout(10000) { graph.api.first { it != null } } }!!
        check(
            api.session.address == "http://127.0.0.1:18874" && api.session.username == "android-qa"
        )
    }

    @Test
    fun activeCacheBudgetDoesNotBlockStreamingOrGrowWithoutLimit() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val graph = (context.applicationContext as RMusicApplication).graph
        requireQa(graph)
        val cache = graph.mediaCache
        val limit = 16L * 1024 * 1024
        cache.resize(limit)
        val key = "budget-qa-${java.util.UUID.randomUUID()}"
        cache.protect(key)
        val body = ByteArray(20 * 1024 * 1024) { 42 }
        val data =
            CacheDataSource.Factory()
                .setCache(cache.store)
                .setCacheWriteDataSinkFactory { PlaybackCacheSink(cache.store) }
                .setUpstreamDataSourceFactory { ByteArrayDataSource(body) }
                .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)
                .createDataSource()
        var total = 0L
        try {
            data.open(DataSpec.Builder().setUri("https://cache.invalid/qa").setKey(key).build())
            val buffer = ByteArray(65536)
            while (true) {
                val n = data.read(buffer, 0, buffer.size)
                if (n == C.RESULT_END_OF_INPUT) break
                total += n
            }
            assertEquals(body.size.toLong(), total)
            assertTrue(cache.bytes() <= limit)
        } finally {
            data.close()
            cache.unprotect(key)
            cache.remove(key)
            cache.resize(graph.preferences.musicBytes)
        }
    }

    @Test
    fun overlappingReadersRemainProtectedUntilLastRelease() {
        val graph =
            (InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
                    as RMusicApplication)
                .graph
        requireQa(graph)
        val cache = graph.mediaCache
        val key = "readers-qa-${java.util.UUID.randomUUID()}"
        val size = 20 * 1024 * 1024
        cache.resize(cache.bytes() + size + 1024 * 1024)
        cache.protect(key)
        cache.protect(key)
        val sink =
            CacheDataSink.Factory()
                .setCache(cache.store)
                .setFragmentSize(256 * 1024)
                .createDataSink()
        val hole = cache.store.startReadWrite(key, 0, size.toLong())
        var releases = 0
        try {
            sink.open(
                DataSpec.Builder()
                    .setUri("https://cache.invalid/readers")
                    .setKey(key)
                    .setLength(size.toLong())
                    .setFlags(DataSpec.FLAG_ALLOW_CACHE_FRAGMENTATION)
                    .build()
            )
            val block = ByteArray(65536) { 73 }
            repeat(size / block.size) { sink.write(block, 0, block.size) }
            sink.close()
            cache.resize(16L * 1024 * 1024)
            assertTrue(cache.store.isCached(key, 0, size.toLong()))
            cache.unprotect(key)
            releases++
            assertTrue(
                "One active reader still owns this file",
                cache.store.isCached(key, 0, size.toLong()),
            )
            cache.unprotect(key)
            releases++
            assertTrue(cache.bytes() <= 16L * 1024 * 1024)
        } finally {
            runCatching { sink.close() }
            cache.store.releaseHoleSpan(hole)
            repeat(2 - releases) { cache.unprotect(key) }
            cache.remove(key)
            cache.resize(graph.preferences.musicBytes)
        }
    }
}
