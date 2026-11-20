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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*
import org.json.JSONObject

@Composable
fun StatsPage(graph: AppGraph, a: Actions) {
    val offline by graph.history.status.collectAsStateWithLifecycle()
    val p =
        rememberPage(
            graph,
            "stats?period_days=30&timezone=${pathId(java.util.TimeZone.getDefault().id)}",
        )
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(22.dp)) {
        item {
            PageTitle("聆听统计", "最近 30 天")
            Loading(p)
            GlassPanel {
                val s = p.value.child("summary")
                Text(
                    "${(s.optDouble("listened_seconds",s.optDouble("seconds"))/60).toInt()} 分钟",
                    style = MaterialTheme.typography.headlineLarge,
                )
                Text("实际聆听时长")
            }
            if (offline.pending > 0 || offline.rejected > 0 || offline.error.isNotBlank()) {
                GlassPanel {
                    Text("离线聆听记录", style = MaterialTheme.typography.titleMedium)
                    Text(if (offline.syncing) "正在补传…" else "待补传 ${offline.pending} 条")
                    if (offline.rejected > 0) Notice("${offline.rejected} 条记录需要检查，已保留原始记录")
                    if (offline.error.isNotBlank()) Notice(offline.error)
                    TextButton(
                        {
                            a.run {
                                graph.history.sync()
                                p.reload()
                            }
                        },
                        enabled = !offline.syncing,
                    ) {
                        Text("立即补传")
                    }
                }
            }
            Section("常听曲目")
        }
        items(p.value.rows("tracks")) { v ->
            TrackRow(
                graph,
                Track(JSONObject(v.toString()).put("id", v.text("track_id", v.text("id")))),
                a,
                {
                    a.run {
                        val t =
                            graph.api.value!!.request(
                                "library/tracks/${pathId(v.text("track_id",v.text("id")))}"
                            )
                        a.play(listOf(t))
                    }
                },
            )
        }
    }
}
