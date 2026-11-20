@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.ui

import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rmusic.core.*
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun DownloadsPage(graph: AppGraph, a: Actions) {
    val jobs by graph.downloads.items.collectAsStateWithLifecycle()
    var entries by remember {
        mutableStateOf(graph.mediaCache.entries(graph.api.value!!.session.cacheNamespace))
    }
    var directory by remember { mutableStateOf(graph.downloads.directory?.toString().orEmpty()) }
    val choose =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
            if (uri != null)
                a.run {
                    graph.application.contentResolver.takePersistableUriPermission(
                        uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION or
                            Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                    )
                    graph.downloads.directory = uri
                    directory = uri.toString()
                }
        }
    LaunchedEffect(Unit) { graph.downloads.refresh() }
    LaunchedEffect(jobs) {
        entries =
            withContext(Dispatchers.IO) {
                graph.mediaCache.entries(graph.api.value!!.session.cacheNamespace)
            }
    }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            PageTitle("下载与缓存")
            GlassPanel(Modifier.fillMaxWidth()) {
                Text(if (directory.isBlank()) "下载到应用缓存" else "已选择导出目录")
                Text(directory, style = MaterialTheme.typography.bodySmall, maxLines = 2)
                Row {
                    TextButton({ choose.launch(null) }) { Text("选择导出目录") }
                    TextButton({
                        graph.downloads.directory = null
                        directory = ""
                    }) {
                        Text("仅缓存")
                    }
                }
                Row {
                    TextButton({ graph.downloads.resumeInterrupted() }) { Text("恢复中断任务") }
                    TextButton({ graph.downloads.cancel() }) { Text("取消未完成下载") }
                }
            }
        }
        items(jobs, key = { "job:${it.id}" }) { job ->
            GlassPanel(Modifier.fillMaxWidth()) {
                Text(job.title)
                Text(
                    "${job.status} · ${job.bytes/1024} KiB",
                    color = MaterialTheme.colorScheme.primary,
                )
                if (job.total > 0 && job.status == "下载中")
                    LinearProgressIndicator(
                        progress = { (job.bytes.toFloat() / job.total).coerceIn(0f, 1f) },
                        modifier = Modifier.fillMaxWidth(),
                    )
                if (job.error.isNotBlank()) Text(job.error, color = MaterialTheme.colorScheme.error)
                Row {
                    if (job.status in setOf("等待", "下载中", "等待网络"))
                        TextButton({ graph.downloads.pause(job.id) }) { Text("暂停") }
                    if (job.resumable)
                        TextButton({ graph.downloads.resume(job.id) }) { Text("继续下载") }
                    if (job.status == "失败")
                        TextButton({ graph.downloads.resume(job.id, reapprove = true) }) {
                            Text("重新申请下载")
                        }
                }
            }
        }
        item {
            Section("可用的本机缓存")
            Text(
                "${entries.size} 首 · ${graph.mediaCache.bytes()/1024/1024} MiB",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        items(entries, key = { it.key }) { entry ->
            TrackRow(graph, entry.track, a, { a.run { graph.runtime.playCached(entry) } }) {
                IconButton({
                    a.run {
                        withContext(Dispatchers.IO) { graph.mediaCache.remove(entry.key) }
                        entries = graph.mediaCache.entries(entry.namespace)
                    }
                }) {
                    Icon(Icons.Rounded.DeleteOutline, "移除缓存")
                }
            }
        }
    }
}
