import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

from lyric_timing import lyric_document, qrc_lines
from provider import Provider


class Timings(unittest.TestCase):
    def test_xml_uses_real_word_duration_and_keeps_silence(self):
        raw = '<QrcInfos><LyricInfo><Lyric_1 LyricContent="[1000,5000]Hello (1200,180)world(3400,700)&#10;[6000,500]乙(6100,150)"/></LyricInfo></QrcInfos>'
        doc = lyric_document(SimpleNamespace(lyric=raw, trans="[00:01.00]翻译", roma=""))
        self.assertEqual(doc['word_timing'][0]['words'], [
            {'time': 1.2, 'duration': .18, 'text': 'Hello '},
            {'time': 3.4, 'duration': .7, 'text': 'world'}])
        self.assertIn('[00:01.000]Hello world', doc['text'])
        self.assertEqual(doc['timing_source'], 'qqmusic_qrc')

    def test_multiline_xml_with_literal_ampersand_preserves_words_and_entities(self):
        raw = ('<?xml version="1.0"?>\n<QrcInfos><LyricInfo>'
               '<Lyric_1 LyricContent="[1000,4000]a & (1200,180)b &amp; c(1900,700)\n'
               '[6000,800]乙(6100,150)\n"/></LyricInfo></QrcInfos>')
        doc = lyric_document(SimpleNamespace(lyric=raw, trans="", roma=""))
        self.assertEqual(doc["text"], "[00:01.000]a & b & c\n[00:06.000]乙")
        self.assertEqual(doc["word_timing"][0]["words"][1], {"time": 1.9, "duration": .7, "text": "b & c"})

    def test_line_lyrics_invalid_order_and_entities_never_become_fake_words(self):
        for raw in ['[00:01.00]甲乙', '[1000,5000]甲(1200,100)乙(1100,200)',
                    '[1000,5000]没有字时间', '<!DOCTYPE x [<!ENTITY y "text">]><x/>']:
            self.assertIsNone(qrc_lines(raw))


class Requests(unittest.IsolatedAsyncioTestCase):
    async def test_provider_requests_qrc_instead_of_inferring_word_times(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            p.client.lyric.get_lyric=AsyncMock(return_value=SimpleNamespace(lyric='[1000,5000]甲(1200,180)乙(3400,700)',trans='',roma=''))
            try:
                result=await p.dispatch('lyrics',{'track_id':'qqmusic:Abc12345'})
                self.assertEqual(len(result['word_timing'][0]['words']),2)
                p.client.lyric.get_lyric.assert_awaited_once_with('Abc12345',trans=True,roma=True,qrc=True)
            finally: await p.close()

    async def test_invalid_xml_falls_back_to_lrc_even_with_instrumental_metadata(self):
        malformed = '<?xml version="1.0"?><QrcInfos><LyricInfo LyricContent="[00:01]纯音乐"/></QrcInfos>'
        with tempfile.TemporaryDirectory() as folder:
            p = Provider(Path(folder), anonymous=True)
            p.client.lyric.get_lyric = AsyncMock(side_effect=[
                SimpleNamespace(lyric=malformed, trans="", roma=""),
                SimpleNamespace(lyric="[00:01.00]测试", trans=malformed, roma=""),
            ])
            try:
                doc = await p.dispatch("lyrics", {"track_id": "qqmusic:Abc12345"})
                self.assertEqual(doc["text"], "[00:01.00]测试")
                self.assertEqual(doc["translation"], "")
                self.assertFalse(p.client.lyric.get_lyric.await_args.kwargs["qrc"])
            finally:
                await p.close()

    async def test_missing_or_failed_qrc_falls_back_to_line_lyrics(self):
        for first in [SimpleNamespace(lyric='[00:01.00]甲乙',trans='',roma=''), RuntimeError('unsupported')]:
            with tempfile.TemporaryDirectory() as folder:
                p=Provider(Path(folder),anonymous=True)
                p.client.lyric.get_lyric=AsyncMock(side_effect=[first,SimpleNamespace(lyric='[00:01.00]甲乙',trans='',roma='')])
                try:
                    result=await p.dispatch('lyrics',{'track_id':'qqmusic:Abc12345'})
                    self.assertNotIn('word_timing',result)
                    self.assertEqual(result['text'],'[00:01.00]甲乙')
                    if isinstance(first,Exception):
                        self.assertEqual(p.client.lyric.get_lyric.await_count,2)
                        self.assertFalse(p.client.lyric.get_lyric.await_args.kwargs['qrc'])
                    else:
                        self.assertEqual(p.client.lyric.get_lyric.await_count,1)
                finally: await p.close()
