package com.rmusic.media

import com.rmusic.core.*
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.json.JSONObject

enum class OutputMode {
    AUTO,
    TRACK_INTEGER,
    TRACK_FLOAT,
    USB_BITPERFECT,
    OFFLOAD,
}

data class PcmFormat(val rate: Int, val bits: Int, val channels: Int)

data class DeviceCaps(
    val maxRate: Int = 48000,
    val maxBits: Int = 24,
    val rates: List<Int> = emptyList(),
    val usb: Boolean = false,
    val bitPerfect: Boolean = false,
    val formats: List<PcmFormat> = emptyList(),
)

data class ListeningPolicy(
    val mobileOpus: Boolean = false,
    val opusKbps: Int = 160,
    val mobileMaxRate: Int = 96000,
    val mobileMaxBits: Int = 24,
    val wifiMaxRate: Int = 192000,
    val wifiMaxBits: Int = 24,
    val bufferSeconds: Int = 15,
    val outputMode: OutputMode = OutputMode.AUTO,
    val deviceId: Int = 0,
    val networkMode: String = "auto",
)

enum class WireCodec {
    ORIGINAL,
    FLAC,
    OPUS,
    PCM,
}

data class StreamPlan(
    val codec: WireCodec,
    val rate: Int,
    val bits: Int,
    val channels: Int,
    val bitrate: Int = 0,
    val sourceRevision: String = "",
    val lossless: Boolean = false,
) {
    val quality
        get() = if (lossless) 2 else if (codec == WireCodec.OPUS) 0 else 1

    fun cacheKey(namespace: String, track: String, start: Double = 0.0) =
        stableHash("$namespace\u0000$track") +
            ":" +
            stableHash(
                "$sourceRevision:$codec:$rate:$bits:$channels:$bitrate:${(start*48000).toLong()}"
            )

    fun json() =
        json(
            "codec" to codec.name,
            "rate" to rate,
            "bits" to bits,
            "channels" to channels,
            "bitrate" to bitrate,
            "source_revision" to sourceRevision,
            "lossless" to lossless,
        )

    fun uri(original: String, start: Double = 0.0): String {
        if (codec == WireCodec.ORIGINAL) return original
        val parsed = original.toHttpUrl()
        val path =
            parsed.encodedPath.replace(
                "/media/",
                when (codec) {
                    WireCodec.OPUS -> "/mobile-opus/"
                    WireCodec.PCM -> "/native-pcm/"
                    else -> "/native-flac/"
                },
            )
        return parsed
            .newBuilder()
            .encodedPath(path)
            .addQueryParameter("start", start.toString())
            .apply {
                addQueryParameter("channels", channels.toString())
                if (codec == WireCodec.OPUS) {
                    addQueryParameter("accept_lossy", "true")
                    addQueryParameter("bitrate", bitrate.toString())
                } else {
                    addQueryParameter("rate", rate.toString())
                    addQueryParameter("bits", bits.toString())
                }
            }
            .build()
            .toString()
    }

    companion object {
        fun from(v: JSONObject) =
            StreamPlan(
                WireCodec.valueOf(v.getString("codec")),
                v.getInt("rate"),
                v.getInt("bits"),
                v.getInt("channels"),
                v.optInt("bitrate"),
                v.text("source_revision"),
                v.optBoolean("lossless"),
            )
    }
}

object PlaybackPolicy {
    fun choose(
        track: Track,
        networkWifi: Boolean,
        caps: DeviceCaps,
        p: ListeningPolicy,
    ): StreamPlan {
        val wifi =
            when (p.networkMode) {
                "wifi" -> true
                "mobile" -> false
                else -> networkWifi
            }
        val maximumRate =
            minOf(caps.maxRate, if (wifi) p.wifiMaxRate else p.mobileMaxRate).coerceIn(8000, 768000)
        val maximumBits =
            minOf(caps.maxBits, if (wifi) p.wifiMaxBits else p.mobileMaxBits).coerceIn(16, 32)
        val rate =
            if (track.dsd)
                listOf(705600, 352800, 176400, 88200, 44100).firstOrNull { it <= maximumRate }
                    ?: minOf(44100, maximumRate)
            else track.rate.takeIf { it in 8000..768000 } ?: minOf(48000, maximumRate)
        val targetRate =
            if (rate <= maximumRate) rate
            else
                listOf(
                        768000,
                        705600,
                        384000,
                        352800,
                        192000,
                        176400,
                        96000,
                        88200,
                        48000,
                        44100,
                        32000,
                        24000,
                        22050,
                        16000,
                        8000,
                    )
                    .firstOrNull { it <= maximumRate && rate % it == 0 }
                    ?: listOf(192000, 96000, 48000, 44100, 32000, 24000, 16000, 8000).first {
                        it <= maximumRate
                    }
        val targetBits =
            listOf(32, 24, 16).first {
                it <= maximumBits &&
                    (track.dsd ||
                        track.bits <= 0 ||
                        it <= (if (track.bits <= 16) 16 else if (track.bits <= 24) 24 else 32))
            }
        val channels = track.channels.coerceIn(1, 2)
        val opus =
            !wifi &&
                p.mobileOpus &&
                maximumRate >= 48000 &&
                p.outputMode != OutputMode.USB_BITPERFECT
        if (
            opus &&
                (track.lossless || track.raw.optInt("bitrate", Int.MAX_VALUE) > p.opusKbps * 1000)
        )
            return StreamPlan(
                WireCodec.OPUS,
                48000,
                0,
                channels,
                p.opusKbps.coerceIn(48, 320),
                track.revision,
                false,
            )
        // DSD always converts in Core. Unknown original precision is probed by Core's FLAC path.
        if (p.outputMode == OutputMode.USB_BITPERFECT) {
            require(caps.usb && caps.bitPerfect) { "所选设备不支持Android原样USB输出" }
            if (caps.formats.isNotEmpty()) {
                val candidates =
                    caps.formats
                        .filter { it.rate <= maximumRate && it.rate <= rate && it.channels in 1..2 }
                        .sortedWith(
                            compareByDescending<PcmFormat> { it.rate == rate }
                                .thenByDescending { rate % it.rate == 0 }
                                .thenByDescending { it.rate }
                                .thenByDescending { minOf(it.bits, maximumBits) }
                                .thenByDescending { it.channels == channels }
                        )
                val chosen = candidates.firstOrNull() ?: error("没有符合上限的USB格式")
                val bits =
                    listOf(32, 24, 16).first {
                        it <= chosen.bits &&
                            it <= maximumBits &&
                            (track.dsd ||
                                track.bits <= 0 ||
                                it <=
                                    (if (track.bits <= 16) 16
                                    else if (track.bits <= 24) 24 else 32))
                    }
                return StreamPlan(
                    WireCodec.PCM,
                    chosen.rate,
                    bits,
                    chosen.channels,
                    sourceRevision = track.revision,
                    lossless = track.lossless,
                )
            }
            val supported = caps.rates.filter { it <= maximumRate && it <= rate }.sortedDescending()
            val usbRate =
                if (targetRate in supported) targetRate
                else
                    supported.firstOrNull { rate % it == 0 }
                        ?: supported.firstOrNull()
                        ?: error("没有兼容且符合上限的USB采样率")
            return StreamPlan(
                WireCodec.PCM,
                usbRate,
                targetBits,
                channels,
                sourceRevision = track.revision,
                lossless = track.lossless,
            )
        }
        val convert =
            track.channels > 2 ||
                track.dsd ||
                rate > maximumRate ||
                track.bits > maximumBits ||
                track.format in setOf("unknown", "", "ape", "wv") ||
                track.rate <= 0
        return StreamPlan(
                if (convert) WireCodec.FLAC else WireCodec.ORIGINAL,
                targetRate,
                if (track.lossless) track.bits else 0,
                channels,
                0,
                track.revision,
                track.lossless,
            )
            .let { if (it.codec == WireCodec.FLAC) it.copy(bits = targetBits) else it }
    }
}
