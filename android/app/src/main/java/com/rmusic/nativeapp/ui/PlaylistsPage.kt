@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
)

package com.rmusic.nativeapp.ui

import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.*
import org.json.JSONArray
import org.json.JSONObject

@Composable
fun PlaylistsPage(graph: AppGraph, a: Actions) {
    val page = rememberPage(graph, "playlists")
    val cloud = rememberPage(graph, "playlist-directory")
    var editing by remember { mutableStateOf<JSONObject?>(null) }
    var deleting by remember { mutableStateOf<JSONObject?>(null) }
    var name by remember { mutableStateOf("") }
    var exportId by remember { mutableStateOf("") }
    var exportFormat by remember { mutableStateOf("json") }
    val export =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/octet-stream")
        ) { uri ->
            if (uri != null)
                a.run {
                    val file =
                        graph.api.value!!.textFile(
                            "playlists/${pathId(exportId)}/export?format=$exportFormat"
                        )
                    withContext(Dispatchers.IO) {
                        graph.application.contentResolver.openOutputStream(uri, "w")!!.use {
                            it.write(file.toByteArray())
                        }
                    }
                    a.message = "播放列表已导出"
                }
        }
    val import =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri != null)
                a.run {
                    val c = graph.application.contentResolver
                    val filename =
                        c.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                            ?.use { if (it.moveToFirst()) it.getString(0) else "" }
                            .orEmpty()
                    val format = filename.substringAfterLast('.').lowercase()
                    require(format in setOf("json", "m3u", "m3u8", "csv", "pls", "xspf")) {
                        "支持 JSON、M3U、CSV、PLS、XSPF 播放列表"
                    }
                    val text =
                        withContext(Dispatchers.IO) {
                            c.openInputStream(uri)!!.use { input ->
                                val out = java.io.ByteArrayOutputStream()
                                val buf = ByteArray(16384)
                                while (true) {
                                    val n = input.read(buf)
                                    if (n < 0) break
                                    require(out.size() + n <= 2 * 1024 * 1024) { "播放列表不能超过2 MiB" }
                                    out.write(buf, 0, n)
                                }
                                out.toString("UTF-8")
                            }
                        }
                    a.importFile = format to text
                    a.nav.open(Page("import", name = filename.substringBeforeLast('.')))
                }
        }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            PageTitle(
                "播放列表",
                action = {
                    Row {
                        IconButton({ import.launch(arrayOf("*/*")) }) {
                            Icon(Icons.Rounded.FileUpload, "导入播放列表")
                        }
                        IconButton({
                            editing = JSONObject()
                            name = ""
                        }) {
                            Icon(Icons.Rounded.Add, "创建播放列表")
                        }
                    }
                },
            )
            Loading(page)
            Loading(cloud)
        }
        items(page.value.rows(), key = { it.text("id") }) { v ->
            val id = v.text("id")
            var menu by remember(id) { mutableStateOf(false) }
            GlassPanel(
                Modifier.fillMaxWidth().clickable {
                    if (a.nav.current.id.startsWith("add:"))
                        a.run {
                            graph.api.value!!.request(
                                "playlists/${pathId(id)}/tracks",
                                "POST",
                                json("track_id" to a.nav.current.id.removePrefix("add:")),
                            )
                            a.message = "已加入播放列表"
                            a.nav.back()
                        }
                    else
                        a.nav.open(
                            Page("tracks", id, v.text("name"), "playlists/${pathId(id)}/tracks")
                        )
                }
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.AutoMirrored.Rounded.QueueMusic, null, Modifier.size(32.dp))
                    Column(Modifier.weight(1f).padding(start = 16.dp)) {
                        Text(v.text("name"), style = MaterialTheme.typography.titleMedium)
                        Text(
                            "${v.optJSONArray("track_ids")?.length() ?: 0} 首",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Box {
                        IconButton({ menu = true }) { Icon(Icons.Rounded.MoreVert, "播放列表管理") }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem(
                                { Text("播放全部") },
                                {
                                    a.run {
                                        graph.runtime.playAll("playlists/${pathId(id)}/tracks")
                                    }
                                    menu = false
                                },
                            )
                            DropdownMenuItem(
                                { Text("重命名") },
                                {
                                    editing = v
                                    name = v.text("name")
                                    menu = false
                                },
                            )
                            DropdownMenuItem(
                                { Text("导出 JSON") },
                                {
                                    exportId = id
                                    exportFormat = "json"
                                    export.launch(v.text("name") + ".json")
                                    menu = false
                                },
                            )
                            DropdownMenuItem(
                                { Text("导出 M3U8") },
                                {
                                    exportId = id
                                    exportFormat = "m3u8"
                                    export.launch(v.text("name") + ".m3u8")
                                    menu = false
                                },
                            )
                            DropdownMenuItem(
                                { Text("刷新云端同步") },
                                {
                                    a.run {
                                        graph.api.value!!.request(
                                            "playlists/${pathId(id)}/sync",
                                            "POST",
                                        )
                                        page.reload()
                                    }
                                    menu = false
                                },
                                enabled = v.has("sync"),
                            )
                            DropdownMenuItem(
                                { Text("删除播放列表") },
                                {
                                    deleting = v
                                    menu = false
                                },
                                enabled = !id.startsWith("favorites-"),
                            )
                        }
                    }
                }
            }
        }
        item { Section("云端播放列表") }
        items(cloud.value.rows(), key = { it.text("source") + it.text("id") }) { v ->
            GlassPanel {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Artwork(graph, v.text("cover_url"), Modifier.size(64.dp))
                    Column(Modifier.weight(1f).padding(start = 14.dp)) {
                        Text(
                            v.text("name", v.text("title")),
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Text(v.text("source"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                Button({
                    a.run {
                        graph.api.value!!.request(
                            "playlists/bind",
                            "POST",
                            json(
                                "provider" to v.text("source"),
                                "playlist_id" to v.text("id"),
                                "account_context" to v.text("account_context"),
                            ),
                        )
                        page.reload()
                        a.message = "已开始同步"
                    }
                }) {
                    Text("同步到曲库")
                }
            }
        }
        item {
            TextButton({
                a.run {
                    graph.api.value!!.request("playlist-directory/refresh", "POST")
                    cloud.reload()
                }
            }) {
                Text("刷新云端目录")
            }
        }
    }
    editing?.let { v ->
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text(if (v.has("id")) "重命名播放列表" else "创建播放列表") },
            text = { OutlinedTextField(name, { name = it }, label = { Text("名称") }) },
            confirmButton = {
                TextButton(
                    {
                        a.run {
                            if (v.has("id"))
                                graph.api.value!!.request(
                                    "playlists/${pathId(v.text("id"))}",
                                    "PUT",
                                    json(
                                        "name" to name,
                                        "expected" to
                                            json(
                                                "name" to v.text("name"),
                                                "track_ids" to v.optJSONArray("track_ids"),
                                            ),
                                    ),
                                )
                            else
                                graph.api.value!!.request(
                                    "playlists",
                                    "POST",
                                    json("name" to name, "track_ids" to JSONArray()),
                                )
                            editing = null
                            page.reload()
                        }
                    },
                    enabled = name.isNotBlank(),
                ) {
                    Text("保存")
                }
            },
            dismissButton = { TextButton({ editing = null }) { Text("取消") } },
        )
    }
    deleting?.let { v ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("删除“${v.text("name")}”？") },
            text = { Text("只删除这份播放列表，不删除歌曲文件。") },
            confirmButton = {
                TextButton({
                    a.run {
                        graph.api.value!!.request("playlists/${pathId(v.text("id"))}", "DELETE")
                        deleting = null
                        page.reload()
                    }
                }) {
                    Text("删除")
                }
            },
            dismissButton = { TextButton({ deleting = null }) { Text("取消") } },
        )
    }
}

@Composable
fun PlaylistImportPage(graph: AppGraph, a: Actions) {
    val file = a.importFile
    if (file == null) {
        Column(Modifier.padding(22.dp)) {
            PageTitle("导入播放列表")
            Notice("导入内容已释放，请重新选择文件")
            TextButton({ a.nav.back() }) { Text("返回") }
        }
        return
    }
    var offset by remember { mutableIntStateOf(0) }
    var data by remember { mutableStateOf(JSONObject()) }
    var selected by remember { mutableStateOf<Map<Int, String>>(emptyMap()) }
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf(a.nav.current.name) }
    LaunchedEffect(offset) {
        busy = true
        try {
            data =
                graph.api.value!!.request(
                    "playlists/import/preview",
                    "POST",
                    json(
                        "format" to file.first,
                        "content" to file.second,
                        "offset" to offset,
                        "limit" to 50,
                    ),
                )
            data.rows().forEachIndexed { i, row ->
                if (row.text("status") == "matched")
                    selected = selected + ((offset + i) to row.child("matched").text("id"))
            }
            error = ""
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            error = e.message ?: "预览失败"
        } finally {
            busy = false
        }
    }
    DisposableEffect(Unit) { onDispose { a.importFile = null } }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            PageTitle("导入播放列表", "只导入明确匹配的曲目，不读取导入文件中的音频地址")
            OutlinedTextField(
                name,
                { name = it },
                label = { Text("播放列表名称") },
                modifier = Modifier.fillMaxWidth(),
            )
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error.isNotBlank()) Notice(error)
            Text("已选 ${selected.size} / ${data.optInt("total")} 首")
        }
        itemsIndexed(data.rows()) { i, row ->
            val key = offset + i
            GlassPanel {
                Text(row.child("entry").text("title"))
                val choices =
                    if (row.text("status") == "matched") listOf(row.child("matched"))
                    else row.rows("candidates")
                if (choices.isEmpty()) Notice("没有明确匹配的曲目")
                choices.forEach { candidate ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(
                            selected[key] == candidate.text("id"),
                            { selected = selected + (key to candidate.text("id")) },
                        )
                        Text(
                            "${candidate.text("title")} · ${candidate.text("artist")}",
                            Modifier.weight(1f),
                        )
                        IconButton({ selected = selected - key }) {
                            Icon(Icons.Rounded.Close, "不导入此项")
                        }
                    }
                }
            }
        }
        item {
            Row {
                TextButton({ offset = (offset - 50).coerceAtLeast(0) }, enabled = offset > 0) {
                    Text("上一页")
                }
                TextButton({ offset += 50 }, enabled = offset + 50 < data.optInt("total")) {
                    Text("下一页")
                }
            }
            Button(
                {
                    a.run {
                        val ids = selected.toSortedMap().values.filter { it.isNotBlank() }
                        graph.api.value!!.request(
                            "playlists/import",
                            "POST",
                            json(
                                "name" to name,
                                "request_id" to java.util.UUID.randomUUID().toString(),
                                "track_ids" to JSONArray(ids),
                            ),
                        )
                        a.importFile = null
                        a.nav.root("playlists")
                        a.message = "已导入 ${ids.size} 首"
                    }
                },
                enabled = selected.isNotEmpty() && name.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("导入已确认曲目")
            }
        }
    }
}
