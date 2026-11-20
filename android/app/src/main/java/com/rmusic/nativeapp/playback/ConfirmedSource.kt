package com.rmusic.nativeapp.playback

import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.*
import com.rmusic.media.*
import java.io.IOException

@UnstableApi
class ConfirmedSource(private val delegate: DataSource, private val plan: StreamPlan) :
    DataSource by delegate {
    // DataSource's Java default method otherwise resolves to an empty map through delegation.
    override fun getResponseHeaders(): Map<String, List<String>> = delegate.responseHeaders

    override fun open(spec: DataSpec): Long {
        val length = delegate.open(spec)
        if (plan.codec != WireCodec.ORIGINAL) {
            val name =
                when (plan.codec) {
                    WireCodec.OPUS -> "X-RMusic-Mobile-Opus"
                    WireCodec.PCM -> "X-RMusic-Native-PCM"
                    else -> "X-RMusic-Native-FLAC"
                }
            val header =
                delegate.responseHeaders.entries
                    .firstOrNull { it.key.equals(name, true) }
                    ?.value
                    ?.joinToString()
                    .orEmpty()
            val fields =
                header
                    .split(';')
                    .map { it.trim().split('=', limit = 2) }
                    .filter { it.size == 2 }
                    .associate { it[0] to it[1] }
            if (
                fields["rate"] != plan.rate.toString() ||
                    fields["channels"] != plan.channels.toString() ||
                    plan.codec != WireCodec.OPUS && fields["bits"] != plan.bits.toString()
            ) {
                delegate.close()
                throw IOException("Core未确认实际音频流格式，已停止播放")
            }
        }
        return length
    }
}
