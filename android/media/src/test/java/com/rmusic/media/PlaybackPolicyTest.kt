package com.rmusic.media

import com.rmusic.core.*
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.Assert.*
import org.junit.Test

class PlaybackPolicyTest {
    private fun track(rate: Int = 96000, bits: Int = 24, format: String = "flac") =
        Track(
            json(
                "id" to "exact:one",
                "format" to format,
                "sample_rate" to rate,
                "bit_depth" to bits,
                "channels" to 2,
            )
        )

    @Test
    fun wifiReplaySelectsLosslessInsteadOfKeepingMobileOpus() {
        val p = ListeningPolicy(mobileOpus = true)
        assertEquals(
            WireCodec.OPUS,
            PlaybackPolicy.choose(track(), false, DeviceCaps(96000, 24), p).codec,
        )
        assertEquals(
            WireCodec.ORIGINAL,
            PlaybackPolicy.choose(track(), true, DeviceCaps(96000, 24), p).codec,
        )
    }

    @Test
    fun bothNetworksRespectRealOutputAndUserLimits() {
        for (wifi in listOf(false, true)) {
            val a =
                PlaybackPolicy.choose(
                    track(192000, 32),
                    wifi,
                    DeviceCaps(48000, 16),
                    ListeningPolicy(),
                )
            assertEquals(WireCodec.FLAC, a.codec)
            assertEquals(48000, a.rate)
            assertEquals(16, a.bits)
        }
    }

    @Test
    fun dsdAlwaysRequiresCoreConversion() {
        val p =
            PlaybackPolicy.choose(track(2822400, 1, "dsf"), true, DeviceCaps(), ListeningPolicy())
        assertEquals(WireCodec.FLAC, p.codec)
        assertEquals(24, p.bits)
    }

    @Test
    fun cachedVariantsAreCoreAccountRevisionAndQualityScoped() {
        val p = PlaybackPolicy.choose(track(), false, DeviceCaps(), ListeningPolicy())
        assertNotEquals(p.cacheKey("one", "same"), p.cacheKey("two", "same"))
        assertNotEquals(p.cacheKey("one", "same"), p.copy(bits = 16).cacheKey("one", "same"))
        assertNotEquals(p.cacheKey("one", "same"), p.cacheKey("one", "same", 2.0))
        val before = track()
        val after =
            Track(
                org.json
                    .JSONObject(before.raw.toString())
                    .put("provider_data", json("source_revision" to "new-master"))
            )
        assertNotEquals(before.revision, after.revision)
    }

    @Test
    fun reverseProxyAndEncodedIdentitySurviveTranscodeUrl() {
        val plan = StreamPlan(WireCodec.OPUS, 48000, 0, 2, 160)
        val uri =
            plan.uri("https://music.example/prefix/api/v1/media/qq%3Aa%2Fb?ticket=secret", 12.5)
        val parsed = uri.toHttpUrl()
        assertEquals("/prefix/api/v1/mobile-opus/qq%3Aa%2Fb", parsed.encodedPath)
        assertEquals("secret", parsed.queryParameter("ticket"))
        assertEquals("true", parsed.queryParameter("accept_lossy"))
    }

    @Test
    fun strictUsbNeverFallsBackToAnUnreportedRoute() {
        try {
            PlaybackPolicy.choose(
                track(),
                true,
                DeviceCaps(),
                ListeningPolicy(outputMode = OutputMode.USB_BITPERFECT),
            )
            fail("must reject")
        } catch (expected: IllegalArgumentException) {}
        val p =
            PlaybackPolicy.choose(
                track(192000, 24),
                true,
                DeviceCaps(96000, 24, listOf(44100, 48000, 96000), true, true),
                ListeningPolicy(outputMode = OutputMode.USB_BITPERFECT),
            )
        assertEquals(WireCodec.PCM, p.codec)
        assertEquals(96000, p.rate)
        assertEquals(24, p.bits)
    }

    @Test
    fun lowerQualityCannotRetireACompleteLosslessCopy() {
        val lossless = StreamPlan(WireCodec.FLAC, 48000, 24, 2, lossless = true)
        val opus = StreamPlan(WireCodec.OPUS, 48000, 0, 2, 160)
        assertTrue(MediaCache.dominates(lossless, opus))
        assertFalse(MediaCache.dominates(opus, lossless))
    }

    @Test
    fun usbNegotiatesARealFormatTupleWithoutDroppingTwentyBitPrecision() {
        val caps =
            DeviceCaps(
                96000,
                32,
                listOf(48000, 96000),
                true,
                true,
                listOf(PcmFormat(96000, 24, 2), PcmFormat(48000, 32, 2)),
            )
        val policy = ListeningPolicy(outputMode = OutputMode.USB_BITPERFECT)
        val a = PlaybackPolicy.choose(track(192000, 32), true, caps, policy)
        assertEquals(96000, a.rate)
        assertEquals(24, a.bits)
        assertEquals(2, a.channels)
        val b = PlaybackPolicy.choose(track(96000, 20), true, caps, policy)
        assertEquals(24, b.bits)
        val mono = Track(track(96000, 24).raw.put("channels", 1))
        assertEquals(2, PlaybackPolicy.choose(mono, true, caps, policy).channels)
    }

    @Test
    fun multichannelSourceIsConvertedBeforeStereoDelivery() {
        val surround = Track(track().raw.put("channels", 6))
        val a = PlaybackPolicy.choose(surround, true, DeviceCaps(96000, 24), ListeningPolicy())
        assertEquals(WireCodec.FLAC, a.codec)
        assertEquals(2, a.channels)
    }

    @Test
    fun compatibleFullLosslessCacheWinsOverNewMobileOpusButNotOverUserLimits() {
        val t = track()
        val desired = StreamPlan(WireCodec.OPUS, 48000, 0, 2, 160, t.revision)
        val full =
            StreamPlan(WireCodec.FLAC, 48000, 24, 2, sourceRevision = t.revision, lossless = true)
        val entry = MediaCache.Entry("full", t, full, 100, "qa")
        assertEquals(
            entry,
            MediaCache.preferredComplete(listOf(entry), "qa", t, desired, 48000, 24, false),
        )
        assertNull(MediaCache.preferredComplete(listOf(entry), "qa", t, desired, 48000, 16, false))
        assertNull(
            MediaCache.preferredComplete(listOf(entry), "another", t, desired, 48000, 24, false)
        )
        assertNull(
            MediaCache.preferredComplete(
                listOf(entry.copy(plan = full.copy(sourceRevision = "old"))),
                "qa",
                t,
                desired,
                48000,
                24,
                false,
            )
        )
        assertNull(MediaCache.preferredComplete(listOf(entry), "qa", t, desired, 48000, 24, true))
    }
}
