@file:OptIn(androidx.media3.common.util.UnstableApi::class)

package com.rmusic.nativeapp.playback

import android.media.AudioTrack
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.exoplayer.audio.*
import com.rmusic.media.OutputDevices
import com.rmusic.media.PcmWidening
import java.nio.ByteBuffer
import kotlin.math.min

/** Integer PCM only; no AudioProcessor/SRC/volume processing in this path. */
class ExactPcmSink(delegate: AudioSink, private val devices: OutputDevices) :
    ForwardingAudioSink(delegate) {
    private var track: AudioTrack? = null
    private var format: Format? = null
    private var inputBytes = 0
    private var outputBytes = 0
    private var rate = 48000
    private var channels = 2
    private var written = 0L
    private var firstUs = 0L
    private var draining = false
    private var started = false
    private var lastHead = 0L
    private var wraps = 0L
    private var pending: ByteBuffer? = null
    private var pendingInput = 0
    private var converted = ByteBuffer.allocateDirect(65536)

    override fun supportsFormat(format: Format) =
        getFormatSupport(format) != AudioSink.SINK_FORMAT_UNSUPPORTED

    override fun getFormatSupport(format: Format): Int =
        if (
            format.sampleMimeType == "audio/raw" &&
                format.pcmEncoding in
                    setOf(C.ENCODING_PCM_16BIT, C.ENCODING_PCM_24BIT, C.ENCODING_PCM_32BIT)
        )
            AudioSink.SINK_FORMAT_SUPPORTED_DIRECTLY
        else AudioSink.SINK_FORMAT_UNSUPPORTED

    override fun configure(
        inputFormat: Format,
        specifiedBufferSize: Int,
        outputChannels: IntArray?,
    ) {
        flush()
        val output = devices.exactFormat()
        require(
            inputFormat.sampleRate == output.sampleRate &&
                inputFormat.channelCount == output.channelCount
        ) {
            "USB采样率或声道与流不一致"
        }
        require(
            inputFormat.pcmEncoding in
                setOf(C.ENCODING_PCM_16BIT, C.ENCODING_PCM_24BIT, C.ENCODING_PCM_32BIT)
        ) {
            "USB原样输出只接受已确认的整数PCM，请使用Core PCM缓存或共享输出"
        }
        inputBytes = OutputDevices.bits(inputFormat.pcmEncoding) / 8
        outputBytes = OutputDevices.bits(output.encoding) / 8
        require(inputBytes in 2..4 && outputBytes in inputBytes..4) { "USB整数格式不兼容，拒绝截断样本" }
        rate = inputFormat.sampleRate
        channels = inputFormat.channelCount
        format = inputFormat
    }

    override fun play() {
        started = true
        track?.play()
    }

    override fun pause() {
        started = false
        track?.pause()
    }

    override fun handleBuffer(
        buffer: ByteBuffer,
        presentationTimeUs: Long,
        encodedAccessUnitCount: Int,
    ): Boolean {
        if (!buffer.hasRemaining()) return true
        if (track == null) {
            track = devices.openExact()
            if (started) track!!.play()
            firstUs = presentationTimeUs
        }
        val sourceFrame = inputBytes * channels
        val targetFrame = outputBytes * channels
        if (inputBytes == outputBytes) {
            val size = track!!.write(buffer, buffer.remaining(), AudioTrack.WRITE_NON_BLOCKING)
            check(size >= 0) { "USB写入失败（$size）" }
            written += size / targetFrame
            return !buffer.hasRemaining()
        }
        if (pending == null) {
            val frames = min(buffer.remaining() / sourceFrame, converted.capacity() / targetFrame)
            if (frames == 0) return false
            pendingInput = PcmWidening.pack(buffer, converted, inputBytes, outputBytes, channels)
            pending = converted
        }
        val bytes = track!!.write(pending!!, pending!!.remaining(), AudioTrack.WRITE_NON_BLOCKING)
        check(bytes >= 0) { "USB写入失败（$bytes）" }
        written += bytes / targetFrame
        if (!pending!!.hasRemaining()) {
            buffer.position(buffer.position() + pendingInput)
            pending = null
            pendingInput = 0
        }
        return !buffer.hasRemaining()
    }

    private fun played(): Long {
        val head = track?.playbackHeadPosition?.toLong()?.and(0xffffffffL) ?: return 0
        if (head < lastHead && lastHead - head > 0x7fffffff) wraps++
        lastHead = head
        return ((wraps shl 32) + head).coerceAtMost(written)
    }

    override fun getCurrentPositionUs(sourceEnded: Boolean): Long =
        if (track == null) AudioSink.CURRENT_POSITION_NOT_SET
        else firstUs + played() * 1000000L / rate

    override fun playToEndOfStream() {
        draining = true
    }

    override fun hasPendingData() = pending != null || played() < written

    override fun isEnded() = draining && !hasPendingData()

    override fun flush() {
        track?.let {
            runCatching {
                it.pause()
                it.flush()
            }
            it.release()
        }
        track = null
        pending = null
        pendingInput = 0
        written = 0
        lastHead = 0
        wraps = 0
        firstUs = 0
        draining = false
    }

    override fun reset() {
        flush()
        format = null
        super.reset()
    }

    override fun release() {
        reset()
        super.release()
    }

    override fun setVolume(
        volume: Float
    ) {} // USB original path does not apply software attenuation.
}
