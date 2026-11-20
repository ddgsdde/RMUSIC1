package com.rmusic.android.development

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper
import org.json.JSONArray
import org.json.JSONObject
import java.net.Inet4Address

/** Let Android's NSD daemon own multicast sockets and interface changes. No subnet scan. */
@Suppress("DEPRECATION")
class CoreDiscovery(context: Context) {
    private val manager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val wifi = context.getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val main = Handler(Looper.getMainLooper())
    private val records = mutableMapOf<String, JSONObject>()
    private val found = mutableSetOf<String>()
    private val queue = java.util.ArrayDeque<NsdServiceInfo>()
    private val callbacks = mutableMapOf<String, NsdManager.ServiceInfoCallback>()
    private var generation = 0
    private var resolving = false
    private var listener: NsdManager.DiscoveryListener? = null
    private var error: String? = null
    private var resolvedCount = 0
    private var lastAddresses = ""
    private var multicast: WifiManager.MulticastLock? = null
    private val stop = Runnable {
        generation++
        listener?.let { runCatching { manager.stopServiceDiscovery(it) } }
        listener = null
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            callbacks.values.toList().forEach { runCatching { manager.unregisterServiceInfoCallback(it) } }
            callbacks.clear()
        }
        multicast?.let { if (it.isHeld) it.release() }; multicast = null
        synchronized(this) { records.clear(); found.clear(); queue.clear(); resolving = false }
    }
    @Synchronized fun snapshot(): String {
        main.post {
            main.removeCallbacks(stop); main.postDelayed(stop, 15000)
            if (listener == null) start()
        }
        return JSONObject().put("available", error == null).put("error", error)
            .put("items", JSONArray(records.values.toList())).apply { if (BuildConfig.DEBUG) put("diagnostics", JSONObject().put("resolved",resolvedCount).put("last_addresses",lastAddresses)) }.toString()
    }
    private fun start() {
        // Required on older Android NSD implementations; bounded to the visible search session.
        runCatching { multicast = wifi.createMulticastLock("rmusic-core-discovery").apply { setReferenceCounted(false); acquire() } }
        val next = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(type: String) { synchronized(this@CoreDiscovery) { error = null } }
            override fun onServiceFound(info: NsdServiceInfo) { synchronized(this@CoreDiscovery) {
                if (!info.serviceType.startsWith("_rmusic-core._tcp") || found.size >= 64) return
                if (!found.add(info.serviceName)) return
                if (android.os.Build.VERSION.SDK_INT >= 34) watch(info)
                else { queue.add(info); resolveNext() }
            } }
            override fun onServiceLost(info: NsdServiceInfo) { synchronized(this@CoreDiscovery) {
                found.remove(info.serviceName); records.remove(info.serviceName)
                if (android.os.Build.VERSION.SDK_INT >= 34) callbacks.remove(info.serviceName)?.let { runCatching { manager.unregisterServiceInfoCallback(it) } }
            } }
            override fun onDiscoveryStopped(type: String) {}
            override fun onStartDiscoveryFailed(type: String, code: Int) { synchronized(this@CoreDiscovery) { error = "局域网发现暂不可用（$code），可手动填写核心地址" }; main.post(stop) }
            override fun onStopDiscoveryFailed(type: String, code: Int) {}
        }
        listener = next
        try { manager.discoverServices("_rmusic-core._tcp.", NsdManager.PROTOCOL_DNS_SD, next) }
        catch (failure: Exception) { synchronized(this) { error = failure.message }; stop.run() }
    }
    @android.annotation.TargetApi(34)
    private fun watch(info: NsdServiceInfo) {
        val epoch = generation
        val callback = object : NsdManager.ServiceInfoCallback {
            override fun onServiceInfoCallbackRegistrationFailed(code: Int) { synchronized(this@CoreDiscovery) { callbacks.remove(info.serviceName) } }
            override fun onServiceInfoCallbackUnregistered() {}
            override fun onServiceLost() { synchronized(this@CoreDiscovery) { if (epoch == generation) records.remove(info.serviceName) } }
            override fun onServiceUpdated(service: NsdServiceInfo) { synchronized(this@CoreDiscovery) {
                if (epoch != generation || !found.contains(info.serviceName)) return
                resolvedCount++
                parse(service)?.let { records[info.serviceName] = it }
            } }
        }
        callbacks[info.serviceName] = callback
        // One-shot resolution may finish before the IPv4 response arrives on dual-stack NAS hosts.
        try { manager.registerServiceInfoCallback(info, { task -> main.post(task) }, callback) }
        catch (_: Exception) { callbacks.remove(info.serviceName) }
    }
    @Synchronized private fun resolveNext() {
        if (resolving || queue.isEmpty() || listener == null) return
        val info = queue.removeFirst(); resolving = true; val epoch = generation
        try { manager.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(service: NsdServiceInfo, code: Int) { synchronized(this@CoreDiscovery) { if (epoch == generation) { resolving = false; resolveNext() } } }
            override fun onServiceResolved(service: NsdServiceInfo) { synchronized(this@CoreDiscovery) {
                if (epoch != generation) return
                resolvedCount++
                if (found.contains(service.serviceName)) parse(service)?.let { records[service.serviceName] = it }
                resolving = false; resolveNext()
            } }
        }) } catch (_: Exception) { resolving = false; resolveNext() }
    }
    private fun parse(info: NsdServiceInfo): JSONObject? {
        fun text(key: String) = info.attributes[key]?.toString(Charsets.UTF_8).orEmpty()
        val id = text("id"); val path = text("path")
        val hosts = if (android.os.Build.VERSION.SDK_INT >= 34) info.hostAddresses else listOfNotNull(info.host)
        lastAddresses = hosts.joinToString { it.javaClass.simpleName }
        if (text("v") != "1" || id.isBlank() || id.length > 200 || id.any { it.isISOControl() } || info.port !in 1..65535) return null
        val lower = path.lowercase()
        if (path.length > 512 || path.isNotEmpty() && !path.startsWith('/') || path.startsWith("//") || path.contains("..") || path.any { it.isISOControl() || it in "?#\\" } || listOf("%2e", "%2f", "%5c").any { lower.contains(it) }) return null
        val addresses = hosts.filterIsInstance<Inet4Address>().filter { !it.isAnyLocalAddress && !it.isLoopbackAddress && !it.isMulticastAddress }.distinct()
        if (addresses.isEmpty()) return null
        val scheme = if (text("tls") == "1") "https" else "http"
        return JSONObject().put("id", id).put("name", text("name").take(200).ifBlank { "R-Music" })
            .put("kind", "core").put("core_id", text("core_id").ifBlank { id })
            .put("addresses", JSONArray(addresses.map { "$scheme://${it.hostAddress}:${info.port}$path" }))
    }
}
