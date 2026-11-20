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
import androidx.compose.ui.unit.*
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*
import org.json.JSONObject

internal val sorts =
    listOf(
        "default" to "默认顺序",
        "title" to "曲名",
        "artist" to "艺术家",
        "release_new" to "最新发行",
        "release_old" to "最早发行",
        "plays" to "聆听次数",
        "duration" to "时长",
    )

@Composable
fun LibraryPage(graph: AppGraph, a: Actions, kind: String) {
    var tab by remember { mutableStateOf("tracks") }
    var sort by remember { mutableStateOf("default") }
    var format by remember { mutableStateOf("") }
    var played by remember { mutableStateOf("") }
    if (kind == "favorites") {
        val f = rememberPage(graph, "favorites")
        val id = f.value.text("id")
        if (id.isNotBlank())
            TracksPage(graph, a, Page("tracks", id, "我喜欢", "playlists/${pathId(id)}/tracks"))
        else
            Column(Modifier.padding(22.dp)) {
                PageTitle("我喜欢")
                Loading(f)
            }
        return
    }
    Column(Modifier.fillMaxSize().padding(horizontal = 22.dp)) {
        PageTitle("音乐空间")
        Row(
            Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            listOf("tracks" to "歌曲", "albums" to "专辑", "artists" to "艺术家").forEach { (k, n) ->
                FilterChip(tab == k, { tab = k }, label = { Text(n) })
            }
        }
        Row(
            Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Choice("排序", sort, sorts) { sort = it }
            if (tab == "tracks") {
                Choice(
                    "格式",
                    format,
                    listOf(
                        "" to "全部",
                        "flac" to "FLAC",
                        "ogg" to "OGG",
                        "mp3" to "MP3",
                        "dsf" to "DSD",
                    ),
                ) {
                    format = it
                }
                Choice("聆听", played, listOf("" to "全部", "unplayed" to "未听过", "played" to "听过")) {
                    played = it
                }
            }
        }
        val path = "library/$tab?sort=$sort&format=$format&listening=$played"
        key(path) {
            if (tab == "tracks") PagedTracks(graph, a, path)
            else EntityList(graph, a, path, if (tab == "artists") "artist" else "album")
        }
    }
}

@Composable
fun PagedTracks(graph: AppGraph, a: Actions, path: String) {
    var rows by remember(path) { mutableStateOf<List<JSONObject>>(emptyList()) }
    var total by remember(path) { mutableIntStateOf(0) }
    var offset by remember(path) { mutableIntStateOf(0) }
    var error by remember(path) { mutableStateOf("") }
    var busy by remember(path) { mutableStateOf(false) }
    var revision by remember(path) { mutableIntStateOf(0) }
    LaunchedEffect(path, offset, revision) {
        busy = true
        val uri = "$path${if('?' in path)'&' else '?'}limit=50&offset=$offset"
        if (offset == 0)
            graph.repository.cached(uri)?.let {
                rows = it.rows()
                total = it.optInt("total", rows.size)
            }
        try {
            val data = graph.repository.refresh(uri)
            val next =
                data
                    .rows()
                    .ifEmpty { data.rows("versions") }
                    .map { it.optJSONObject("track") ?: it }
            val combined = if (offset == 0) next else rows + next
            rows =
                if (path.startsWith("playlists/")) combined
                else combined.distinctBy { it.text("id") }
            total = data.optInt("total", rows.size)
            error = ""
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            error = e.message ?: "加载失败"
        } finally {
            busy = false
        }
    }
    LazyColumn(
        verticalArrangement = Arrangement.spacedBy(6.dp),
        contentPadding = PaddingValues(bottom = 22.dp),
    ) {
        item {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                TextButton({
                    a.selecting = !a.selecting
                    if (!a.selecting) a.selection = emptyMap()
                }) {
                    Text(if (a.selecting) "取消" else "多选")
                }
                if (a.selecting)
                    TextButton({ a.selection = rows.map(::Track).associateBy { it.id } }) {
                        Text("全选已加载")
                    }
                Text(
                    "$total 首",
                    Modifier.weight(1f),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                TextButton({ a.run { graph.runtime.playAll(path) } }) {
                    Icon(Icons.Rounded.PlayArrow, null)
                    Text("播放全部")
                }
            }
        }
        if (busy) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        itemsIndexed(rows, key = { i, v -> "${v.text("id")}:$i" }) { i, v ->
            TrackRow(
                graph,
                Track(v),
                a,
                { a.play(rows, i) },
                removeFromPlaylist =
                    if (path.startsWith("playlists/")) {
                        {
                            a.run {
                                graph.api.value!!.request(
                                    path.substringBefore('?'),
                                    "POST",
                                    json("track_id" to v.text("id"), "remove" to true),
                                )
                                offset = 0
                                revision++
                                a.message = "已移出播放列表"
                            }
                        }
                    } else null,
            )
        }
        if (error.isNotBlank()) item { TextButton({ revision++ }) { Text("$error · 重试") } }
        if (rows.size < total && !busy)
            item {
                LaunchedEffect(offset) { offset = rows.size }
                Notice("加载更多…")
            }
        if (rows.isEmpty() && !busy && error.isBlank()) item { Notice("这里还没有歌曲") }
    }
}

@Composable
fun TracksPage(graph: AppGraph, a: Actions, p: Page) {
    Column(Modifier.fillMaxSize().padding(horizontal = 22.dp)) {
        PageTitle(p.name)
        PagedTracks(graph, a, p.path)
    }
}

@Composable
fun EntityList(graph: AppGraph, a: Actions, path: String, kind: String) {
    var offset by remember(path) { mutableIntStateOf(0) }
    var rows by remember(path) { mutableStateOf<List<JSONObject>>(emptyList()) }
    var total by remember(path) { mutableIntStateOf(0) }
    val data = rememberPage(graph, "$path&limit=50&offset=$offset")
    LaunchedEffect(data.value) {
        if (data.value.has("items")) {
            rows =
                if (offset == 0) data.value.rows()
                else (rows + data.value.rows()).distinctBy { it.text("id") }
            total = data.value.optInt("total", rows.size)
        }
    }
    LazyColumn(contentPadding = PaddingValues(bottom = 20.dp)) {
        item { Loading(data) }
        items(rows, key = { it.text("id") }) { v ->
            ListItem(
                headlineContent = { Text(v.text("title", v.text("name"))) },
                supportingContent = {
                    Text(v.text("artist", "${v.optInt("track_count",v.optInt("count"))} 首"))
                },
                leadingContent = {
                    Artwork(
                        graph,
                        v.text("cover_url", v.text("image_url")),
                        Modifier.size(64.dp),
                        kind == "artist",
                    )
                },
                colors =
                    ListItemDefaults.colors(
                        containerColor = androidx.compose.ui.graphics.Color.Transparent
                    ),
                modifier =
                    Modifier.clip(RoundedCornerShape(16.dp)).clickable {
                        a.nav.open(Page(kind, v.text("id"), v.text("title", v.text("name"))))
                    },
            )
        }
        if (rows.size < total && !data.busy && data.error.isBlank())
            item {
                LaunchedEffect(offset) { offset = rows.size }
                Notice("加载更多…")
            }
    }
}
