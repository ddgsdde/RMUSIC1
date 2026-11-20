@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.palette.graphics.Palette
import coil3.BitmapImage
import coil3.request.ImageRequest
import coil3.request.SuccessResult
import coil3.request.allowHardware
import com.rmusic.nativeapp.AppGraph
import com.rmusic.nativeapp.artworkCacheKey
import com.rmusic.nativeapp.artworkMemoryKey
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Tiny, one-shot CPU sample. No full-size bitmap or animation loop is retained. */
@Composable
fun albumPalette(graph: AppGraph, cover: String, fallback: Color): Color {
    val namespace = graph.api.value?.session?.cacheNamespace.orEmpty()
    val target by
        produceState(fallback, cover, namespace) {
            val url = graph.api.value?.image(cover) ?: return@produceState
            value =
                withContext(Dispatchers.IO) {
                    val result =
                        graph.images.execute(
                            ImageRequest.Builder(graph.application)
                                .data(url)
                                .networkCachePolicy(
                                    if (graph.runtime.selectedZone.value == "offline")
                                        coil3.request.CachePolicy.DISABLED
                                    else coil3.request.CachePolicy.ENABLED
                                )
                                .memoryCacheKey(artworkMemoryKey(namespace, url) + ":palette")
                                .diskCacheKey(artworkCacheKey(namespace, url))
                                .size(96)
                                .allowHardware(false)
                                .build()
                        )
                    val bitmap = ((result as? SuccessResult)?.image as? BitmapImage)?.bitmap
                    bitmap?.let {
                        Palette.from(it)
                            .maximumColorCount(8)
                            .generate()
                            .let { p -> p.mutedSwatch ?: p.vibrantSwatch ?: p.dominantSwatch }
                            ?.rgb
                            ?.let(::Color)
                    } ?: fallback
                }
        }
    return animateColorAsState(target, tween(700), label = "album colour").value
}
