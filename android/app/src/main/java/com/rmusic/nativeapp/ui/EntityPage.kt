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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*

@Composable
fun EntityPage(graph: AppGraph, a: Actions, p: Page) {
    var editing by remember(p) { mutableStateOf(false) }
    var tab by remember(p) { mutableStateOf("all") }
    var sort by remember(p) { mutableStateOf("default") }
    var pageNum by remember(p) { mutableIntStateOf(1) }
    var localPage by remember(p) { mutableIntStateOf(1) }
    val base = "library/catalog/${p.kind}/${pathId(p.id)}"
    val data =
        rememberPage(
            graph,
            "$base?sort=${if(sort=="hot")"default" else sort}&platform_sort=$sort&page=$pageNum&local_page=$localPage",
        )
    val albums =
        if (p.kind == "artist" && tab == "albums")
            rememberPage(graph, "$base/albums?sort=$sort&page=$pageNum")
        else null
    val body = if (albums != null) albums.value else data.value
    val profile = data.value.child("profile")
    val local = body.child("local")
    val platforms = body.rows("platforms")
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Artwork(
                    graph,
                    profile.text("image_url", data.value.child("anchor").text("cover_url")),
                    Modifier.size(108.dp),
                    p.kind == "artist",
                )
                Column(Modifier.padding(start = 20.dp).weight(1f)) {
                    Text(
                        if (p.kind == "artist") "艺术家" else "专辑",
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        data.value.text("name", p.name),
                        style = MaterialTheme.typography.headlineMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(
                        profile.text("release_date", profile.text("country")),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Loading(data)
            albums?.let { Loading(it) }
        }
        if (profile.text("description").isNotBlank())
            item {
                var expanded by remember { mutableStateOf(false) }
                GlassPanel {
                    Text(
                        profile.text("description"),
                        maxLines = if (expanded) Int.MAX_VALUE else 4,
                        overflow = TextOverflow.Ellipsis,
                    )
                    TextButton({ expanded = !expanded }) { Text(if (expanded) "收起" else "展开完整简介") }
                }
            }
        item {
            Row(
                Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                listOf("all" to "全部曲目", "local" to "本地曲库")
                    .plus(if (p.kind == "artist") listOf("albums" to "专辑") else emptyList())
                    .forEach { (k, n) ->
                        FilterChip(
                            tab == k,
                            {
                                tab = k
                                pageNum = 1
                            },
                            label = { Text(n) },
                        )
                    }
                Choice("排序", sort, sorts + listOf("hot" to "平台热度")) {
                    sort = it
                    pageNum = 1
                    localPage = 1
                }
                IconButton(
                    { editing = true },
                    enabled = !data.busy && data.error.isBlank() && data.value.has("profile"),
                ) {
                    Icon(Icons.Rounded.Edit, "编辑资料")
                }
                IconButton({
                    data.reload()
                    albums?.reload()
                }) {
                    Icon(Icons.Rounded.Refresh, "刷新资料")
                }
            }
        }
        if (tab == "local" || platforms.none { it.rows().isNotEmpty() } || tab == "albums") {
            item { Section(if (tab == "albums") "曲库专辑" else "本地曲库") }
            itemsIndexed(local.rows(), key = { _, v -> "local:${v.text("id")}" }) { i, v ->
                if (tab == "albums")
                    AlbumCard(graph, v) { a.nav.open(Page("album", v.text("id"), v.text("title"))) }
                else TrackRow(graph, Track(v), a, { a.play(local.rows(), i) })
            }
            if (local.optInt("next_page") > 0)
                item { TextButton({ localPage++ }) { Text("下一页本地曲库") } }
        }
        if (tab != "local")
            for (platform in platforms) {
                item {
                    Section(platform.text("name", platform.text("source")))
                    if (platform.text("notice").isNotBlank()) Notice(platform.text("notice"))
                }
                itemsIndexed(
                    platform.rows(),
                    key = { i, v -> "${platform.text("source")}:${v.text("id")}:$i" },
                ) { i, v ->
                    if (tab == "albums")
                        AlbumCard(graph, v) {
                            a.nav.open(Page("album", v.text("id"), v.text("title")))
                        }
                    else TrackRow(graph, Track(v), a, { a.play(platform.rows(), i) })
                }
                if (platform.optInt("next_page") > 0)
                    item {
                        TextButton({ pageNum = platform.optInt("next_page") }) { Text("下一页平台目录") }
                    }
            }
    }
    if (editing)
        EditProfile(
            graph,
            a,
            p,
            profile,
            data.value.optLong("profile_revision"),
            { editing = false },
        ) {
            data.reload()
        }
}
