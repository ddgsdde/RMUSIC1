@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.rmusic.nativeapp.ui

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.*
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*

@Composable
fun HomePage(graph: AppGraph, a: Actions) {
    val resume by graph.recovery.offered.collectAsStateWithLifecycle()
    val page = rememberPage(graph, "library/home")
    val tracks = page.value.rows("tracks")
    val albums = page.value.rows("recent_albums").ifEmpty { page.value.rows("albums") }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(22.dp),
    ) {
        item {
            PageTitle("聆听", java.time.LocalDate.now().toString())
            Loading(page)
        }
        resume?.let { mark ->
            item {
                GlassPanel {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Artwork(graph, mark.track.cover, Modifier.size(62.dp))
                        Column(Modifier.weight(1f).padding(start = 14.dp)) {
                            Text("继续上次聆听", color = MaterialTheme.colorScheme.primary)
                            Text(mark.track.title, maxLines = 1)
                            Text(
                                "${mark.position.toInt()/60}:${(mark.position.toInt()%60).toString().padStart(2,'0')} · ${if(mark.offline) "本机缓存" else "Core播放列表"}",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        IconButton({ a.run { graph.recovery.resume() } }) {
                            Icon(Icons.Rounded.PlayArrow, "恢复上次播放")
                        }
                        IconButton({ graph.recovery.dismiss() }) {
                            Icon(Icons.Rounded.Close, "收起播放恢复")
                        }
                    }
                }
            }
        }
        if (tracks.isNotEmpty())
            item {
                val t = Track(tracks.first())
                GlassPanel {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                "为你精选",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.primary,
                                letterSpacing = 2.sp,
                            )
                            Spacer(Modifier.height(26.dp))
                            Text(
                                t.title,
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = FontWeight.Bold,
                                maxLines = 2,
                            )
                            ArtistLinks(t, a)
                            Spacer(Modifier.height(20.dp))
                            Button({ a.play(tracks) }) {
                                Icon(Icons.Rounded.PlayArrow, null)
                                Text("播放全部")
                            }
                        }
                        Artwork(graph, t.cover, Modifier.size(100.dp))
                    }
                }
            }
        item {
            Section("最近专辑") { a.nav.root("library") }
            LazyRow(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                items(albums, key = { it.text("id") }) { v ->
                    AlbumCard(graph, v) { a.nav.open(Page("album", v.text("id"), v.text("title"))) }
                }
            }
        }
        if (tracks.isNotEmpty()) item { Section("常听曲目") { a.nav.root("library") } }
        itemsIndexed(tracks, key = { _, v -> v.text("id") }) { i, v ->
            TrackRow(graph, Track(v), a, { a.play(tracks, i) })
        }
        item {
            Section("你的音乐")
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton({ a.nav.root("favorites") }) {
                    Icon(Icons.Rounded.FavoriteBorder, null)
                    Text("我喜欢")
                }
                OutlinedButton({ a.nav.root("stats") }) { Text("聆听统计") }
            }
        }
    }
}
