@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
)

package com.rmusic.nativeapp.ui

import android.os.SystemClock
import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.*
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import com.rmusic.core.*
import com.rmusic.nativeapp.AppGraph
import kotlinx.coroutines.*
import org.json.JSONObject

@Composable
private fun selected(graph: AppGraph): JSONObject {
    val zones by graph.runtime.zones.collectAsStateWithLifecycle()
    val zone by graph.runtime.selectedZone.collectAsStateWithLifecycle()
    return zones.firstOrNull { it.text("id") == zone } ?: JSONObject()
}

@Composable
fun MiniPlayer(graph: AppGraph, a: Actions, open: () -> Unit) {
    val local by graph.runtime.state.collectAsStateWithLifecycle()
    val zone = selected(graph)
    val track =
        if (zone.text("endpoint_id") == graph.preferences.endpointId)
            local.track ?: zone.optJSONObject("track")?.let(::Track)
        else zone.optJSONObject("track")?.let(::Track)
    if (track == null) return
    val playing = zone.text("status") == "playing"
    Surface(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(22.dp))
            .clickable(onClick = open),
        shape = RoundedCornerShape(22.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .90f),
        border = BorderStroke(1.dp, Color.White.copy(alpha = .07f)),
    ) {
        Row(Modifier.padding(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Artwork(graph, track.cover, Modifier.size(48.dp))
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(
                    track.title,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    fontWeight = FontWeight.Medium,
                )
                Text(
                    if (local.buffering && zone.text("endpoint_id") == graph.preferences.endpointId)
                        "正在加载音源…"
                    else track.artist,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            IconButton({ a.run { graph.runtime.control(if (playing) "pause" else "play") } }) {
                Icon(
                    if (playing) Icons.Rounded.Pause else Icons.Rounded.PlayArrow,
                    if (playing) "暂停" else "播放",
                    Modifier.size(30.dp),
                )
            }
            IconButton({ a.run { graph.runtime.control("next") } }) {
                Icon(Icons.Rounded.SkipNext, "下一首")
            }
        }
    }
}

@Composable
fun PlayerScreen(graph: AppGraph, a: Actions, close: () -> Unit) {
    val zone = selected(graph)
    val local by graph.runtime.state.collectAsStateWithLifecycle()
    val isLocal = zone.text("endpoint_id") == graph.preferences.endpointId
    val track =
        if (isLocal) local.track ?: zone.optJSONObject("track")?.let(::Track)
        else zone.optJSONObject("track")?.let(::Track)
    val background = albumPalette(graph, track?.cover.orEmpty(), MaterialTheme.colorScheme.primary)
    var queue by remember { mutableStateOf(false) }
    var signal by remember { mutableStateOf(false) }
    var lyrics by remember { mutableStateOf(false) }
    var modes by remember { mutableStateOf(false) }
    var translation by remember { mutableStateOf(graph.preferences.translate) }
    BackHandler { close() }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        BoxWithConstraints(
            Modifier.fillMaxSize()
                .background(
                    Brush.radialGradient(
                        listOf(
                            background.copy(alpha = .22f),
                            Color.Transparent,
                        )
                    )
                )
        ) {
            val compact = maxHeight < 560.dp
            Column(
                Modifier.fillMaxSize()
                    .then(if (compact) Modifier.verticalScroll(rememberScrollState()) else Modifier)
                    .systemBarsPadding()
                    .padding(horizontal = 24.dp)
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(close) { Icon(Icons.Rounded.ExpandMore, "收起播放页") }
                    Column(
                        Modifier.weight(1f),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(
                            "NOW PLAYING",
                            style = MaterialTheme.typography.labelSmall,
                            letterSpacing = 2.sp,
                        )
                        Text(
                            zone.text("name", "正在连接设备"),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    IconButton({ queue = true }) {
                        Icon(Icons.AutoMirrored.Rounded.QueueMusic, "播放队列")
                    }
                }
                Box(
                    (if (compact) Modifier.height(230.dp) else Modifier.weight(1f)).fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    if (track != null) {
                        if (lyrics)
                            LyricsView(
                                graph,
                                track,
                                if (isLocal) local.position else zone.optDouble("position"),
                                zone.text("status") == "playing" && !local.buffering,
                                translation,
                            )
                        else
                            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                Artwork(
                                    graph,
                                    track.cover,
                                    Modifier.widthIn(max = 390.dp)
                                        .fillMaxWidth(.92f)
                                        .aspectRatio(1f)
                                        .padding(16.dp)
                                        .shadow(24.dp, RoundedCornerShape(24.dp)),
                                )
                                Spacer(Modifier.height(16.dp))
                                Text(
                                    track.album,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                    }
                }
                if (track != null) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                track.title,
                                style = MaterialTheme.typography.headlineSmall,
                                maxLines = 2,
                                fontWeight = FontWeight.Bold,
                            )
                            ArtistLinks(track, a)
                        }
                        IconButton({ a.favorite(track) }, Modifier.size(56.dp)) {
                            Icon(
                                if (track.id in a.favorites) Icons.Rounded.Favorite
                                else Icons.Rounded.FavoriteBorder,
                                "喜欢",
                                Modifier.size(30.dp),
                                tint =
                                    if (track.id in a.favorites) MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                    var dragging by remember(track.id) { mutableStateOf<Float?>(null) }
                    val position = if (isLocal) local.position else zone.optDouble("position")
                    Slider(
                        value =
                            dragging
                                ?: position
                                    .toFloat()
                                    .coerceIn(0f, track.duration.toFloat().coerceAtLeast(1f)),
                        onValueChange = { dragging = it },
                        onValueChangeFinished = {
                            val value = dragging
                            dragging = null
                            if (value != null)
                                a.run {
                                    graph.runtime.control(
                                        "seek",
                                        json("position" to value.toDouble()),
                                    )
                                }
                        },
                        valueRange = 0f..track.duration.toFloat().coerceAtLeast(1f),
                        thumb = {
                            Box(
                                Modifier.size(14.dp)
                                    .background(MaterialTheme.colorScheme.primary, CircleShape)
                            )
                        },
                        track = { state ->
                            SliderDefaults.Track(
                                sliderState = state,
                                modifier = Modifier.height(4.dp),
                                thumbTrackGapSize = 0.dp,
                                drawStopIndicator = null,
                            )
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(Modifier.fillMaxWidth()) {
                        Text(
                            duration(dragging?.toDouble() ?: position),
                            Modifier.weight(1f),
                            style = MaterialTheme.typography.labelMedium,
                        )
                        Text(duration(track.duration), style = MaterialTheme.typography.labelMedium)
                    }
                    Row(
                        Modifier.fillMaxWidth().padding(vertical = 12.dp),
                        horizontalArrangement = Arrangement.SpaceEvenly,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IconButton(
                            { a.run { graph.runtime.control("previous") } },
                            Modifier.size(56.dp),
                        ) {
                            Icon(Icons.Rounded.SkipPrevious, "上一首", Modifier.size(32.dp))
                        }
                        FilledIconButton(
                            {
                                a.run {
                                    graph.runtime.control(
                                        if (zone.text("status") == "playing") "pause" else "play"
                                    )
                                }
                            },
                            Modifier.size(76.dp),
                            shape = CircleShape,
                        ) {
                            if (isLocal && local.buffering)
                                CircularProgressIndicator(
                                    Modifier.size(30.dp),
                                    color = MaterialTheme.colorScheme.onPrimary,
                                    strokeWidth = 2.dp,
                                )
                            else
                                Icon(
                                    if (zone.text("status") == "playing") Icons.Rounded.Pause
                                    else Icons.Rounded.PlayArrow,
                                    "播放或暂停",
                                    Modifier.size(42.dp),
                                )
                        }
                        IconButton(
                            { a.run { graph.runtime.control("next") } },
                            Modifier.size(56.dp),
                        ) {
                            Icon(Icons.Rounded.SkipNext, "下一首", Modifier.size(32.dp))
                        }
                        Box {
                            IconButton({ modes = true }) {
                                Icon(
                                    if (zone.optBoolean("shuffle")) Icons.Rounded.Shuffle
                                    else if (zone.text("repeat") == "one") Icons.Rounded.RepeatOne
                                    else if (zone.text("repeat") == "all") Icons.Rounded.Repeat
                                    else Icons.AutoMirrored.Rounded.PlaylistPlay,
                                    "播放模式",
                                )
                            }
                            DropdownMenu(modes, { modes = false }) {
                                listOf(
                                        "off" to "顺序播放",
                                        "all" to "列表循环",
                                        "one" to "单曲循环",
                                        "shuffle" to "随机播放",
                                    )
                                    .forEach { (k, n) ->
                                        DropdownMenuItem(
                                            { Text(n) },
                                            {
                                                a.run {
                                                    graph.api.value!!.request(
                                                        "zones/${pathId(zone.text("id"))}",
                                                        "PATCH",
                                                        json(
                                                            "repeat" to
                                                                if (k == "shuffle") "off" else k,
                                                            "shuffle" to (k == "shuffle"),
                                                        ),
                                                    )
                                                }
                                                modes = false
                                            },
                                        )
                                    }
                            }
                        }
                    }
                    Row(
                        Modifier.fillMaxWidth().padding(bottom = 14.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        TextButton({ signal = true }) {
                            Icon(Icons.Rounded.GraphicEq, null)
                            Text(
                                if (isLocal) local.transport.ifBlank { "音频链路" } else "音频链路",
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.widthIn(max = 210.dp),
                            )
                        }
                        IconToggleButton(lyrics, { lyrics = it }) {
                            Icon(Icons.Rounded.Lyrics, "歌词")
                        }
                        if (lyrics)
                            IconToggleButton(
                                translation,
                                {
                                    translation = it
                                    graph.preferences.translate = it
                                },
                            ) {
                                Text("译")
                            }
                    }
                    if (isLocal && local.error.isNotBlank())
                        Text(
                            local.error,
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.padding(bottom = 12.dp),
                        )
                }
            }
        }
    }
    if (queue)
        ModalBottomSheet(onDismissRequest = { queue = false }) {
            val rows = zone.rows("queue")
            LazyColumn(
                Modifier.fillMaxWidth().heightIn(max = 560.dp),
                contentPadding = PaddingValues(18.dp),
            ) {
                item { PageTitle("播放队列", "${rows.size} 首") }
                itemsIndexed(rows, key = { i, v -> "${v.text("id")}:$i" }) { i, v ->
                    TrackRow(
                        graph,
                        Track(v),
                        a,
                        {
                            a.run { graph.runtime.control("play", json("queue_index" to i)) }
                            queue = false
                        },
                    )
                }
            }
        }
    if (signal)
        ModalBottomSheet(onDismissRequest = { signal = false }) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                PageTitle("音频链路")
                if (isLocal) {
                    val t = local.track
                    Chain(
                        "音源",
                        listOf(
                                t?.format?.uppercase(),
                                t?.rate?.takeIf { it > 0 }?.let { "$it Hz" },
                                t?.bits?.takeIf { it > 0 }?.let { "$it bit" },
                            )
                            .filterNotNull()
                            .joinToString(" · "),
                    )
                    Chain(
                        if (local.cached) "本机读取" else "网络传输",
                        local.transport + if (local.cached) " · 本机完整缓存" else "",
                    )
                    Chain("解码", local.decoder.ifBlank { "等待实际解码输出" })
                    val o = local.output
                    Chain(
                        "AudioTrack",
                        listOf(
                                o.text("device"),
                                o.optInt("rate").takeIf { it > 0 }?.let { "$it Hz" },
                                o.optInt("bits").takeIf { it > 0 }?.let { "有效 $it bit" },
                                "欠载计数 ${o.optInt("underruns")}",
                            )
                            .filterNotNull()
                            .joinToString(" · "),
                    )
                    Chain("系统混音器／路由", o.text("mixer_note", "尚未取得实际输出信息"))
                } else {
                    val p = rememberPage(graph, "endpoints")
                    Loading(p)
                    val endpoint =
                        p.value.rows().firstOrNull { it.text("id") == zone.text("endpoint_id") }
                    val chain = endpoint?.child("playback")?.child("signal_path")
                    if (chain != null && chain.length() > 0) {
                        Chain("播放端报告", chain.text("output"))
                        Chain(
                            "输出格式",
                            "${chain.optInt("sample_rate")} Hz · ${chain.optInt("bit_depth")} bit",
                        )
                        chain.optJSONArray("processing")?.let {
                            for (i in 0 until it.length()) Chain("处理", it.optString(i))
                        }
                    } else Notice("播放端尚未报告链路，不推测输出格式")
                }
                Spacer(Modifier.height(16.dp))
            }
        }
}

@Composable
private fun Chain(title: String, text: String) {
    Column {
        Text(
            title,
            color = MaterialTheme.colorScheme.primary,
            style = MaterialTheme.typography.labelLarge,
        )
        Spacer(Modifier.height(6.dp))
        Text(text.ifBlank { "尚未报告" })
        HorizontalDivider(Modifier.padding(top = 14.dp), color = MaterialTheme.colorScheme.outline)
    }
}

@Composable
private fun LyricsView(
    graph: AppGraph,
    t: Track,
    position: Double,
    playing: Boolean,
    translation: Boolean,
) {
    val data = rememberPage(graph, "library/tracks/${pathId(t.id)}/lyrics")
    val lines =
        remember(data.value) {
            Lyrics.parse(
                data.value.text(
                    "text",
                    data.value.text("lyrics", data.value.text("lyric", data.value.text("lrc"))),
                ),
                data.value.text("translation", data.value.text("trans")),
            )
        }
    val scroll = rememberLazyListState()
    val anchor = with(androidx.compose.ui.platform.LocalDensity.current) { 120.dp.roundToPx() }
    var clock by remember(t.id) { mutableDoubleStateOf(position) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(position, playing, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            val at = SystemClock.elapsedRealtime()
            while (isActive) {
                clock =
                    position + if (playing) (SystemClock.elapsedRealtime() - at) / 1000.0 else 0.0
                if (!playing) break
                delay(50)
            }
        }
    }
    val index =
        lines
            .indexOfLast {
                it.millis <=
                    clock * 1000 + graph.preferences.lyricOffset + data.value.optInt("offset_ms")
            }
            .coerceAtLeast(0)
    // Stable timestamp keys, no view replacement or text reflow when the active line changes.
    LaunchedEffect(index) {
        if (lines.isNotEmpty()) scroll.animateScrollToItem(index, scrollOffset = -anchor)
    }
    if (lines.isEmpty())
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Loading(data)
            val plain = data.value.text("text")
            if (plain.isNotBlank())
                Text(
                    plain,
                    Modifier.fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(vertical = 36.dp),
                    style = MaterialTheme.typography.headlineSmall,
                )
            else Notice(if (data.busy) "加载歌词…" else "这首歌暂无歌词")
        }
    else
        LazyColumn(
            Modifier.fillMaxSize()
                .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                .drawWithContent {
                    drawContent()
                    drawRect(
                        Brush.verticalGradient(
                            0f to Color.Transparent,
                            .10f to Color.Black,
                            .82f to Color.Black,
                            1f to Color.Transparent,
                        ),
                        blendMode = BlendMode.DstIn,
                    )
                },
            state = scroll,
            contentPadding = PaddingValues(top = 140.dp, bottom = 240.dp),
            verticalArrangement = Arrangement.spacedBy(28.dp),
        ) {
            itemsIndexed(lines, key = { i, l -> "${l.millis}:$i" }) { i, l ->
                val alpha by
                    animateFloatAsState(
                        if (i == index) 1f else .38f,
                        tween(280),
                        label = "lyric emphasis",
                    )
                Column(Modifier.fillMaxWidth().alpha(alpha)) {
                    Text(
                        l.text,
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.SemiBold,
                        color =
                            if (i == index) MaterialTheme.colorScheme.primary
                            else MaterialTheme.colorScheme.onSurface,
                    )
                    if (translation && l.translation.isNotBlank()) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            l.translation,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyLarge,
                        )
                    }
                }
            }
        }
}
