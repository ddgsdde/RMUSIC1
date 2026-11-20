package com.rmusic.media

import android.content.Context
import android.net.*
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.distinctUntilChanged

class Connectivity(context: Context) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)

    fun unmeteredWifi(): Boolean =
        manager.getNetworkCapabilities(manager.activeNetwork)?.let {
            it.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
                it.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        } == true

    fun online() =
        manager
            .getNetworkCapabilities(manager.activeNetwork)
            ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true

    data class State(val network: String, val available: Boolean, val unmeteredWifi: Boolean)

    private fun snapshot() =
        State(manager.activeNetwork?.toString().orEmpty(), online(), unmeteredWifi())

    fun changes() = callbackFlow {
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    trySend(snapshot())
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    capabilities: NetworkCapabilities,
                ) {
                    trySend(snapshot())
                }

                override fun onLost(network: Network) {
                    trySend(snapshot())
                }
            }
        manager.registerDefaultNetworkCallback(callback)
        trySend(snapshot())
        awaitClose { manager.unregisterNetworkCallback(callback) }
    }
        .distinctUntilChanged()
}
