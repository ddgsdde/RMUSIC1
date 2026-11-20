package com.rmusic.core

import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import okhttp3.*
import org.json.JSONObject

/**
 * JSON control only. Audio never rides this socket; group/scheduled output is deliberately absent.
 */
class EndpointSocket(
    private val api: CoreApi,
    private val endpoint: JSONObject,
    private val onCommand: suspend (JSONObject) -> Unit,
    private val eventsOnly: Boolean = false,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val client =
        api.http
            .newBuilder()
            .callTimeout(0, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS)
            .build()
    private var socket: WebSocket? = null
    private var reconnect: Job? = null
    private var generation = 0L
    private var retry = 0
    private val mutable = MutableStateFlow(false)
    val online = mutable.asStateFlow()

    fun connect() {
        reconnect?.cancel()
        val epoch = ++generation
        val target =
            api.url(if (eventsOnly) "api/v1/events" else "api/v1/bridge/events")
                .replaceFirst("https://", "wss://")
                .replaceFirst("http://", "ws://")
        socket =
            client.newWebSocket(
                Request.Builder()
                    .url(target)
                    .header("Authorization", "Bearer ${api.session.token}")
                    .build(),
                object : WebSocketListener() {
                    override fun onOpen(ws: WebSocket, response: Response) {
                        if (epoch != generation) {
                            ws.close(1000, "superseded")
                            return
                        }
                        retry = 0
                        mutable.value = eventsOnly
                        if (!eventsOnly)
                            ws.send(json("type" to "register", "endpoint" to endpoint).toString())
                    }

                    override fun onMessage(ws: WebSocket, text: String) {
                        if (
                            epoch != generation ||
                                text.length > (if (eventsOnly) 8 * 1024 * 1024 else 131072)
                        )
                            return
                        val value = runCatching { JSONObject(text) }.getOrNull() ?: return
                        if (value.text("type") == "registered") mutable.value = true
                        scope.launch { if (epoch == generation) onCommand(value) }
                    }

                    override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                        lost(epoch, response?.code == 401)
                    }

                    override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                        lost(epoch, false)
                    }
                },
            )
    }

    private fun lost(epoch: Long, unauthorized: Boolean) {
        if (epoch != generation) return
        mutable.value = false
        if (unauthorized) {
            scope.launch { onCommand(json("type" to "session_expired")) }
            return
        }
        reconnect?.cancel()
        reconnect = scope.launch {
            delay((1000L shl retry.coerceAtMost(5)).coerceAtMost(30000))
            retry++
            connect()
        }
    }

    fun reconnectIfNeeded() {
        if (!online.value) {
            socket?.cancel()
            connect()
        }
    }

    fun rename(name: String) {
        endpoint.put("name", name)
        if (!eventsOnly && online.value) send(json("type" to "register", "endpoint" to endpoint))
    }

    fun send(value: JSONObject): Boolean = socket?.send(value.toString()) == true

    fun close() {
        ++generation
        reconnect?.cancel()
        socket?.close(1000, "client exit")
        socket = null
        mutable.value = false
        scope.cancel()
    }
}
