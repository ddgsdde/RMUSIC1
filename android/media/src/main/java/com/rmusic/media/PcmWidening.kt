package com.rmusic.media

import java.nio.ByteBuffer

/** Little-endian signed PCM, left-aligned into a wider integer container without quantization. */
object PcmWidening {
    fun pack(
        source: ByteBuffer,
        target: ByteBuffer,
        inputBytes: Int,
        outputBytes: Int,
        channels: Int,
    ): Int {
        require(inputBytes in 2..4 && outputBytes in inputBytes..4 && channels in 1..2)
        target.clear()
        val frames =
            minOf(
                source.remaining() / (inputBytes * channels),
                target.remaining() / (outputBytes * channels),
            )
        val start = source.position()
        repeat(frames) { frame ->
            repeat(channels) { channel ->
                repeat(outputBytes - inputBytes) { target.put(0) }
                repeat(inputBytes) { byte ->
                    target.put(
                        source.get(
                            start + frame * inputBytes * channels + channel * inputBytes + byte
                        )
                    )
                }
            }
        }
        target.flip()
        return frames * inputBytes * channels
    }
}
