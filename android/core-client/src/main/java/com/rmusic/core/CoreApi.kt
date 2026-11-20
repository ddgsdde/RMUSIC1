package com.rmusic.core

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import okhttp3.*
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

class CoreFailure(val status: Int, val code: String, message: String) : IOException(message)

class CoreApi(val session: CoreSession, val http: OkHttpClient = client()) {
    val base = session.address.trimEnd('/').toHttpUrl()

    fun url(path: String): String =
        if (path.startsWith("http")) {
            val target = path.toHttpUrl()
            require(
                target.host == base.host && target.port == base.port && target.scheme == base.scheme
            ) {
                "拒绝跨Core地址发送凭据"
            }
            target.toString()
        } else "${session.address.trimEnd('/')}/${path.trimStart('/')}"

    suspend fun request(
        path: String,
        method: String = "GET",
        body: JSONObject? = null,
    ): JSONObject =
        withContext(Dispatchers.IO) {
            val req = Request.Builder().url(url("api/v1/${path.trimStart('/')}"))
            // Identity preflight is public: do not disclose the saved token to a replaced Core.
            if (session.token.isNotEmpty() && path.substringBefore('?') != "status")
                req.header("Authorization", "Bearer ${session.token}")
            if (method != "GET")
                req.method(
                    method,
                    (body?.toString() ?: "{}").toRequestBody("application/json".toMediaType()),
                )
            http.newCall(req.build()).await().use { response ->
                if ((response.body?.contentLength() ?: 0) > 8 * 1024 * 1024)
                    throw IOException("Core响应过大，请缩小分页")
                val source = response.body?.source()
                source?.request(8 * 1024 * 1024 + 1L)
                val bytes = source?.buffer?.readByteArray() ?: byteArrayOf()
                if (bytes.size > 8 * 1024 * 1024) throw IOException("Core响应过大，请缩小分页")
                val text = bytes.toString(Charsets.UTF_8)
                val value = runCatching {
                    if (text.trimStart().startsWith("[")) json("items" to JSONArray(text))
                    else JSONObject(text)
                }
                    .getOrElse { JSONObject() }
                if (!response.isSuccessful)
                    throw CoreFailure(
                        response.code,
                        value.child("error").text("code"),
                        value.child("error").text("message", "Core请求失败（${response.code}）"),
                    )
                value
            }
        }

    suspend fun textFile(path: String, maximum: Int = 2 * 1024 * 1024): String =
        withContext(Dispatchers.IO) {
            val request =
                Request.Builder()
                    .url(url("api/v1/$path"))
                    .header("Authorization", "Bearer ${session.token}")
                    .build()
            http.newCall(request).await().use { r ->
                if (!r.isSuccessful) throw IOException("文件读取失败（${r.code}）")
                val source = r.body?.source() ?: throw IOException("Core未返回文件")
                source.request(maximum + 1L)
                val bytes = source.buffer.readByteArray()
                require(bytes.size <= maximum) { "文件超过大小上限" }
                bytes.toString(Charsets.UTF_8)
            }
        }

    suspend fun mediaUrl(track: Track): String {
        val path = request("library/tracks/${pathId(track.id)}/ticket").text("api_path")
        require(path.startsWith("api/v1/media/") && path.contains("ticket=")) { "Core未返回媒体授权" }
        return url(path)
    }

    suspend fun command(zone: String, action: String, body: JSONObject = JSONObject()) =
        request("zones/${pathId(zone)}/$action", "POST", body)

    suspend fun queue(zone: String, tracks: List<Track>, index: Int = 0): JSONObject {
        require(tracks.size in 1..10000 && index in tracks.indices)
        val queued =
            request(
                "zones/${pathId(zone)}/queue",
                "PUT",
                json("track_ids" to JSONArray(tracks.map { it.id }), "queue_index" to index),
            )
        if (queued.text("status") == "playing" && queued.text("playback_id").isNotBlank())
            return queued
        return command(zone, "play", json("queue_index" to index))
    }

    fun image(path: String): String? =
        when {
            path.isBlank() -> null
            path.startsWith("/api/") -> url(path)
            base.encodedPath.trimEnd('/').isNotEmpty() &&
                path.startsWith(base.encodedPath.trimEnd('/') + "/api/") ->
                base
                    .newBuilder()
                    .encodedPath(path.substringBefore('?'))
                    .encodedQuery(path.substringAfter('?', "").ifEmpty { null })
                    .build()
                    .toString()
            path.startsWith("https://") -> path
            else -> null
        }

    fun close() {
        http.dispatcher.cancelAll()
        http.connectionPool.evictAll()
    }

    companion object {
        fun client() =
            OkHttpClient.Builder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)
                .callTimeout(45, TimeUnit.SECONDS)
                .followRedirects(false)
                .build()

        suspend fun login(
            address: String,
            username: String,
            password: String,
            totp: String = "",
        ): Pair<CoreApi, JSONObject> {
            val normalized = address.trim().trimEnd('/').removeSuffix("/api/v1")
            val uri = normalized.toHttpUrl()
            require(uri.scheme == "https" || isPrivateHost(uri.host)) { "外部Core需要HTTPS地址" }
            val anonymous = CoreApi(CoreSession(normalized, "", "", "", username, false))
            val status =
                try {
                    anonymous.request("status")
                } finally {
                    anonymous.close()
                }
            require(status.optBoolean("initialized")) { "请先在Core网页完成初始化" }
            val loginClient = CoreApi(anonymous.session)
            val value =
                try {
                    loginClient.request(
                        "auth/login",
                        "POST",
                        json("username" to username.trim(), "password" to password, "totp" to totp),
                    )
                } finally {
                    loginClient.close()
                }
            val authenticated =
                CoreApi(
                    CoreSession(
                        normalized,
                        value.text("token"),
                        status.text("core_id"),
                        "",
                        username,
                        false,
                    )
                )
            val user =
                try {
                    authenticated.request("auth/me")
                } finally {
                    authenticated.close()
                }
            val session =
                CoreSession(
                    normalized,
                    value.text("token"),
                    status.text("core_id"),
                    user.text("id"),
                    user.text("username", username),
                    user.text("role") == "admin",
                )
            require(
                session.token.isNotEmpty() &&
                    session.coreId.isNotEmpty() &&
                    session.userId.isNotEmpty()
            ) {
                "Core登录响应不完整"
            }
            return CoreApi(session) to status
        }

        fun isPrivateHost(host: String): Boolean {
            val parts = host.split('.')
            val numbers = parts.mapNotNull(String::toIntOrNull)
            val ipv4 = parts.size == 4 && numbers.size == 4 && numbers.all { it in 0..255 }
            return host == "localhost" ||
                host == "::1" ||
                host.endsWith(".local") ||
                host.contains(':') &&
                    (host.startsWith("fd", true) || host.startsWith("fc", true)) ||
                ipv4 &&
                    (numbers[0] in setOf(10, 127) ||
                        numbers[0] == 192 && numbers[1] == 168 ||
                        numbers[0] == 172 && numbers[1] in 16..31)
        }
    }
}

suspend fun Call.await(): Response = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel() }
    enqueue(
        object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (continuation.isActive) continuation.resumeWithException(e)
            }

            override fun onResponse(call: Call, response: Response) {
                if (continuation.isActive)
                    continuation.resume(response) { _, value, _ -> value.close() }
                else response.close()
            }
        }
    )
}
