@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
)

package com.rmusic.nativeapp.ui

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.*
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rmusic.core.*
import com.rmusic.media.*
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.*

private val rates =
    listOf(44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000).map {
        it.toString() to "${it/1000.0} kHz"
    }

@Composable
private fun SettingSwitch(
    title: String,
    detail: String = "",
    value: Boolean,
    change: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(end = 16.dp)) {
            Text(title)
            if (detail.isNotBlank())
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
        Switch(value, change)
    }
}

@Composable
fun SettingsPage(graph: AppGraph, a: Actions, onTheme: (String) -> Unit) {
    val p by graph.preferences.policy.collectAsStateWithLifecycle()
    var music by remember {
        mutableStateOf((graph.preferences.musicBytes / 1024 / 1024).toString())
    }
    var images by remember {
        mutableStateOf((graph.preferences.imageBytes / 1024 / 1024).toString())
    }
    var resumeDownloads by remember { mutableStateOf(graph.preferences.resumeDownloads) }
    var name by remember { mutableStateOf(graph.preferences.clientName) }
    var backgroundControl by remember { mutableStateOf(graph.preferences.backgroundControl) }
    fun save(next: ListeningPolicy) = a.run { graph.preferences.save(next) }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item { PageTitle("本机设置", "Android 独立播放端") }
        item {
            GlassPanel {
                Section("外观")
                Row(
                    Modifier.horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    listOf("emerald" to "翡翠", "violet" to "紫晶", "ocean" to "海蓝", "amber" to "琥珀")
                        .forEach { (k, n) -> OutlinedButton({ onTheme(k) }) { Text(n) } }
                }
            }
        }
        item {
            GlassPanel {
                Section("外出与音质")
                Choice(
                    "网络策略",
                    p.networkMode,
                    listOf("auto" to "自动", "wifi" to "Wi-Fi 音质", "mobile" to "外出省流"),
                ) {
                    save(p.copy(networkMode = it))
                }
                SettingSwitch("外出使用 Opus", "明确有损；Wi-Fi 重播恢复兼容的无损格式", p.mobileOpus) {
                    save(p.copy(mobileOpus = it))
                }
                Choice(
                    "Opus",
                    p.opusKbps.toString(),
                    listOf(64, 96, 128, 160, 192, 256, 320).map { it.toString() to "$it kbps" },
                ) {
                    save(p.copy(opusKbps = it.toInt()))
                }
                Spacer(Modifier.height(12.dp))
                Choice("外出采样率上限", p.mobileMaxRate.toString(), rates) {
                    save(p.copy(mobileMaxRate = it.toInt()))
                }
                Choice(
                    "外出位深上限",
                    p.mobileMaxBits.toString(),
                    listOf("16" to "16 bit", "24" to "24 bit", "32" to "32 bit"),
                ) {
                    save(p.copy(mobileMaxBits = it.toInt()))
                }
                Spacer(Modifier.height(12.dp))
                Choice("Wi-Fi 采样率上限", p.wifiMaxRate.toString(), rates) {
                    save(p.copy(wifiMaxRate = it.toInt()))
                }
                Choice(
                    "Wi-Fi 位深上限",
                    p.wifiMaxBits.toString(),
                    listOf("16" to "16 bit", "24" to "24 bit", "32" to "32 bit"),
                ) {
                    save(p.copy(wifiMaxBits = it.toInt()))
                }
                Text(
                    "不主动升频。超出输出能力或上限时由 Core 转换，DSD 始终在 Core 解码。",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Choice(
                    "网络预取",
                    p.bufferSeconds.toString(),
                    listOf(5, 10, 15, 30, 60).map { it.toString() to "$it 秒" },
                ) {
                    save(p.copy(bufferSeconds = it.toInt()))
                }
            }
        }
        item {
            GlassPanel {
                Section("缓存与下载")
                SettingSwitch("重新打开后继续下载", "只恢复被系统中断的任务；手动暂停保持暂停", resumeDownloads) {
                    resumeDownloads = it
                    graph.preferences.resumeDownloads = it
                }
                Text(
                    "音乐 ${graph.mediaCache.bytes()/1024/1024} MiB",
                    color = MaterialTheme.colorScheme.primary,
                )
                OutlinedTextField(
                    music,
                    { music = it },
                    label = { Text("音乐预算 · MiB") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(
                    images,
                    { images = it },
                    label = { Text("图片预算 · MiB") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Button({
                    a.run {
                        val m = music.toLong() * 1024 * 1024
                        val i = images.toLong() * 1024 * 1024
                        require(
                            m in 16L * 1024 * 1024..32L * 1024 * 1024 * 1024 &&
                                i in 8L * 1024 * 1024..1024L * 1024 * 1024
                        ) {
                            "预算超出范围"
                        }
                        graph.preferences.musicBytes = m
                        graph.preferences.imageBytes = i
                        withContext(Dispatchers.IO) { graph.mediaCache.resize(m) }
                        a.message = "音乐预算已应用；图片预算下次启动生效"
                    }
                }) {
                    Text("保存预算")
                }
                Text(
                    "超限回收较久未用的缓存；播放中的内容暂时保留。已导出文件不参与回收。",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                TextButton({ a.nav.root("downloads") }) { Text("管理下载与缓存") }
            }
        }
        item {
            GlassPanel {
                Section("设备身份")
                SettingSwitch("后台远程控制", "保留常驻通知，允许其他设备控制此播放端", backgroundControl) { enabled ->
                    backgroundControl = enabled
                    graph.preferences.backgroundControl = enabled
                    if (enabled)
                        graph.application.startForegroundService(
                            android.content.Intent(
                                graph.application,
                                com.rmusic.nativeapp.playback.PlaybackService::class.java,
                            )
                        )
                    else
                        com.rmusic.nativeapp.playback.PlaybackService.current
                            .get()
                            ?.updateRemotePolicy()
                }
                OutlinedTextField(
                    name,
                    { name = it },
                    label = { Text("设备名称") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Button({
                    a.run {
                        graph.preferences.clientName = name
                        graph.runtime.rename(name)
                        val zone =
                            graph.runtime.zones.value.firstOrNull {
                                it.text("endpoint_id") == graph.preferences.endpointId
                            }
                        if (zone != null)
                            graph.api.value!!.request(
                                "zones/${pathId(zone.text("id"))}",
                                "PATCH",
                                json("name" to name),
                            )
                        a.message = "设备名称已保存"
                    }
                }) {
                    Text("保存名称")
                }
                Text(
                    graph.api.value?.session?.address.orEmpty(),
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedButton({ graph.logout() }) { Text("退出 Core") }
            }
        }
    }
}

@Composable
fun DevicesPage(graph: AppGraph, a: Actions) {
    val zones by graph.runtime.zones.collectAsStateWithLifecycle()
    val selected by graph.runtime.selectedZone.collectAsStateWithLifecycle()
    val p by graph.preferences.policy.collectAsStateWithLifecycle()
    var settings by remember { mutableStateOf(false) }
    var outputs by remember { mutableStateOf(graph.devices.list()) }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(22.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        item {
            PageTitle(
                "声音设备",
                action = {
                    IconButton({ outputs = graph.devices.list() }) {
                        Icon(Icons.Rounded.Refresh, "刷新设备")
                    }
                },
            )
        }
        if (zones.none { it.text("endpoint_id") == graph.preferences.endpointId })
            item {
                GlassPanel(Modifier.fillMaxWidth()) {
                    Text(graph.preferences.clientName, style = MaterialTheme.typography.titleLarge)
                    Text(
                        "这台 Android · 独立播放，不参与同步分组",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(12.dp))
                    OutlinedButton({ settings = true }) {
                        Icon(Icons.Rounded.Tune, null)
                        Text("输出方式与设备")
                    }
                }
            }
        items(
            zones.sortedBy { if (it.text("endpoint_id") == graph.preferences.endpointId) 0 else 1 },
            key = { it.text("id") },
        ) { z ->
            GlassPanel(Modifier.fillMaxWidth()) {
                if (z.text("endpoint_id") == graph.preferences.endpointId)
                    OutlinedButton({ settings = true }) {
                        Icon(Icons.Rounded.Tune, null)
                        Text("输出方式与设备")
                    }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Speaker, null)
                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                        Text(z.text("name"), style = MaterialTheme.typography.titleMedium)
                        Text(
                            z.child("track").text("title", "未播放"),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    RadioButton(
                        selected == z.text("id"),
                        { graph.runtime.selectedZone.value = z.text("id") },
                    )
                }
                Row {
                    TextButton({
                        graph.runtime.selectedZone.value = z.text("id")
                        a.run {
                            graph.runtime.control(
                                if (z.text("status") == "playing") "pause" else "play"
                            )
                        }
                    }) {
                        Text(if (z.text("status") == "playing") "暂停" else "播放")
                    }
                    TextButton({
                        graph.runtime.selectedZone.value = z.text("id")
                        a.run { graph.runtime.control("next") }
                    }) {
                        Text("下一首")
                    }
                }
                var volume by
                    remember(z.optDouble("volume")) {
                        mutableFloatStateOf(z.optDouble("volume", 1.0).toFloat())
                    }
                Slider(
                    volume,
                    { volume = it },
                    onValueChangeFinished = {
                        a.run {
                            graph.api.value!!.request(
                                "zones/${pathId(z.text("id"))}",
                                "PATCH",
                                json("volume" to volume.toDouble()),
                            )
                        }
                    },
                )
            }
        }
    }
    if (settings)
        ModalBottomSheet(onDismissRequest = { settings = false }) {
            Column(
                Modifier.verticalScroll(rememberScrollState()).padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                PageTitle("Android 输出")
                Choice(
                    "路由",
                    p.deviceId.toString(),
                    listOf("0" to "系统默认输出") +
                        outputs.map { it.id.toString() to "${it.name} · ${it.type}" },
                ) {
                    graph.preferences.save(
                        p.copy(deviceId = it.toInt(), outputMode = OutputMode.AUTO)
                    )
                }
                val device = outputs.firstOrNull { it.id == p.deviceId }
                val options =
                    listOf(
                        OutputMode.AUTO to "自动兼容（Float优先）",
                        OutputMode.TRACK_INTEGER to "AudioTrack 整数",
                        OutputMode.TRACK_FLOAT to "AudioTrack Float",
                    ) +
                        if (device?.bitPerfect == true)
                            listOf(OutputMode.USB_BITPERFECT to "USB 原样输出")
                        else emptyList()
                Choice("输出方式", p.outputMode.name, options.map { it.first.name to it.second }) {
                    graph.preferences.save(p.copy(outputMode = OutputMode.valueOf(it)))
                }
                Text(
                    "${device?.name ?: "系统默认输出"}\n${device?.rates?.joinToString {"${it/1000.0} kHz"}.orEmpty()}",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "设置应用于下一次播放。共享路径的系统混音器格式、蓝牙实际编码可能不公开；不猜测。USB 原样仅向实际报告对应能力的设备开放。",
                    style = MaterialTheme.typography.bodySmall,
                )
                Spacer(Modifier.height(18.dp))
            }
        }
}

@Composable
fun CorePage(graph: AppGraph, a: Actions) {
    val page = rememberPage(graph, "settings")
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(22.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        PageTitle("Core 设置")
        Loading(page)
        if (graph.api.value?.session?.admin == true)
            GlassPanel {
                Section("移动音频")
                SettingSwitch(
                    "允许 Android 外出 Opus",
                    "只允许显式选择的有损省流请求；不修改存储原件",
                    page.value.optBoolean("mobile_opus_enabled"),
                ) { enabled ->
                    a.run {
                        graph.api.value!!.request(
                            "settings",
                            "PATCH",
                            json("mobile_opus_enabled" to enabled),
                        )
                        page.reload()
                    }
                }
            }
        Notice("完整的 Core 管理功能仍可通过 Core WebUI 使用。客户端设置与 Core 设置独立保存。")
    }
}
