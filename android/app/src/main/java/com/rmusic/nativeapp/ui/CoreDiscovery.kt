package com.rmusic.nativeapp.ui

import android.net.nsd.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.*

/** Discovery exists only on the login page and is stopped on disposal. */
@Composable
fun CoreDiscovery(select: (String) -> Unit) {
    val context = LocalContext.current
    val found = remember { mutableStateMapOf<String, String>() }
    DisposableEffect(context) {
        val manager = context.getSystemService(NsdManager::class.java)
        val main = android.os.Handler(android.os.Looper.getMainLooper())
        var active = true
        val listener =
            object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(type: String) {}

                override fun onServiceFound(service: NsdServiceInfo) {
                    if (!active) return
                    manager.resolveService(
                        service,
                        object : NsdManager.ResolveListener {
                            override fun onResolveFailed(info: NsdServiceInfo, code: Int) {}

                            override fun onServiceResolved(info: NsdServiceInfo) {
                                val host = info.host?.hostAddress ?: return
                                val txt = info.attributes
                                val scheme =
                                    if (txt["tls"]?.toString(Charsets.UTF_8) == "1") "https"
                                    else "http"
                                val prefix =
                                    txt["path"]?.toString(Charsets.UTF_8).orEmpty().trimEnd('/')
                                if (
                                    prefix.isNotEmpty() &&
                                        (!prefix.startsWith('/') ||
                                            prefix.contains("..") ||
                                            prefix.any { it == '?' || it == '#' || it == '\\' })
                                )
                                    return
                                val url =
                                    "$scheme://${if(':' in host)"[$host]" else host}:${info.port}$prefix"
                                main.post { if (active) found[info.serviceName] = url }
                            }
                        },
                    )
                }

                override fun onServiceLost(info: NsdServiceInfo) {
                    main.post { found.remove(info.serviceName) }
                }

                override fun onDiscoveryStopped(type: String) {}

                override fun onStartDiscoveryFailed(type: String, code: Int) {}

                override fun onStopDiscoveryFailed(type: String, code: Int) {}
            }
        runCatching {
            manager.discoverServices("_rmusic-core._tcp.", NsdManager.PROTOCOL_DNS_SD, listener)
        }
        onDispose {
            active = false
            runCatching { manager.stopServiceDiscovery(listener) }
        }
    }
    found.forEach { (name, url) -> TextButton({ select(url) }) { Text("$name · $url") } }
}
