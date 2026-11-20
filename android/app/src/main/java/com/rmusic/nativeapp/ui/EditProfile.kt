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

@Composable
fun EditProfile(
    graph: AppGraph,
    a: Actions,
    p: Page,
    profile: JSONObject,
    revision: Long,
    close: () -> Unit,
    saved: () -> Unit,
) {
    val keys =
        listOf(
            "description" to "简介",
            "image_url" to "图片地址",
            "genre" to "风格",
            "release_date" to "发行日期",
            "country" to "地区",
            "label" to "唱片公司",
        )
    val original = remember { JSONObject(profile.toString()) }
    val expectedRevision = remember { revision }
    var error by remember { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }
    var fields by remember {
        mutableStateOf(keys.associate { it.first to original.text(it.first) })
    }
    ModalBottomSheet(onDismissRequest = close) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            PageTitle("编辑资料")
            keys.forEach { (key, label) ->
                OutlinedTextField(
                    fields[key].orEmpty(),
                    { fields = fields + (key to it) },
                    label = { Text(label) },
                    modifier = Modifier.fillMaxWidth(),
                    minLines = if (key == "description") 3 else 1,
                )
            }
            if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error)
            if (saving) LinearProgressIndicator(Modifier.fillMaxWidth())
            Button(
                {
                    a.run {
                        saving = true
                        error = ""
                        try {
                            val changed = JSONObject()
                            fields
                                .filter { (k, v) -> v != original.text(k) }
                                .forEach { (k, v) -> changed.put(k, v) }
                            if (changed.length() > 0)
                                graph.api.value!!.request(
                                    "library/catalog/${p.kind}/${pathId(p.id)}/profile",
                                    "PATCH",
                                    json(
                                        "expected_revision" to expectedRevision,
                                        "fields" to changed,
                                    ),
                                )
                            saved()
                            close()
                        } catch (e: Exception) {
                            if (e is CancellationException) throw e
                            error = e.message ?: "保存失败，请重新加载资料后再试"
                        } finally {
                            saving = false
                        }
                    }
                },
                Modifier.fillMaxWidth(),
                enabled = !saving,
            ) {
                Text("保存修改")
            }
            Text(
                "人工修改由 Core 保存，刷新不会覆盖；资料已发生变化时会要求重新确认。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(20.dp))
        }
    }
}
