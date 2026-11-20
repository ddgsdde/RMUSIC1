package com.rmusic.media

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

class LocalPreferences(context: Context) {
    private val disk = context.getSharedPreferences("native-settings", Context.MODE_PRIVATE)
    private val mutable = MutableStateFlow(read())
    val policy = mutable.asStateFlow()
    var musicBytes: Long
        get() = disk.getLong("musicBytes", 512L * 1024 * 1024)
        set(v) {
            require(v in 16L * 1024 * 1024..32L * 1024 * 1024 * 1024)
            disk.edit().putLong("musicBytes", v).apply()
        }

    var imageBytes: Long
        get() = disk.getLong("imageBytes", 96L * 1024 * 1024)
        set(v) {
            require(v in 8L * 1024 * 1024..1024L * 1024 * 1024)
            disk.edit().putLong("imageBytes", v).apply()
        }

    var theme: String
        get() = disk.getString("theme", "emerald")!!
        set(v) {
            disk.edit().putString("theme", v).apply()
        }

    var rawPcmCompatibility: Boolean
        get() = disk.getBoolean("rawPcm-" + android.os.Build.FINGERPRINT, false)
        set(value) {
            disk.edit().putBoolean("rawPcm-" + android.os.Build.FINGERPRINT, value).apply()
        }

    var resumeDownloads: Boolean
        get() = disk.getBoolean("resumeDownloads", true)
        set(value) {
            disk.edit().putBoolean("resumeDownloads", value).apply()
        }

    var backgroundControl: Boolean
        get() = disk.getBoolean("backgroundControl", true)
        set(value) {
            disk.edit().putBoolean("backgroundControl", value).apply()
        }

    var translate: Boolean
        get() = disk.getBoolean("translation", true)
        set(v) {
            disk.edit().putBoolean("translation", v).apply()
        }

    var lyricOffset: Int
        get() = disk.getInt("lyricOffset", 0)
        set(v) {
            disk.edit().putInt("lyricOffset", v.coerceIn(-5000, 5000)).apply()
        }

    var clientName: String
        get() = disk.getString("clientName", android.os.Build.MODEL)!!
        set(v) {
            require(v.isNotBlank() && v.length <= 80)
            disk.edit().putString("clientName", v).apply()
        }

    val endpointId: String
        get() =
            disk.getString("endpointId", null)
                ?: ("android-native-" + java.util.UUID.randomUUID()).also {
                    disk.edit().putString("endpointId", it).commit()
                }

    fun save(value: ListeningPolicy) {
        require(value.bufferSeconds in 5..60)
        disk
            .edit()
            .putString(
                "policy",
                JSONObject()
                    .put("opus", value.mobileOpus)
                    .put("kbps", value.opusKbps)
                    .put("mobileRate", value.mobileMaxRate)
                    .put("mobileBits", value.mobileMaxBits)
                    .put("wifiRate", value.wifiMaxRate)
                    .put("wifiBits", value.wifiMaxBits)
                    .put("buffer", value.bufferSeconds)
                    .put("output", value.outputMode.name)
                    .put("device", value.deviceId)
                    .put("network", value.networkMode)
                    .toString(),
            )
            .apply()
        mutable.value = value
    }

    private fun read() = runCatching {
        val v = JSONObject(disk.getString("policy", "{}")!!)
        ListeningPolicy(
            v.optBoolean("opus"),
            v.optInt("kbps", 160),
            v.optInt("mobileRate", 96000),
            v.optInt("mobileBits", 24),
            v.optInt("wifiRate", 192000),
            v.optInt("wifiBits", 24),
            v.optInt("buffer", 15),
            OutputMode.valueOf(v.optString("output", "AUTO")),
            v.optInt("device"),
            v.optString("network", "auto"),
        )
    }
        .getOrDefault(ListeningPolicy())
}
