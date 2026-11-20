package com.rmusic.media

import android.content.Context
import android.media.*
import android.os.Build
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.DefaultAudioSink
import com.rmusic.core.json
import java.lang.ref.WeakReference
import org.json.JSONObject

data class OutputDevice(
    val id: Int,
    val name: String,
    val type: String,
    val rates: List<Int>,
    val encodings: List<Int>,
    val usb: Boolean,
    val bitPerfect: Boolean,
)

@UnstableApi
class OutputDevices(context: Context, private val preferences: LocalPreferences) {
    private val manager = context.getSystemService(AudioManager::class.java)
    private var playing = WeakReference<AudioTrack>(null)
    private var requested: AudioMixerAttributes? = null
    private var preferred: AudioDeviceInfo? = null
    var confirmedMixer: JSONObject? = null
        private set

    private val attrs =
        AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()

    fun list(): List<OutputDevice> =
        manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).map { d ->
            val usb =
                d.type in
                    setOf(
                        AudioDeviceInfo.TYPE_USB_DEVICE,
                        AudioDeviceInfo.TYPE_USB_HEADSET,
                        AudioDeviceInfo.TYPE_USB_ACCESSORY,
                    )
            val strict =
                Build.VERSION.SDK_INT >= 34 &&
                    usb &&
                    manager.getSupportedMixerAttributes(d).any {
                        it.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT &&
                            it.format.encoding in integerEncodings
                    }
            OutputDevice(
                d.id,
                d.productName.toString(),
                when (d.type) {
                    AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
                    AudioDeviceInfo.TYPE_BLE_HEADSET -> "蓝牙"
                    AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "扬声器"
                    else -> if (usb) "USB DAC" else "音频输出"
                },
                d.sampleRates.toList(),
                d.encodings.toList(),
                usb,
                strict,
            )
        }

    fun caps(): DeviceCaps {
        val d = list().firstOrNull { it.id == preferences.policy.value.deviceId }
        val rate =
            d?.rates?.maxOrNull()
                ?: manager.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)?.toIntOrNull()
                ?: 48000
        val bits =
            when {
                d?.encodings?.contains(AudioFormat.ENCODING_PCM_32BIT) == true -> 32
                d?.encodings?.contains(AudioFormat.ENCODING_PCM_24BIT_PACKED) == true -> 24
                d?.encodings?.contains(AudioFormat.ENCODING_PCM_FLOAT) == true -> 24
                d?.encodings?.isNotEmpty() == true -> 16
                else -> 24
            }
        if (
            preferences.policy.value.outputMode == OutputMode.USB_BITPERFECT &&
                Build.VERSION.SDK_INT >= 34
        ) {
            val device =
                manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull {
                    it.id == preferences.policy.value.deviceId
                }
            val formats =
                device
                    ?.let {
                        manager
                            .getSupportedMixerAttributes(it)
                            .filter { m ->
                                m.mixerBehavior ==
                                    AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT &&
                                    m.format.encoding in integerEncodings
                            }
                            .map { m ->
                                PcmFormat(
                                    m.format.sampleRate,
                                    bits(m.format.encoding),
                                    m.format.channelCount,
                                )
                            }
                    }
                    .orEmpty()
            return DeviceCaps(
                formats.maxOfOrNull { it.rate } ?: rate,
                formats.maxOfOrNull { it.bits } ?: 16,
                formats.map { it.rate }.distinct(),
                d?.usb == true,
                formats.isNotEmpty(),
                formats,
            )
        }
        return DeviceCaps(
            rate.coerceIn(8000, 768000),
            if (preferences.policy.value.outputMode == OutputMode.TRACK_INTEGER) 16
            else minOf(bits, 24),
            d?.rates.orEmpty(),
            d?.usb == true,
            d?.bitPerfect == true,
        )
    }

    fun prepare(plan: StreamPlan) {
        clearMixer()
        preferred =
            manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull {
                it.id == preferences.policy.value.deviceId
            }
        if (preferences.policy.value.outputMode != OutputMode.USB_BITPERFECT) return
        require(Build.VERSION.SDK_INT >= 34 && preferred != null) { "USB原样输出需要Android14及指定USB设备" }
        val target =
            manager
                .getSupportedMixerAttributes(preferred!!)
                .sortedBy { bits(it.format.encoding) }
                .firstOrNull { m ->
                    m.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT &&
                        m.format.encoding in
                            setOf(
                                AudioFormat.ENCODING_PCM_16BIT,
                                AudioFormat.ENCODING_PCM_24BIT_PACKED,
                                AudioFormat.ENCODING_PCM_32BIT,
                            ) &&
                        m.format.sampleRate == plan.rate &&
                        bits(m.format.encoding) >= plan.bits &&
                        m.format.channelCount == plan.channels
                } ?: error("设备没有此格式的USB原样档位，请调整输出上限或输出方式")
        requested = target
    }

    fun exactFormat(): AudioFormat {
        check(Build.VERSION.SDK_INT >= 34)
        return checkNotNull(requested).format
    }

    fun openExact(): AudioTrack {
        check(Build.VERSION.SDK_INT >= 34)
        val m = checkNotNull(requested) { "USB输出尚未协商" }
        val d = checkNotNull(preferred)
        check(manager.setPreferredMixerAttributes(attrs, d, m)) { "系统拒绝USB原样输出" }
        val minimum =
            AudioTrack.getMinBufferSize(
                m.format.sampleRate,
                m.format.channelMask,
                m.format.encoding,
            )
        require(minimum > 0) { "系统不接受此USB格式" }
        val track =
            AudioTrack.Builder()
                .setAudioAttributes(attrs)
                .setAudioFormat(m.format)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(
                    maxOf(minimum, m.format.sampleRate * m.format.channelCount * 4 / 10)
                )
                .build()
        if (track.state != AudioTrack.STATE_INITIALIZED || !track.setPreferredDevice(d)) {
            track.release()
            clearMixer()
            error("USB AudioTrack创建或路由失败")
        }
        playing = WeakReference(track)
        confirmedMixer =
            json(
                "rate" to m.format.sampleRate,
                "bits" to bits(m.format.encoding),
                "behavior" to "bit_perfect",
                "evidence" to "Android原样混音请求已接受；DAC逐位未实测",
            )
        return track
    }

    val provider = DefaultAudioSink.AudioTrackProvider { config, attributes, session, context ->
        if (Build.VERSION.SDK_INT >= 34 && requested != null) {
            val m = requested!!
            require(
                config.sampleRate == m.format.sampleRate && config.encoding == m.format.encoding
            ) {
                "解码器输出与USB原样档位不一致，已拒绝伪装独占"
            }
            check(manager.setPreferredMixerAttributes(attrs, preferred!!, m)) { "系统拒绝USB原样混音属性" }
            confirmedMixer =
                json(
                    "rate" to m.format.sampleRate,
                    "bits" to bits(m.format.encoding),
                    "behavior" to "bit_perfect",
                    "evidence" to "Android已接受请求，DAC逐位结果未验证",
                )
        }
        DefaultAudioSink.AudioTrackProvider.DEFAULT.getAudioTrack(
                config,
                attributes,
                session,
                context,
            )
            .also { t ->
                preferred?.let {
                    if (!t.setPreferredDevice(it)) {
                        t.release()
                        error("系统拒绝指定输出设备")
                    }
                }
                playing = WeakReference(t)
            }
    }

    fun snapshot(): JSONObject {
        val track =
            playing.get()?.takeIf {
                runCatching { it.state == AudioTrack.STATE_INITIALIZED }.getOrDefault(false)
            }
        fun <T> observed(read: () -> T): T? =
            if (track == null) null else runCatching(read).getOrNull()
        return json(
            "device" to (observed { track!!.routedDevice?.productName?.toString() } ?: "系统默认输出"),
            "requested_device" to preferred?.productName?.toString(),
            "rate" to observed { track!!.sampleRate },
            "bits" to observed { bits(track!!.audioFormat) },
            "encoding" to observed { track!!.audioFormat },
            "container_bits" to
                observed { track!!.audioFormat }
                    ?.let {
                        if (it == AudioFormat.ENCODING_PCM_FLOAT) 32 else bits(it)
                    },
            "channels" to observed { track!!.channelCount },
            "buffer_frames" to observed { track!!.bufferSizeInFrames },
            "underruns" to observed { track!!.underrunCount },
            "offload" to
                (if (Build.VERSION.SDK_INT >= 29) observed { track!!.isOffloadedPlayback }
                else null),
            "mixer" to
                if (track != null && observed { track.routedDevice?.id } == preferred?.id)
                    confirmedMixer
                else null,
            "requested_mixer" to confirmedMixer,
            "mixer_note" to
                if (confirmedMixer == null) "系统未公开当前混音器格式；蓝牙编码由系统协商"
                else "Android原样混音属性已接受；不等同硬件逐位验证",
        )
    }

    fun volume(): Float =
        manager.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat() /
            manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)

    fun volume(value: Float) {
        if (preferences.policy.value.outputMode != OutputMode.USB_BITPERFECT)
            manager.setStreamVolume(
                AudioManager.STREAM_MUSIC,
                (value.coerceIn(0f, 1f) * manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
                    .toInt(),
                0,
            )
    }

    fun clearMixer() {
        if (Build.VERSION.SDK_INT >= 34)
            preferred?.let { manager.clearPreferredMixerAttributes(attrs, it) }
        requested = null
        confirmedMixer = null
    }

    fun release() {
        clearMixer()
        playing.clear()
    }

    companion object {
        private val integerEncodings =
            setOf(
                AudioFormat.ENCODING_PCM_16BIT,
                AudioFormat.ENCODING_PCM_24BIT_PACKED,
                AudioFormat.ENCODING_PCM_32BIT,
            )

        fun bits(encoding: Int) =
            when (encoding) {
                AudioFormat.ENCODING_PCM_8BIT -> 8
                AudioFormat.ENCODING_PCM_16BIT -> 16
                AudioFormat.ENCODING_PCM_24BIT_PACKED -> 24
                AudioFormat.ENCODING_PCM_32BIT -> 32
                AudioFormat.ENCODING_PCM_FLOAT -> 24
                else -> 0
            }
    }
}
