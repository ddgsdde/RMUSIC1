@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.rmusic.nativeapp.ui

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material.icons.automirrored.rounded.*
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.*
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*

@Composable
fun SearchPage(graph: AppGraph, a: Actions, query: String) {
    val page = rememberPage(graph, "search?q=${pathId(query)}&limit=60")
    var tab by remember(query) { mutableStateOf("all") }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            PageTitle("“$query”")
            Loading(page)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("all" to "综合", "items" to "歌曲", "albums" to "专辑", "artists" to "艺术家")
                    .forEach { (k, n) -> FilterChip(tab == k, { tab = k }, label = { Text(n) }) }
            }
        }
        for ((key, title, kind) in
            listOf(Triple("artists", "艺术家", "artist"), Triple("albums", "专辑", "album"))) {
            if (
                (tab == "all" || tab == key) && (tab != "all" || page.value.rows(key).isNotEmpty())
            ) {
                item { Section(title) }
                items(
                    page.value.rows(key).take(if (tab == "all") 6 else 60),
                    key = { "$key:${it.text("id")}" },
                ) { v ->
                    ListItem(
                        headlineContent = { Text(v.text("name", v.text("title"))) },
                        supportingContent = { Text(v.text("source")) },
                        leadingContent = {
                            Artwork(
                                graph,
                                v.text("image_url", v.text("cover_url")),
                                Modifier.size(64.dp),
                                kind == "artist",
                            )
                        },
                        colors =
                            ListItemDefaults.colors(
                                containerColor = androidx.compose.ui.graphics.Color.Transparent
                            ),
                        modifier =
                            Modifier.clickable {
                                a.nav.open(
                                    Page(kind, v.text("id"), v.text("name", v.text("title")))
                                )
                            },
                    )
                }
            }
        }
        if (tab == "all" || tab == "items") {
            item { Section("歌曲") }
            itemsIndexed(page.value.rows(), key = { _, v -> "song:${v.text("id")}" }) { i, v ->
                TrackRow(graph, Track(v), a, { a.play(page.value.rows(), i) })
            }
        }
        item {
            val notice = page.value.text("notice")
            if (notice.isNotBlank()) Notice(notice)
        }
    }
}
