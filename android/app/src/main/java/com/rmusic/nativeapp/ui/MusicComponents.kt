@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.rmusic.nativeapp.ui

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.*
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*
import org.json.JSONObject

@Composable
fun Section(title: String, action: (() -> Unit)? = null) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            title,
            Modifier.weight(1f),
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold,
        )
        if (action != null)
            IconButton(action) { Icon(Icons.AutoMirrored.Rounded.ArrowForward, "查看全部") }
    }
}

@Composable
fun AlbumCard(graph: AppGraph, v: JSONObject, onClick: () -> Unit) {
    Column(
        Modifier.width(148.dp)
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick)
            .padding(4.dp)
    ) {
        Artwork(
            graph,
            v.text("cover_url", v.text("image_url")),
            Modifier.size(140.dp),
            v.text("kind") == "artist",
        )
        Spacer(Modifier.height(10.dp))
        Text(
            v.text("title", v.text("name")),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            fontWeight = FontWeight.Medium,
        )
        Text(
            v.text("artist", v.text("source")),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
fun ArtistLinks(t: Track, a: Actions) {
    val names =
        t.credits
            .map { it.text("name") }
            .filter { it.isNotBlank() }
            .ifEmpty { t.artist.split(" / ").map(String::trim).filter(String::isNotEmpty) }
    Row(Modifier.horizontalScroll(rememberScrollState())) {
        names.forEachIndexed { i, name ->
            if (i > 0)
                Text(
                    " · ",
                    Modifier.padding(vertical = 8.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            Text(
                name,
                Modifier.clip(RoundedCornerShape(8.dp))
                    .clickable { a.entity(t, "artist", name) }
                    .padding(vertical = 8.dp, horizontal = 2.dp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
    }
}

@Composable
fun TrackRow(
    graph: AppGraph,
    t: Track,
    a: Actions,
    play: () -> Unit,
    removeFromPlaylist: (() -> Unit)? = null,
    extra: (@Composable () -> Unit)? = null,
) {
    var menu by remember(t.id) { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .combinedClickable(
                onClick = { if (a.selecting) a.toggle(t) else play() },
                onLongClick = { menu = true },
            )
            .padding(vertical = 6.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(56.dp)) {
            Artwork(graph, t.cover, Modifier.fillMaxSize())
            IconButton(play, Modifier.fillMaxSize()) {
                Icon(
                    Icons.Rounded.PlayArrow,
                    "播放 ${t.title}",
                    tint = androidx.compose.ui.graphics.Color.White.copy(alpha = .85f),
                )
            }
        }
        Column(Modifier.weight(1f).padding(start = 12.dp)) {
            Text(
                t.title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontWeight = FontWeight.Medium,
            )
            ArtistLinks(t, a)
            if (t.rate > 0 || t.format.isNotBlank() && t.format != "unknown")
                Text(
                    listOf(
                            t.format.uppercase().takeUnless { it == "UNKNOWN" }.orEmpty(),
                            if (t.bits > 0) "${t.bits} bit" else "",
                            if (t.rate > 0) "${t.rate/1000.0} kHz" else "",
                        )
                        .filter { it.isNotBlank() }
                        .joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
        if (a.selecting) Checkbox(t.id in a.selection, { a.toggle(t) })
        extra?.invoke()
        Box {
            IconButton({ menu = true }) { Icon(Icons.Rounded.MoreVert, "歌曲操作") }
            DropdownMenu(menu, { menu = false }) {
                DropdownMenuItem(
                    { Text(if (t.id in a.favorites) "取消喜欢" else "加入我喜欢") },
                    {
                        a.favorite(t)
                        menu = false
                    },
                    leadingIcon = { Icon(Icons.Rounded.FavoriteBorder, null) },
                )
                DropdownMenuItem(
                    { Text("加入播放队列") },
                    {
                        a.run {
                            graph.api.value!!.request(
                                "zones/${pathId(graph.runtime.selectedZone.value)}/queue",
                                "POST",
                                json("track_ids" to org.json.JSONArray(listOf(t.id))),
                            )
                        }
                        menu = false
                    },
                )
                DropdownMenuItem(
                    { Text("加入播放列表") },
                    {
                        a.nav.open(Page("playlists", "add:${t.id}", "添加歌曲"))
                        menu = false
                    },
                )
                DropdownMenuItem(
                    { Text("前往专辑") },
                    {
                        a.entity(t, "album")
                        menu = false
                    },
                    enabled = t.album.isNotBlank(),
                )
                DropdownMenuItem(
                    { Text("下载到本机") },
                    {
                        graph.downloads.enqueue(t)
                        a.message = "已加入下载队列"
                        menu = false
                    },
                )
                if (removeFromPlaylist != null)
                    DropdownMenuItem(
                        { Text("移出播放列表") },
                        {
                            removeFromPlaylist()
                            menu = false
                        },
                    )
                DropdownMenuItem(
                    { Text("音乐版本") },
                    {
                        a.nav.open(
                            Page("tracks", t.id, "音乐版本", "library/tracks/${pathId(t.id)}/versions")
                        )
                        menu = false
                    },
                )
            }
        }
    }
}
