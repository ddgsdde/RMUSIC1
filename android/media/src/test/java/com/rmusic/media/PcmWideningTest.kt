package com.rmusic.media

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.*
import org.junit.Test

class PcmWideningTest {
    @Test
    fun integerWideningPreservesSignedAmplitudeAndLowOrderBits() {
        val values = listOf(-8388608, -65537, -1, 0, 1, 257, 65537, 8388607)
        val source = ByteBuffer.allocate(values.size * 3)
        values.forEach { v -> repeat(3) { source.put((v shr (it * 8)).toByte()) } }
        source.flip()
        val target = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(source.remaining(), PcmWidening.pack(source, target, 3, 4, 2))
        assertEquals(
            0,
            source.position(),
        ) // The caller advances only after AudioTrack accepts the bytes.
        values.forEach { assertEquals(it shl 8, target.int) }
    }

    @Test
    fun boundedScratchBufferConsumesOnlyWholeStereoFrames() {
        val source = ByteBuffer.allocate(24)
        val target = ByteBuffer.allocate(17)
        assertEquals(12, PcmWidening.pack(source, target, 3, 4, 2))
        assertEquals(16, target.remaining())
    }
}
