package com.rmusic.nativeapp

import androidx.media3.common.util.UnstableApi
import coil3.ImageLoader
import coil3.disk.DiskCache
import coil3.memory.MemoryCache
import coil3.network.ConnectivityChecker
import coil3.network.cachecontrol.CacheControlCacheStrategy
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import com.rmusic.core.*
import java.io.File
import java.util.concurrent.TimeUnit
import okhttp3.OkHttpClient
import okio.Path.Companion.toPath

fun artworkCacheKey(namespace: String, url: String) = "$namespace:${stableHash(url)}"

fun artworkMemoryKey(namespace: String, url: String) =
    artworkCacheKey(namespace, url) +
        if (url.contains("/api/v1/covers/")) "" else ":${System.currentTimeMillis()/60000}"

/** A single image pool. Tokens are attached only to this Core's origin and API prefix. */
@UnstableApi
@OptIn(kotlin.time.ExperimentalTime::class)
fun AppGraph.artworkLoader(): ImageLoader {
    val client =
        OkHttpClient.Builder()
            .dispatcher(
                okhttp3.Dispatcher().apply {
                    maxRequests = 10
                    maxRequestsPerHost = 6
                }
            )
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .callTimeout(25, TimeUnit.SECONDS)
            .followRedirects(true)
            .followSslRedirects(false)
            .addNetworkInterceptor { chain ->
                val req = chain.request()
                val a = api.value
                val url = req.url
                val local =
                    a != null &&
                        url.host == a.base.host &&
                        url.port == a.base.port &&
                        url.scheme == a.base.scheme &&
                        url.encodedPath.startsWith(a.base.encodedPath.trimEnd('/') + "/api/")
                chain.proceed(
                    if (local)
                        req.newBuilder()
                            .header("Authorization", "Bearer ${a!!.session.token}")
                            .build()
                    else req.newBuilder().removeHeader("Authorization").build()
                )
            }
            .build()
    return ImageLoader.Builder(application)
        .memoryCache { MemoryCache.Builder().maxSizeBytes(24L * 1024 * 1024).build() }
        .diskCache {
            DiskCache.Builder()
                .directory(File(application.cacheDir, "artwork-v1").absolutePath.toPath())
                .maxSizeBytes(preferences.imageBytes)
                .build()
        }
        .components {
            add(
                OkHttpNetworkFetcherFactory(
                    callFactory = { client },
                    cacheStrategy = { CacheControlCacheStrategy() },
                    connectivityChecker = { ConnectivityChecker { true } },
                )
            )
        }
        .build()
}
