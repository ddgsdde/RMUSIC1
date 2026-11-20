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
import androidx.compose.ui.unit.*
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*

@Composable
fun DiscoveryPage(graph: AppGraph, a: Actions) {
    val data = rememberPage(graph, "recommendations?limit=40")
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            PageTitle(
                "发现音乐",
                action = { IconButton({ data.reload() }) { Icon(Icons.Rounded.Refresh, "换一批") } },
            )
            Loading(data)
            if (data.value.text("message").isNotBlank()) Notice(data.value.text("message"))
        }
        itemsIndexed(data.value.rows(), key = { i, v -> "top:$i:${v.text("id")}" }) { i, v ->
            TrackRow(graph, Track(v), a, { a.play(data.value.rows(), i) })
        }
        data.value.rows("sections").forEach { section ->
            item { Section(section.text("title")) }
            itemsIndexed(
                section.rows(),
                key = { i, v -> "${section.text("id")}:$i:${v.text("id")}" },
            ) { i, v ->
                TrackRow(graph, Track(v), a, { a.play(section.rows(), i) })
            }
        }
    }
}
