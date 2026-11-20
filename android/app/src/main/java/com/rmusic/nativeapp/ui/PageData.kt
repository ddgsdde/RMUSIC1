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
import org.json.JSONObject

class PageData {
    var value by mutableStateOf(JSONObject())
    var busy by mutableStateOf(true)
    var error by mutableStateOf("")
    var revision by mutableIntStateOf(0)

    fun reload() {
        revision++
    }
}

@Composable
fun rememberPage(graph: AppGraph, path: String): PageData {
    val state = remember(path) { PageData() }
    LaunchedEffect(path, state.revision) {
        state.busy = true
        graph.repository.cached(path)?.let { state.value = it }
        try {
            state.value = graph.repository.refresh(path)
            state.error = ""
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            state.error = e.message ?: "加载失败"
        } finally {
            state.busy = false
        }
    }
    return state
}

@Composable
fun Loading(state: PageData) {
    if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
    if (state.error.isNotBlank())
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                state.error,
                Modifier.weight(1f),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            TextButton({ state.reload() }) { Text("重试") }
        }
}
