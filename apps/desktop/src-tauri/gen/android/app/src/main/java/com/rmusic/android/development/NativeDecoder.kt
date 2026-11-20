package com.rmusic.android.development

import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaDataSource
import android.media.MediaExtractor
import android.media.MediaFormat
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToLong

/** Codec-only adapter. No AudioTrack, service, Activity callback or independent network client. */
@androidx.annotation.Keep
object NativeDecoder {
    @JvmStatic external fun readAt(id: String, position: Long, output: ByteArray, offset: Int, size: Int): Int
    @JvmStatic external fun nextPacket(id: String, output: ByteArray): String
    @JvmStatic external fun size(id: String): Long
    @JvmStatic external fun cancelled(id: String): Boolean
    private val sessions = ConcurrentHashMap<String,Session>()
    fun activeSessions(): Int = sessions.size
    fun command(command: String, payload: String, output: ByteArray): String = try {
        val args = JSONObject(payload); val id = args.getString("id")
        when (command) {
            "open" -> { check(!sessions.containsKey(id)); sessions[id] = Session(id,args); JSONObject() }
            "read" -> { val session = sessions[id] ?: error("decoder session closed"); JSONObject().put("bytes",session.read(output)).put("processing",session.processing()) }
            "close" -> { sessions.remove(id)?.close(); JSONObject() }
            else -> error("unknown decoder command")
        }.toString()
    } catch (error: Exception) { JSONObject().put("error",error.message ?: error.javaClass.simpleName).toString() }
    private class Session(private val id: String,args: JSONObject) {
        private val extractor = MediaExtractor()
        private var codec: MediaCodec? = null
        private val rate = args.getInt("rate")
        private val channels = args.getInt("channels")
        private val target = args.getLong("skip")
        private var duration: Long? = null
        private var liveOpus = false
        private val packetOpus = args.has("opus_header")
        private val packetBytes = if (packetOpus) ByteArray(65536) else ByteArray(0)
        private var encoding = AudioFormat.ENCODING_PCM_16BIT
        private var inputEnded = false
        private var ended = false
        private var pending = ByteArray(0)
        private var cursor = 0
        private var decodedFrames = 0L
        private var returnedFrames = 0L
        private var firstPts: Long? = null
        private var sourceInfo = ""
        private var mime = ""
        init {
            try {
                val source = if (packetOpus) {
                    mime = "audio/opus"
                    val values = args.getJSONArray("opus_header")
                    val header = ByteArray(values.length()) { values.getInt(it).toByte() }
                    val delay = ((header[10].toInt() and 255) or ((header[11].toInt() and 255) shl 8)).toLong() * 1000000000L / 48000
                    MediaFormat.createAudioFormat(mime,rate,channels).apply {
                        setByteBuffer("csd-0",ByteBuffer.wrap(header))
                        setByteBuffer("csd-1",ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(delay).apply { flip() })
                        setByteBuffer("csd-2",ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(80000000L).apply { flip() })
                    }
                } else {
                    extractor.setDataSource(object : MediaDataSource() {
                        override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int) = NativeDecoder.readAt(id,position,buffer,offset,size)
                        override fun getSize() = NativeDecoder.size(id)
                        override fun close() { }
                    })
                    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true } ?: error("未找到音频轨道")
                    extractor.selectTrack(track)
                    extractor.getTrackFormat(track).also { mime = it.getString(MediaFormat.KEY_MIME)!! }
                }
                require(mime in listOf("audio/mp4a-latm","audio/opus")) { "系统预载只接受AAC／Opus" }
                sourceInfo = source.toString()
                duration = if (source.containsKey(MediaFormat.KEY_DURATION)) source.getLong(MediaFormat.KEY_DURATION).takeIf { it > 0 }?.let { (it * rate / 1000000.0).roundToLong() } else null
                liveOpus = packetOpus || mime == "audio/opus" && NativeDecoder.size(id) < 0
                // An unseekable Core stream carries its verified programme duration separately.
                // MediaExtractor cannot inspect the final Ogg granule before playback begins.
                if (duration == null) duration = args.optDouble("duration",Double.NaN).takeIf { it.isFinite() && it > 0 }?.let { (it * rate).roundToLong() }
                require(!liveOpus || duration != null) { "实时 Opus 流缺少已确认的节目时长" }
                source.setInteger(MediaFormat.KEY_PCM_ENCODING,AudioFormat.ENCODING_PCM_FLOAT)
                val decoder = MediaCodec.createDecoderByType(mime); codec = decoder
                decoder.configure(source,null,null,0); decoder.start()
            } catch (error: Exception) { close(); throw error }
        }
        fun read(output: ByteArray): Int {
            require(output.isNotEmpty() && output.size % (channels * 4) == 0)
            val decoder = codec ?: error("decoder closed")
            val info = MediaCodec.BufferInfo(); var progress = System.nanoTime()
            while (cursor == pending.size && !ended) {
                check(!cancelled(id)) { "decoder cancelled" }
                if (!inputEnded) {
                    val index = decoder.dequeueInputBuffer(10000)
                    if (index >= 0) {
                        val input = decoder.getInputBuffer(index)!!; input.clear()
                        val packet = if (packetOpus) JSONObject(NativeDecoder.nextPacket(id,packetBytes)) else null
                        val count = packet?.getInt("bytes") ?: extractor.readSampleData(input,0)
                        if (count < 0) { decoder.queueInputBuffer(index,0,0,0,MediaCodec.BUFFER_FLAG_END_OF_STREAM); inputEnded = true }
                        else {
                            if (packetOpus) { require(count <= input.remaining()); input.put(packetBytes,0,count) }
                            decoder.queueInputBuffer(index,0,count,packet?.getLong("pts") ?: extractor.sampleTime,0)
                            if (!packetOpus) extractor.advance()
                        }
                        progress = System.nanoTime()
                    }
                }
                val index = decoder.dequeueOutputBuffer(info,10000)
                if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    val format = decoder.outputFormat
                    require(format.getInteger(MediaFormat.KEY_SAMPLE_RATE) == rate && format.getInteger(MediaFormat.KEY_CHANNEL_COUNT) == channels) { "源格式变化；不自动重采样" }
                    val nextEncoding = if (format.containsKey(MediaFormat.KEY_PCM_ENCODING)) format.getInteger(MediaFormat.KEY_PCM_ENCODING) else AudioFormat.ENCODING_PCM_16BIT
                    require(decodedFrames == 0L || nextEncoding == encoding) { "播放中解码精度变化，已停止" }
                    encoding = nextEncoding
                    require(encoding in listOf(AudioFormat.ENCODING_PCM_FLOAT,AudioFormat.ENCODING_PCM_16BIT,AudioFormat.ENCODING_PCM_32BIT)) { "不支持的系统解码PCM编码" }
                } else if (index >= 0) {
                    try {
                        if (info.size > 0) {
                            if (firstPts == null) firstPts = info.presentationTimeUs
                            val width = if (encoding == AudioFormat.ENCODING_PCM_16BIT) 2 else 4
                            require(info.size % (channels * width) == 0)
                            val frames = info.size / (channels * width); require(frames <= 65536)
                            // Packet Opus is decoded sequentially; Ogg tail timestamps can overlap
                            // after granule trimming. Count actual codec output after its pre-skip.
                            val start = if (packetOpus) decodedFrames else (info.presentationTimeUs * rate / 1000000.0).roundToLong()
                            val skip = max(0L,target-start).coerceAtMost(frames.toLong()).toInt()
                            val end = min(frames.toLong(),duration?.minus(start) ?: frames.toLong()).coerceAtLeast(skip.toLong()).toInt()
                            val input = decoder.getOutputBuffer(index)!!.order(ByteOrder.LITTLE_ENDIAN)
                            input.position(info.offset + skip * channels * width); input.limit(info.offset + end * channels * width)
                            val data = ByteBuffer.allocate((end-skip)*channels*4).order(ByteOrder.LITTLE_ENDIAN)
                            while (input.hasRemaining()) {
                                val value = when (encoding) {
                                    AudioFormat.ENCODING_PCM_16BIT -> input.short.toInt() shl 16
                                    AudioFormat.ENCODING_PCM_32BIT -> input.int
                                    else -> { val value = input.float; require(value.isFinite()); (value.toDouble()*2147483648.0).coerceIn(Int.MIN_VALUE.toDouble(),Int.MAX_VALUE.toDouble()).toInt() }
                                }
                                data.putInt(value)
                            }
                            pending = data.array(); cursor = 0; decodedFrames += frames
                        }
                        ended = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                        progress = System.nanoTime()
                    } finally { decoder.releaseOutputBuffer(index,false) }
                }
                check(System.nanoTime()-progress < 15000000000L) { "系统解码器停止推进" }
            }
            val count = min(output.size,pending.size-cursor)
            pending.copyInto(output,0,cursor,cursor+count); cursor += count; returnedFrames += count/(channels*4)
            if (count == 0 && ended && liveOpus) check(returnedFrames >= max(0L,duration!!-target)) { "实时 Opus 流提前结束，节目帧尚未完整读取" }
            return count
        }
        fun close() {
            if (BuildConfig.DEBUG) android.util.Log.d("RMusicDecoder",JSONObject().put("source",sourceInfo).put("first_pts",firstPts).put("duration_frames",duration).put("encoding",encoding).put("decoded_frames",decodedFrames).put("returned_frames",returnedFrames).toString())
            try { codec?.let { try { it.stop() } catch (_: IllegalStateException) { } finally { it.release() } } } finally { codec = null; extractor.release() }
        }
        fun processing(): String {
            val label = if (mime == "audio/opus") "Opus" else "AAC"
            val precision = if (encoding == AudioFormat.ENCODING_PCM_FLOAT) "float32" else if (encoding == AudioFormat.ENCODING_PCM_32BIT) "PCM32" else "PCM16"
            return "Android $label 解码：$precision → PCM32 容器"
        }
    }
}
