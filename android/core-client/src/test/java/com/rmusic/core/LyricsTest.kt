package com.rmusic.core

import org.junit.Assert.*
import org.junit.Test

class LyricsTest {
    @Test
    fun qqTimestampsTranslationAndMultipleTagsArePreserved() {
        val rows = Lyrics.parse("[00:12:34]第一行\n[00:14.500][00:15.500]第二行", "[00:12.340]翻译")
        assertEquals(3, rows.size)
        assertEquals(12340, rows[0].millis)
        assertEquals("翻译", rows[0].translation)
        assertEquals(15500, rows[2].millis)
    }

    @Test
    fun pureInstrumentalAndUntimedLyricsAreNotFabricated() {
        assertTrue(Lyrics.parse("没有时间戳").isEmpty())
        assertEquals("纯音乐", Lyrics.parse("[00:00.00]纯音乐")[0].text)
    }

    @Test
    fun privateAddressRecognitionDoesNotTreatPublicLookalikesAsLocal() {
        assertTrue(CoreApi.isPrivateHost("172.20.0.1"))
        assertFalse(CoreApi.isPrivateHost("172.40.0.1"))
        assertFalse(CoreApi.isPrivateHost("10.evil.example"))
    }

    @Test
    fun artworkRespectsReverseProxyPrefixWithoutDuplicatingIt() {
        val api =
            CoreApi(CoreSession("https://music.example/prefix", "", "core", "user", "qa", false))
        assertEquals(
            "https://music.example/prefix/api/v1/artwork/one",
            api.image("/api/v1/artwork/one"),
        )
        assertEquals(
            "https://music.example/prefix/api/v1/covers/two",
            api.image("/prefix/api/v1/covers/two"),
        )
        assertNull(api.image("http://untrusted.example/image"))
        api.close()
    }
}
