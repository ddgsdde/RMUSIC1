@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.media

import androidx.media3.datasource.*
import androidx.media3.datasource.cache.*
import java.io.IOException

/** A failed optional disk write must never turn a healthy network stream into a playback error. */
class PlaybackCacheSink(
    private val cache: Cache,
    private val unavailable: (IOException) -> Unit = {},
) : DataSink {
    private var delegate: CacheDataSink? = null

    override fun open(spec: DataSpec) {
        delegate = CacheDataSink(cache, 256 * 1024)
        try {
            delegate!!.open(
                spec
                    .buildUpon()
                    .setFlags(spec.flags or DataSpec.FLAG_ALLOW_CACHE_FRAGMENTATION)
                    .build()
            )
        } catch (e: IOException) {
            disable(e)
        }
    }

    override fun write(buffer: ByteArray, offset: Int, length: Int) {
        try {
            delegate?.write(buffer, offset, length)
        } catch (e: IOException) {
            disable(e)
        }
    }

    override fun close() {
        try {
            delegate?.close()
        } catch (e: IOException) {
            unavailable(e)
        } finally {
            delegate = null
        }
    }

    private fun disable(error: IOException) {
        runCatching { delegate?.close() }
        delegate = null
        unavailable(error)
    }
}
