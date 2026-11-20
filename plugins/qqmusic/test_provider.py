import json
import asyncio
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from provider import Provider, ProviderError, PublicRequestBudget, OriginalFile, OriginalFileType, audio_fields, inspect_header, quality_candidates, track_from
from qqmusic_api import Credential
from qqmusic_api.core.exceptions import LoginDeviceLimitError
from qqmusic_api.models.login import QR, QRCodeLoginEvents, QRLoginResult, QRLoginType


def flac_header(rate=96000, bits=24, channels=2, seconds=120):
    packed = (rate << 44) | ((channels - 1) << 41) | ((bits - 1) << 36) | (rate * seconds)
    return b"fLaC" + b"\x80\x00\x00\x22" + b"\x00" * 10 + packed.to_bytes(8, "big") + b"\x00" * 16


class Comments(unittest.IsolatedAsyncioTestCase):
    async def test_artwork_probe_uses_anonymous_exact_identity_and_omitted_flags_stay_unknown(self):
        from qqmusic_api.models.singer import SingerDetailResponse
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            person={"basic_info":{"singer_mid":"Abc123","name":"Artist","has_photo":0}}
            p.metadata_client.singer.get_desc=lambda ids,**kw:{"singer_list":[person]}
            url='https://y.gtimg.cn/music/photo_new/T001R300x300M000Abc123.jpg'
            v=await p.dispatch('artwork_probe',{'url':url})
            self.assertEqual(v['image_status'],'unavailable'); self.assertIsNone(v['image_url'])
            self.assertEqual(v['id'],'Abc123');self.assertEqual(v['name'],'Artist')
            calls=p.metadata_request.await_count
            self.assertFalse((await p.dispatch('artwork_probe',{'url':'https://other.test/x'}))['matched'])
            self.assertEqual(p.metadata_request.await_count,calls)
            person['basic_info'].pop('has_photo')
            model=SingerDetailResponse.model_validate({'singer_list':[person]})
            p.metadata_client.singer.get_desc=lambda ids,**kw:model
            self.assertNotIn('image_status',await p.dispatch('artwork_probe',{'url':url}))
            person['basic_info']['singer_mid']='Other123'
            p.metadata_client.singer.get_desc=lambda ids,**kw:{'singer_list':[person]}
            with self.assertRaises(ValueError):await p.dispatch('artwork_probe',{'url':url})
            await p.close()

    async def test_read_cache_force_refresh_and_failure_keeps_content_without_secrets(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder),anonymous=True)
            provider.client.song.query_song=AsyncMock(return_value=SimpleNamespace(tracks=[{"id":123}]))
            record={"cmid":"abc","nick":"<b>听友</b>","content":"<script>x</script>文字&amp;换行\n第二行","pub_time":1,"praise_num":2,"reply_cnt":3,"encrypt_uin":"private-account","credential":"secret","sub_comments":[{"CmId":"reply","Nick":"子回复作者","Content":"回复正文","EncryptUin":"private-account","ParentComment":{"Nick":"原作者"}}]}
            provider.client.comment.get_hot_comments=AsyncMock(return_value=SimpleNamespace(comments=[SimpleNamespace(**record)],total=42,has_more=1))
            # The SDK normally returns Pydantic records.
            provider.client.comment.get_hot_comments.return_value.comments=[type("Item",(),{"seq_no":"987","model_dump":lambda self,**kw:record})()]
            first=await provider.dispatch("comments",{"track_id":"qqmusic:Abc12345","kind":"hot"})
            self.assertEqual(first["items"][0]["author"],"听友");self.assertNotIn("private-account",str(first));self.assertNotIn("secret",str(first));self.assertNotIn("<script>",first["items"][0]["text"])
            self.assertEqual(first["items"][0]["replies"][0]["text"],"回复正文");self.assertEqual(first["items"][0]["replies"][0]["reply_to"],"原作者");self.assertEqual(first["next_cursor"],"987");self.assertTrue((await provider.dispatch("comments",{"track_id":"qqmusic:Abc12345","kind":"hot"}))["cached"])
            self.assertEqual(provider.client.comment.get_hot_comments.await_count,1)
            await provider.dispatch("comments",{"track_id":"qqmusic:Abc12345","kind":"hot","refresh":True});self.assertEqual(provider.client.comment.get_hot_comments.await_count,2)
            provider.client.comment.get_hot_comments.side_effect=RuntimeError("offline")
            stale=await provider.dispatch("comments",{"track_id":"qqmusic:Abc12345","kind":"hot","refresh":True});self.assertTrue(stale["stale"]);self.assertEqual(stale["items"],first["items"])
            for options in ({"kind":"add_comment"},{"page":0},{"page_size":100},{"cursor":"../../"}):
                with self.assertRaises(ProviderError):await provider.dispatch("comments",{"track_id":"qqmusic:Abc12345",**options})
            await provider.close()


class Formats(unittest.TestCase):
    def test_native_hires_fields_and_quality_hint_do_not_claim_verified_parameters(self):
        file=OriginalFile.model_validate({"media_mid":"Media123","size_hires":20000,"hires_sample":96000,"hires_bitdepth":24,"size_flac":10000})
        track=track_from({"mid":"Abc12345","file":file.model_dump(),"album":{}})
        self.assertEqual(track["provider_data"]["quality_hint"]["quality"],"HIRES")
        self.assertFalse(track["provider_data"]["quality_hint"]["verified"])
        self.assertEqual(track["sample_rate"],0)
        self.assertEqual(OriginalFileType.HIRES.s,"RS01")

    def test_original_tiers_exclude_enhanced_audio_even_when_master_is_larger(self):
        song={"mid":"Abc12345","vs":["","","","Master123"],"file":{"size_hires":20000,"size_flac":10000,"size_new":[30000,40000,50000],"size_320mp3":5000}}
        self.assertEqual([v["type"] for v in quality_candidates(song)],["HIRES","FLAC","MP3_320"])
    def test_negative_upstream_indexes_are_unknown_unsigned_track_numbers(self):
        song={"mid":"Abc12345","title":"晴天","singer":[],"album":{},"file":{},"index_album":-31073,"index_cd":-1}
        track=track_from(song)
        self.assertEqual((track["track_number"],track["disc_number"]),(0,0))

    def test_actual_flac_parameters(self):
        result = inspect_header(flac_header())
        self.assertEqual((result["sample_rate"], result["bit_depth"], result["channels"]), (96000, 24, 2))
        self.assertEqual(result["duration"], 120)
        self.assertTrue(result["hires"])
        self.assertFalse(inspect_header(flac_header(44100, 16))["hires"])

    def test_lossy_ogg_is_never_hires(self):
        self.assertEqual(inspect_header(b"OggS" + b"\x00" * 50)["hires"], False)

    def test_ogg_identification_reports_real_rate_without_inventing_pcm_depth(self):
        def page(packet):
            return b"OggS" + b"\x00" * 22 + b"\x01" + bytes([len(packet)]) + packet
        vorbis = b"\x01vorbis" + b"\x00" * 4 + b"\x02" + (44100).to_bytes(4, "little") + b"\x00" * 18
        result = inspect_header(page(vorbis))
        self.assertEqual((result["sample_rate"], result["channels"]), (44100, 2))
        self.assertNotIn("bit_depth", result)
        opus = b"OpusHead\x01\x02\x00\x00" + (44100).to_bytes(4, "little") + b"\x00\x00\x00"
        result = inspect_header(page(opus))
        self.assertEqual((result["format"], result["sample_rate"]), ("opus", 48000))

    def test_html_or_encrypted_audio_rejected(self):
        for data in (b"<html>login required", b"\xaa\xbb\xcc", b"fLaC", flac_header(rate=0)):
            with self.assertRaises(ProviderError):
                inspect_header(data)

    def test_no_preview_or_encrypted_candidates(self):
        song = {"mid": "Abc123456", "file": {"size_try": 900, "size_flac": 7000, "size_128mp3": 1000}, "vs": []}
        candidates = quality_candidates(song)
        self.assertEqual([item["type"] for item in candidates], ["FLAC", "MP3_128"])
        self.assertFalse(any("TRY" in item["type"] for item in candidates))

    def test_recording_versions_not_merged_by_title(self):
        base = {"title": "夜航", "singer": [], "album": {}, "file": {}}
        self.assertNotEqual(track_from({**base, "mid": "Abc12345"})["recording_id"],
                            track_from({**base, "mid": "Def12345"})["recording_id"])


class Resolution(unittest.IsolatedAsyncioTestCase):
    async def test_lossless_policy_never_substitutes_enhanced_master(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder))
            song={"mid":"Abc12345","interval":120,"file":{"size_new":[10000]},"vs":["","","","Master123"]}
            provider.client.song.get_song_urls=AsyncMock(return_value=SimpleNamespace(expiration=600,data=[SimpleNamespace(filename="AI00Master123.flac",result=0,purl="master.flac",ekey="")]))
            provider.client.song.get_cdn_dispatch=AsyncMock(return_value=SimpleNamespace(sip=[]))
            with patch("provider.probe_url",return_value={"format":"flac","verified":True,"hires":True,"bit_depth":24,"sample_rate":96000,"content_length":10000}):
                with self.assertRaises(ProviderError):
                    await provider.resolve(song,quality_candidates(song),{"quality":"FLAC"})
            provider.client.song.get_song_urls.assert_not_awaited()
            await provider.close()

    async def test_hires_policy_rejects_cd_resolution_even_with_master_label(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder))
            song={"mid":"Abc12345","interval":120,"file":{"size_hires":10000,"media_mid":"Native123"}}
            provider.client.song.get_song_urls=AsyncMock(return_value=SimpleNamespace(expiration=600,data=[SimpleNamespace(filename="RS01Native123.flac",result=0,purl="native.flac",ekey="")]))
            provider.client.song.get_cdn_dispatch=AsyncMock(return_value=SimpleNamespace(sip=[]))
            with patch("provider.probe_url",return_value={"format":"flac","verified":True,"hires":False,"bit_depth":16,"sample_rate":44100,"content_length":10000}):
                with self.assertRaises(ProviderError) as error:
                    await provider.resolve(song,quality_candidates(song),{"quality":"HIRES"})
            self.assertEqual(error.exception.code,"hires_unavailable")
            await provider.close()

    async def test_native_policy_falls_back_from_denied_hires_to_original_flac(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder))
            song={"mid":"Abc12345","interval":120,"file":{"media_mid":"Native123","size_hires":20000,"size_flac":10000,"size_new":[50000]},"vs":["","","","Master123"]}
            provider.client.song.get_song_urls=AsyncMock(return_value=SimpleNamespace(expiration=600,data=[
                SimpleNamespace(filename="RS01Native123.flac",result=104003,purl="",ekey=""),
                SimpleNamespace(filename="F000Native123.flac",result=0,purl="native.flac",ekey="")]))
            provider.client.song.get_cdn_dispatch=AsyncMock(return_value=SimpleNamespace(sip=[]))
            with patch("provider.probe_url",return_value={"format":"flac","verified":True,"hires":False,"bit_depth":16,"sample_rate":44100,"content_length":10000}):
                result=await provider.resolve(song,quality_candidates(song),{"quality":"native"})
            self.assertEqual(result["quality"],"FLAC")
            files=provider.client.song.get_song_urls.call_args.args[0]
            self.assertEqual([v.file_type.s for v in files],["RS01","F000"])
            await provider.close()

    async def test_logout_cancels_pending_qr_before_credentials_can_return(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = Provider(Path(folder))
            pending = asyncio.create_task(asyncio.sleep(3600))
            provider.login_tasks["pending"] = pending
            provider.sessions["pending"] = {"status": "waiting"}
            await provider.dispatch("logout", {})
            self.assertTrue(pending.cancelled())
            self.assertFalse(provider.login_tasks)
            self.assertFalse(provider.sessions)

    async def test_lossy_substitution_cannot_pass_as_flac(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = Provider(Path(folder))
            song = {"mid": "Abc12345", "interval": 120, "file": {"media_mid": "Media12345", "size_flac": 10000}}
            provider.client.song.get_song_urls = AsyncMock(return_value=SimpleNamespace(expiration=600, data=[
                SimpleNamespace(filename="F000Media12345.flac", result=0, purl="audio.flac", ekey="")]))
            provider.client.song.get_cdn_dispatch = AsyncMock(return_value=SimpleNamespace(sip=[]))
            with patch("provider.probe_url", return_value={"format": "mp3", "verified": True, "content_length": 10000}):
                with self.assertRaises(ProviderError):
                    await provider.resolve(song, quality_candidates(song), {})

    async def test_denied_higher_quality_falls_back_without_false_hires(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = Provider(Path(folder))
            song = {"mid": "Abc12345", "interval": 120, "pay": {"pay_down": 1},
                    "file": {"media_mid": "Media12345", "size_flac": 10000, "size_320mp3": 5000}}
            provider.client.song.get_song_urls = AsyncMock(return_value=SimpleNamespace(expiration=600, data=[
                # Deliberately reversed: match filename, not response array order.
                SimpleNamespace(filename="M800Media12345.mp3", result=0, purl="M800Media12345.mp3?vkey=x", ekey=""),
                SimpleNamespace(filename="F000Media12345.flac", result=104003, purl="", ekey=""),
            ]))
            provider.client.song.get_cdn_dispatch = AsyncMock(return_value=SimpleNamespace(sip=["https://isure.stream.qqmusic.qq.com/"]))
            with patch("provider.probe_url", return_value={"format": "mp3", "verified": True, "hires": False, "content_length": 5000}):
                result = await provider.resolve(song, quality_candidates(song), {})
            self.assertEqual(result["quality"], "MP3_320")
            self.assertFalse(result["hires"])
            self.assertFalse(result["download_allowed"])
            self.assertTrue(result["cache_allowed"])

    async def test_preview_length_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = Provider(Path(folder))
            song = {"mid": "Abc12345", "interval": 120, "file": {"media_mid": "Media12345", "size_flac": 10000}}
            provider.client.song.get_song_urls = AsyncMock(return_value=SimpleNamespace(expiration=600, data=[
                SimpleNamespace(filename="F000Media12345.flac", result=0, purl="audio.flac", ekey="")]))
            provider.client.song.get_cdn_dispatch = AsyncMock(return_value=SimpleNamespace(sip=[]))
            with patch("provider.probe_url", return_value={"format": "flac", "verified": True, "duration": 30, "content_length": 2000}):
                with self.assertRaises(ProviderError):
                    await provider.resolve(song, quality_candidates(song), {})


class Login(unittest.IsolatedAsyncioTestCase):
    def fixture(self, folder):
        provider = Provider(Path(folder))
        # Only synthetic QR/credentials in an isolated temp directory; no live account/network.
        provider.client.login.get_qrcode = AsyncMock(side_effect=lambda kind: QR(b"synthetic-png", kind, "image/png", "test-qr"))

        async def pending(_qr):
            await asyncio.Future()

        provider.client.login.check_qrcode = AsyncMock(side_effect=pending)
        self.addAsyncCleanup(provider.close)
        return provider

    async def test_default_qq_uses_real_sdk_session_and_reports_login_type(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            state = await provider.dispatch("login_start", {})
            provider.client.login.get_qrcode.assert_awaited_once_with(QRLoginType.QQ)
            self.assertEqual(state["login_type"], "qq")
            self.assertEqual(state["status"], "waiting")
            poll = await provider.dispatch("login_poll", {"session_id": state["session_id"]})
            self.assertEqual(poll["login_type"], "qq")
            self.assertNotIn("credential", poll)
            await provider.close()

    async def test_wechat_type_is_passed_to_real_sdk_and_persisted_on_done(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            # This is a deliberately synthetic credential, never a real user credential.
            credential = Credential(musicid=123, musickey="W_X_SYNTHETIC_TEST", loginType=1)
            provider.client.login.check_qrcode = AsyncMock(return_value=QRLoginResult(QRCodeLoginEvents.DONE, credential))
            state = await provider.dispatch("login_start", {"type": "wx"})
            provider.client.login.get_qrcode.assert_awaited_once_with(QRLoginType.WX)
            self.assertEqual(QRLoginType.WX.value, "wx")
            self.assertEqual(state["login_type"], "wx")
            await provider.login_tasks[state["session_id"]]
            poll = await provider.dispatch("login_poll", {"session_id": state["session_id"]})
            self.assertEqual((poll["status"], poll["login_type"]), ("authenticated", "wx"))
            self.assertNotIn("qr_data_url", poll)
            self.assertNotIn("credential", poll)
            self.assertFalse(provider.login_tasks)
            self.assertTrue(provider.credential_path.exists())
            restored = Provider(Path(folder))
            self.addAsyncCleanup(restored.close)
            self.assertEqual(restored.credential_path, provider.credential_path)
            self.assertTrue(restored.client.credential.is_valid())
            self.assertEqual(restored.client.credential.login_type, 1)

    async def test_wechat_device_limit_reports_action_without_exposing_upstream_data(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            existing = Credential(musicid=123, musickey="W_X_SYNTHETIC_TEST", loginType=1)
            provider.save_credential(existing)
            saved = provider.credential_path.read_bytes()
            provider.client.login.check_qrcode = AsyncMock(side_effect=LoginDeviceLimitError(
                code=20279, data={"token": "SYNTHETIC_SECRET"}))
            state = await provider.dispatch("login_start", {"type": "wx"})
            await provider.login_tasks[state["session_id"]]
            poll = await provider.dispatch("login_poll", {"session_id": state["session_id"]})
            self.assertEqual(poll["status"], "error")
            self.assertIn("20279", poll["error"])
            self.assertIn("设备管理", poll["error"])
            self.assertNotIn("SYNTHETIC_SECRET", str(poll))
            self.assertNotIn("qr_data_url", poll)
            self.assertEqual(provider.credential_path.read_bytes(), saved)
            self.assertFalse(provider.login_tasks)

    async def test_invalid_type_does_not_cancel_or_replace_an_active_session(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            state = await provider.dispatch("login_start", {})
            sid = state["session_id"]
            task = provider.login_tasks[sid]
            for value in ("wechat", "WX", "", None, [], 1):
                with self.assertRaises(ProviderError) as error:
                    await provider.dispatch("login_start", {"type": value})
                self.assertEqual(error.exception.code, "invalid_login_type")
                self.assertIs(provider.login_tasks[sid], task)
                self.assertFalse(task.cancelled())
                self.assertEqual(provider.sessions[sid], state)
            provider.client.login.get_qrcode.assert_awaited_once()
            await provider.close()

    async def test_switch_waits_for_old_poll_cancellation_before_new_wechat_session(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            started = asyncio.Event()
            cancelled = asyncio.Event()

            async def old_poll(_qr):
                started.set()
                try:
                    await asyncio.Future()
                finally:
                    cancelled.set()

            provider.client.login.check_qrcode = AsyncMock(side_effect=old_poll)
            first = await provider.dispatch("login_start", {})
            old = provider.login_tasks[first["session_id"]]
            await asyncio.wait_for(started.wait(), 1)
            second = await provider.dispatch("login_start", {"type": "wx"})
            self.assertTrue(cancelled.is_set())
            self.assertTrue(old.cancelled())
            self.assertEqual(second["login_type"], "wx")
            self.assertNotIn(first["session_id"], provider.sessions)
            self.assertNotIn(first["session_id"], provider.login_tasks)
            with self.assertRaises(ProviderError):
                await provider.dispatch("login_poll", {"session_id": first["session_id"]})
            self.assertFalse(provider.credential_path.exists())
            await provider.close()

    async def test_refusal_and_expiry_are_terminal_and_cannot_save_later_done(self):
        class Events:
            def __init__(self, terminal):
                self.terminal = terminal

            async def __aiter__(self):
                yield QRLoginResult(QRCodeLoginEvents.CONF)
                yield QRLoginResult(self.terminal)
                yield QRLoginResult(QRCodeLoginEvents.DONE, Credential(musicid=123, musickey="SYNTHETIC_TEST"))

        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            for terminal, status in ((QRCodeLoginEvents.REFUSE, "refused"), (QRCodeLoginEvents.TIMEOUT, "expired")):
                provider.sessions["test"] = {"status": "waiting", "login_type": "wx", "qr_data_url": "synthetic"}
                with patch.object(provider, "save_credential") as save:
                    await provider._watch_login("test", Events(terminal))
                    save.assert_not_called()
                self.assertEqual(provider.sessions["test"]["status"], status)
                self.assertNotIn("qr_data_url", provider.sessions["test"])
            self.assertFalse(provider.credential_path.exists())

    async def test_missing_credential_or_early_stream_end_is_error_and_hides_qr(self):
        class Events:
            def __init__(self, event):
                self.event = event

            async def __aiter__(self):
                yield QRLoginResult(self.event)

        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            for event in (QRCodeLoginEvents.DONE, QRCodeLoginEvents.SCAN):
                provider.sessions["test"] = {"status": "waiting", "login_type": "wx", "qr_data_url": "synthetic"}
                with patch.object(provider, "save_credential") as save:
                    await provider._watch_login("test", Events(event))
                    save.assert_not_called()
                self.assertEqual(provider.sessions["test"]["status"], "error")
                self.assertNotIn("qr_data_url", provider.sessions["test"])
            self.assertFalse(provider.credential_path.exists())

    async def test_legacy_mobile_type_still_uses_upstream_enum(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            state = await provider.dispatch("login_start", {"type": "mobile"})
            provider.client.login.get_qrcode.assert_awaited_once_with(QRLoginType.MOBILE)
            self.assertEqual(state["login_type"], "mobile")
            await provider.close()


def song(mid):
    return {"mid": mid, "title": mid, "singer": [], "album": {}, "file": {"size_flac": 1000}}


def detail(songs=(), *, total=None, hasmore=0, code=0):
    return SimpleNamespace(songs=list(songs), total=len(songs) if total is None else total,
                           hasmore=hasmore, code=code, subcode=0, info=SimpleNamespace(picurl="https://y.gtimg.cn/liked.jpg"))


def playlist(pid, *, dirid=1):
    return SimpleNamespace(id=pid, dirid=dirid, title=f"歌单 {pid}", songnum=3, picurl="", invalid=False)


class Browsing(unittest.IsolatedAsyncioTestCase):
    async def test_inflight_account_change_never_returns_old_recommendations(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=self.fixture(folder)
            async def changed():
                provider.client.credential.musicid=456
                return SimpleNamespace(songs=[song("OldUser123")])
            provider.client.recommend.get_guess_recommend.side_effect=changed
            with self.assertRaises(ProviderError) as error:
                await provider.dispatch("recommendations",{})
            self.assertEqual(error.exception.code,"account_changed")
            self.assertFalse((Path(folder)/"recommendations.json").exists())
            await provider.close()
    async def test_daily_discovery_reuses_snapshot_across_refresh_and_process_restart(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=self.fixture(folder)
            provider.client.recommend.get_guess_recommend.return_value.songs=[song("Daily123")]
            first=await provider.dispatch("recommendations",{"limit":5})
            second=await provider.dispatch("recommendations",{"limit":5,"refresh":True})
            self.assertEqual(first,second)
            provider.client.recommend.get_guess_recommend.assert_awaited_once()
            restarted=self.fixture(folder)
            self.assertEqual((await restarted.dispatch("recommendations",{"limit":5}))["items"],first["items"])
            restarted.client.recommend.get_guess_recommend.assert_not_awaited()
            restarted.client.credential.musicid=456
            await restarted.dispatch("recommendations",{"limit":5})
            restarted.client.recommend.get_guess_recommend.assert_awaited_once()
            await provider.close();await restarted.close()

    async def test_liked_write_uses_normal_song_type_and_requires_readback(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=self.fixture(folder)
            provider.client.song=SimpleNamespace(query_song=AsyncMock(return_value=SimpleNamespace(tracks=[{"mid":"Liked123","id":123,"type":1}])))
            provider.client.songlist.like_song=AsyncMock(return_value=True)
            provider.playlist_snapshot=AsyncMock(side_effect=[{"track_ids":[],"revision":"before"},{"track_ids":["qqmusic:Liked123"],"revision":"after"}])
            result=await provider._playlist_apply({"playlist_id":"liked","action":"add","track_ids":["qqmusic:Liked123"],"revision":"before"})
            provider.client.songlist.like_song.assert_awaited_once_with([(123,0)])
            self.assertEqual(result["track_ids"],["qqmusic:Liked123"])
            await provider.close()
    async def test_playlist_write_uses_owned_directory_and_reads_back_before_success(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            provider.playlist_snapshot = AsyncMock(side_effect=[
                {"track_ids": [], "revision": "old"},
                {"track_ids": ["qqmusic:Cloud123"], "revision": "new"}])
            provider.client.song = SimpleNamespace(query_song=AsyncMock(return_value=SimpleNamespace(tracks=[{"mid":"Cloud123","id":99,"type":1}])))
            provider.client.songlist.add_songs = AsyncMock(return_value=True)
            result = await provider.dispatch("playlist_apply", {"playlist_id":"1234","action":"add","track_ids":["qqmusic:Cloud123"],"revision":"old"})
            provider.client.songlist.add_songs.assert_awaited_once_with(1, [(99,0)], tid=1234)
            self.assertEqual(result["revision"], "new")
            provider.playlist_snapshot = AsyncMock(return_value={"track_ids": [], "revision": "changed"})
            with self.assertRaisesRegex(ProviderError, "冲突"):
                await provider.dispatch("playlist_apply", {"playlist_id":"1234","action":"add","track_ids":["qqmusic:Cloud123"],"revision":"old"})
            with self.assertRaisesRegex(ProviderError, "只能修改"):
                await provider.dispatch("playlist_apply", {"playlist_id":"5678","action":"add","track_ids":["qqmusic:Cloud123"],"revision":"old"})
            self.assertEqual(provider.client.songlist.add_songs.await_count, 1)
    def fixture(self, folder):
        provider = Provider(Path(folder))
        provider.ensure_login = AsyncMock()
        provider.client = SimpleNamespace(
            credential=SimpleNamespace(musicid=123, login_type="wx", encrypt_uin="encrypted-account"),
            user=SimpleNamespace(
                get_fav_song=AsyncMock(return_value=detail([song("Liked123")], total=101, hasmore=1)),
                get_created_songlist=AsyncMock(return_value=SimpleNamespace(finished=True, playlists=[playlist(201, dirid=201), playlist(1234)])),
                get_fav_songlist=AsyncMock(return_value=SimpleNamespace(hide=False, playlists=[playlist(5678)], hasmore=0))),
            songlist=SimpleNamespace(get_detail=AsyncMock(return_value=detail([song("List1234")], total=101, hasmore=1))),
            recommend=SimpleNamespace(get_guess_recommend=AsyncMock(return_value=SimpleNamespace(songs=[])),
                                      get_radar_recommend=AsyncMock(return_value=SimpleNamespace(songs=[], has_more=False))))
        return provider

    async def test_all_uses_true_liked_api_and_separate_created_collected(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            result = await provider.dispatch("playlists", {})
            self.assertEqual([(v["id"], v["kind"]) for v in result["items"]],
                             [("liked", "liked"), ("1234", "created"), ("5678", "collected")])
            self.assertEqual(result["items"][0]["track_count"], 101)
            self.assertEqual(result["items"][0]["cover_url"], "https://y.gtimg.cn/liked.jpg")
            provider.client.user.get_fav_song.assert_awaited_once_with("encrypted-account", num=1, page=1)
            provider.client.user.get_fav_songlist.assert_awaited_once_with("encrypted-account", num=100, page=1)
            self.assertIsNone(result["next_page"])
            provider.ensure_login.assert_awaited_once()

    async def test_created_local_paging_does_not_repeat_liked_or_collected(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            provider.client.user.get_created_songlist.return_value.playlists = [playlist(i + 1) for i in range(101)]
            first = await provider.dispatch("playlists", {"collection": "created"})
            second = await provider.dispatch("playlists", {"collection": "created", "page": 2})
            self.assertEqual((len(first["items"]), first["next_page"]), (100, 2))
            self.assertEqual([v["id"] for v in second["items"]], ["101"])
            self.assertIsNone(second["next_page"])
            provider.client.user.get_fav_song.assert_not_awaited()
            provider.client.user.get_fav_songlist.assert_not_awaited()

    async def test_liked_track_page_uses_fav_song_not_playlist_favorites(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            result = await provider.dispatch("playlist_tracks", {"playlist_id": "liked", "page": 2})
            self.assertEqual((result["items"][0]["id"], result["playlist_id"], result["next_page"]), ("qqmusic:Liked123", "liked", 3))
            provider.client.user.get_fav_song.assert_awaited_once_with("encrypted-account", num=100, page=2)
            provider.client.songlist.get_detail.assert_not_awaited()
            provider.client.user.get_fav_songlist.assert_not_awaited()

    async def test_numeric_tracks_and_legacy_sync_preserve_total_and_page(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            result = await provider.dispatch("sync", {"playlist_id": "1234", "page": 2})
            self.assertEqual((result["total"], result["next_page"], result["items"][0]["source"]), (101, 3, "qqmusic"))
            provider.client.songlist.get_detail.assert_awaited_once_with(1234, num=100, page=2)
            result = await provider.dispatch("sync", {"collection": "favorites", "page": 2})
            self.assertEqual(result["playlists"][0]["kind"], "collected")
            self.assertEqual(result["items"], [])

    async def test_private_denied_and_incomplete_playlists_are_explicit(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            provider.client.user.get_fav_songlist.return_value.hide = True
            with self.assertRaisesRegex(ProviderError, "不可见"):
                await provider.dispatch("playlists", {"collection": "collected"})
            provider.client.songlist.get_detail.return_value = detail(code=403)
            with self.assertRaisesRegex(ProviderError, "没有权限"):
                await provider.dispatch("playlist_tracks", {"playlist_id": "1234"})
            provider.client.user.get_created_songlist.return_value.finished = False
            with self.assertRaisesRegex(ProviderError, "未完成"):
                await provider.dispatch("playlists", {"collection": "created"})

    async def test_bad_browse_parameters_never_request_upstream(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            for method, params in [("playlists", {"page": 0}), ("playlists", {"collection": "favorites"}),
                                   ("playlist_tracks", {"playlist_id": "-2"}), ("playlist_tracks", {"playlist_id": "１２３"})]:
                with self.assertRaises(ProviderError):
                    await provider.dispatch(method, params)
            provider.client.user.get_fav_song.assert_not_awaited()
            provider.client.user.get_created_songlist.assert_not_awaited()
            provider.client.songlist.get_detail.assert_not_awaited()

    async def test_cloud_recommendations_deduplicate_seed_and_recording_exclusions(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            provider.client.recommend.get_guess_recommend.return_value.songs = [song("Seed1234"), song("Track123A"), song("Track123A")]
            provider.client.recommend.get_radar_recommend.return_value = SimpleNamespace(songs=[song("Track123A"), song("Track123B"), song("Track123C")], has_more=False)
            result = await provider.dispatch("recommendations", {"seed_track_id": "qqmusic:Seed1234", "limit": 5,
                                                                  "exclude_recording_ids": ["qqmusic:Track123C"]})
            self.assertEqual([v["id"] for v in result["items"]], ["qqmusic:Track123A", "qqmusic:Track123B"])
            self.assertFalse(result["seed_applied"])
            self.assertEqual(result["basis"], "qq-personalized")
            self.assertEqual(result["upstream_methods"], ["guess", "radar"])
            provider.client.recommend.get_radar_recommend.assert_awaited_once_with(page=1)

    async def test_guess_failure_uses_only_real_radar_and_reports_warning(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            provider.client.recommend.get_guess_recommend.side_effect = RuntimeError("private upstream response")
            provider.client.recommend.get_radar_recommend.return_value = SimpleNamespace(songs=[song("Radar123")], has_more=False)
            result = await provider.dispatch("recommendations", {"limit": 10})
            self.assertEqual(result["upstream_methods"], ["radar"])
            self.assertEqual(result["items"][0]["id"], "qqmusic:Radar123")
            self.assertTrue(result["warnings"])
            self.assertNotIn("private upstream", str(result))

    async def test_empty_cloud_results_stay_empty_and_both_fail_explicitly(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = self.fixture(folder)
            self.assertEqual((await provider.dispatch("recommendations", {}))["items"], [])
            (Path(folder)/"recommendations.json").write_text("{}")
            provider.client.recommend.get_guess_recommend.side_effect = RuntimeError()
            provider.client.recommend.get_radar_recommend.side_effect = RuntimeError()
            with self.assertRaises(ProviderError) as caught:
                await provider.dispatch("recommendations", {"refresh": True})
            self.assertEqual(caught.exception.code, "recommendations_unavailable")

    async def test_account_features_require_login_before_upstream(self):
        with tempfile.TemporaryDirectory() as folder:
            provider = Provider(Path(folder))
            for method in ("playlists", "playlist_tracks", "recommendations"):
                with self.assertRaises(ProviderError) as caught:
                    await provider.dispatch(method, {"playlist_id": "liked"})
                self.assertEqual(caught.exception.code, "login_required")


if __name__ == "__main__":
    unittest.main()

class MetadataMatching(unittest.TestCase):
    def song(self, mid="Abc12345", title="夜航", album="专辑", duration=180):
        return {"mid":mid,"title":title,"singer":[{"name":"歌手"}],"album":{"title":album},"interval":duration,"file":{}}
    def test_exact_version_and_duration_match(self):
        from provider import metadata_match
        local={"title":"夜航","artist":"歌手","album":"专辑","duration":180}
        self.assertEqual(metadata_match(local,[self.song()])["mid"],"Abc12345")
        for song in [self.song(title="夜航 (Live)"),self.song(album="专辑 Remastered"),self.song(duration=190)]:
            self.assertIsNone(metadata_match(local,[song]))
    def test_ambiguous_same_name_editions_and_missing_duration_are_not_matched(self):
        from provider import metadata_match
        local={"title":"夜航","artist":"歌手","album":"专辑","duration":180}
        self.assertIsNone(metadata_match(local,[self.song(),self.song(mid="Def12345")]))
        self.assertIsNone(metadata_match({**local,"duration":0},[self.song()]))
    def test_markup_is_plain_text(self):
        from provider import plain
        self.assertEqual(plain("<p>发行 &amp; 制作</p>"),"发行 & 制作")

class MetadataFetch(unittest.IsolatedAsyncioTestCase):
    async def test_basic_metadata_does_not_wait_for_optional_calls_and_full_reuses_song_detail(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder));self.addAsyncCleanup(provider.close)
            provider.metadata_request=AsyncMock(side_effect=lambda request: request())
            song={"mid":"Abc12345","title":"夜航","singer":[],"album":{},"interval":180,"file":{}}
            provider.metadata_client.song.get_detail=AsyncMock(return_value={"track":song})
            provider.metadata_client.song.get_producer=AsyncMock(return_value={"data":[]})
            provider.metadata_client.lyric.get_lyric=AsyncMock(return_value={"lyric":"","trans":""})
            async def immediate(request):return await request()
            provider.metadata_request.side_effect=immediate
            basic=await provider.metadata({"mid":"Abc12345","section":"basic"})
            self.assertTrue(basic["partial"])
            provider.metadata_client.song.get_producer.assert_not_awaited()
            provider.metadata_client.lyric.get_lyric.assert_not_awaited()
            full=await provider.metadata({"mid":"Abc12345","section":"all"})
            self.assertNotIn("partial",full)
            provider.metadata_client.song.get_detail.assert_awaited_once()
    async def test_local_indexing_never_uses_account_client(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder))
            self.addAsyncCleanup(provider.close)
            song={"mid":"Abc12345","title":"夜航","singer":[{"name":"歌手"}],"album":{"title":"专辑"},"interval":180,"file":{}}
            provider.client.search.search_by_type=AsyncMock(side_effect=AssertionError("account search used"))
            provider.client.song.get_detail=AsyncMock(side_effect=AssertionError("account metadata used"))
            provider.metadata_client.search.search_by_type=AsyncMock(return_value=SimpleNamespace(song=[song]))
            provider.metadata_client.song.get_detail=AsyncMock(return_value={"track":song})
            provider.metadata_client.song.get_producer=AsyncMock(return_value={"data":[]})
            provider.metadata_client.lyric.get_lyric=AsyncMock(return_value={"lyric":""})
            with patch.object(provider.metadata_budget,"reserve",return_value=0):
                value=await provider.metadata({"track":{"title":"夜航","artist":"歌手","album":"专辑","duration":180}})
            self.assertTrue(value["matched"])
            self.assertFalse(provider.metadata_client.credential.is_valid())
            provider.client.search.search_by_type.assert_not_awaited()
            provider.client.song.get_detail.assert_not_awaited()

    async def test_public_throttle_and_rate_limit_never_fall_back_to_account(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder));self.addAsyncCleanup(provider.close)
            foreground=Provider(Path(folder),anonymous=True);self.addAsyncCleanup(foreground.close)
            request=AsyncMock(return_value={})
            clock=[100.0]
            async def advance(delay): clock[0]+=delay
            with patch("provider.time.time",side_effect=lambda:clock[0]),patch("provider.asyncio.sleep",side_effect=advance) as sleep:
                await provider.metadata_request(request);await foreground.metadata_request(request)
                self.assertAlmostEqual(sleep.await_args_list[0].args[0],1.1)
                limited=RuntimeError("limited");limited.response=SimpleNamespace(status_code=429)
                request.side_effect=limited
                with self.assertRaises(ProviderError):await provider.metadata_request(request)
                # Another worker and a restarted worker must observe the same cooldown.
                with self.assertRaises(ProviderError):await foreground.metadata_request(request)
                restarted=Provider(Path(folder),anonymous=True);self.addAsyncCleanup(restarted.close)
                with self.assertRaises(ProviderError):await restarted.metadata_request(request)
                self.assertEqual(request.await_count,3)
                clock[0]+=60
                request.side_effect=None
                await restarted.metadata_request(request)
                self.assertEqual(request.await_count,4)

    async def test_shared_budget_reserves_only_one_concurrent_request_and_survives_restart(self):
        with tempfile.TemporaryDirectory() as folder:
            first=PublicRequestBudget(Path(folder));second=PublicRequestBudget(Path(folder))
            with patch("provider.time.time",return_value=100):
                delays=await asyncio.gather(asyncio.to_thread(first.reserve),asyncio.to_thread(second.reserve))
                self.assertEqual(sum(delay==0 for delay in delays),1)
                self.assertAlmostEqual(max(delays),1.1)
                restarted=PublicRequestBudget(Path(folder))
                self.assertAlmostEqual(restarted.reserve(),1.1)
            with patch("provider.time.time",return_value=105):first.finish(False)
            with patch("provider.time.time",return_value=105):self.assertAlmostEqual(second.reserve(),1.1)

    async def test_anonymous_worker_does_not_read_a_saved_account(self):
        with tempfile.TemporaryDirectory() as folder:
            Path(folder,"credential.json").write_text("must never be parsed",encoding="utf-8")
            provider=Provider(Path(folder),anonymous=True);self.addAsyncCleanup(provider.close)
            self.assertFalse(provider.client.credential.is_valid())
            self.assertFalse(provider.metadata_client.credential.is_valid())

    async def test_background_provenance_and_optional_failures_are_retained(self):
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder))
            song={"mid":"Abc12345","title":"夜航","singer":[],"album":{},"interval":180,"file":{}}
            provider.metadata_client.song.get_detail=AsyncMock(return_value={"track":song,"intro":[{"value":"<p>真实介绍</p>"}],"pub_time":[{"value":"2020-01-01"}]})
            provider.metadata_client.song.get_producer=AsyncMock(return_value={"data":[{"title":"作曲","producers":[{"name":"作者"}]}]})
            provider.metadata_client.lyric.get_lyric=AsyncMock(side_effect=RuntimeError("upstream"))
            first=await provider.metadata({"mid":"Abc12345"})
            self.assertTrue(first["matched"])
            self.assertEqual(first["background"]["description"],"真实介绍")
            self.assertEqual(first["background"]["credits"][0]["names"],["作者"])
            self.assertEqual(first["source"],"qqmusic")
            self.assertIn("lyrics",first["warnings"])
            self.assertEqual(await provider.metadata({"mid":"Abc12345"}),first)
            provider.metadata_client.song.get_detail.assert_awaited_once()

class RequestPacing(unittest.IsolatedAsyncioTestCase):
    async def test_internal_requests_obey_configured_interval_and_shared_cooldown(self):
        from provider import Client, REQUEST_CATEGORY
        with tempfile.TemporaryDirectory() as folder:
            provider=Provider(Path(folder));self.addAsyncCleanup(provider.close)
            provider.request_limits.update(search_ms=250,cooldown_seconds=7)
            clock=[100.0]
            async def advance(delay):clock[0]+=delay
            token=REQUEST_CATEGORY.set("search")
            try:
                with patch("provider.time.time",side_effect=lambda:clock[0]),patch("provider.asyncio.sleep",side_effect=advance) as sleep,patch.object(Client,"execute",new_callable=AsyncMock) as execute:
                    await provider.client.execute(object());await provider.client.execute(object())
                    self.assertAlmostEqual(sleep.await_args_list[0].args[0],.25)
                    error=RuntimeError("limited");error.response=SimpleNamespace(status_code=429);execute.side_effect=error
                    with self.assertRaises(RuntimeError):await provider.client.execute(object())
                    with self.assertRaises(ProviderError):await provider.client.execute(object())
                    self.assertEqual(execute.await_count,3)
                    clock[0]+=7;execute.side_effect=None;await provider.client.execute(object());self.assertEqual(execute.await_count,4)
            finally:REQUEST_CATEGORY.reset(token)

class RequestPriority(unittest.TestCase):
    def test_shared_background_budget_yields_to_waiting_foreground_and_recovers_abandoned_waiter(self):
        with tempfile.TemporaryDirectory() as folder:
            foreground=PublicRequestBudget(Path(folder));background=PublicRequestBudget(Path(folder),background=True)
            with patch("provider.time.time",return_value=100):self.assertEqual(background.reserve(),0);self.assertAlmostEqual(foreground.reserve(),1.1)
            with patch("provider.time.time",return_value=101.2):self.assertAlmostEqual(background.reserve(),.1);self.assertEqual(foreground.reserve(),0);foreground.finish(False)
            with patch("provider.time.time",return_value=102.4):self.assertEqual(background.reserve(),0)
            with patch("provider.time.time",return_value=103):foreground.reserve()
            with patch("provider.time.time",return_value=109):self.assertEqual(background.reserve(),0)

class RelatedCatalog(unittest.IsolatedAsyncioTestCase):
    async def test_client_public_profile_skips_only_the_biography_request(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            cloud=track_from(self.song())
            p.metadata=AsyncMock(return_value={"matched":True,"source":"qqmusic","track":cloud,"artists":[{"mid":"Artist001","name":"歌手"}]})
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            p.metadata_client.album.get_song=lambda identity,**kw:{"album_mid":identity,"total_num":1,"song_list":[self.song()]}
            details=[]
            def detail(identity):
                details.append(identity)
                return {"album":{"mid":"Album001","desc":"Profile"}}
            p.metadata_client.album.get_detail=detail
            value=await p.dispatch("related_catalog",{"track":cloud,"kind":"album","skip_profile":True})
            self.assertEqual(len(value["items"]),1)
            self.assertEqual(value["profile"]["id"],"Album001")
            self.assertEqual(details,[])
            value=await p.dispatch("related_catalog",{"track":cloud,"kind":"album"})
            self.assertEqual(value["profile"]["description"],"Profile")
            self.assertEqual(details,["Album001"])
            await p.close()

    def song(self, mid="Song001", album="Album001", singer="Artist001", subtitle=""):
        return {"mid":mid,"title":"夜航","subtitle":subtitle,"interval":180,"singer":[{"mid":singer,"name":"歌手"}],"album":{"mid":album,"title":"专辑"},"file":{"size_flac":100}}
    async def test_pages_use_stable_ids_and_reject_foreign_members(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            cloud=track_from(self.song());local={**cloud,"id":"local:one","source":"local","provider_data":{}}
            p.metadata=AsyncMock(return_value={"matched":True,"source":"qqmusic","track":cloud,"artists":[{"mid":"Artist001","name":"歌手"}]})
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            p.metadata_client.album.get_song=lambda identity,**kwargs:{"album_mid":identity,"total_num":51,"song_list":[self.song()]}
            first=await p.dispatch("related_catalog",{"track":local})
            self.assertEqual(first["next_page"],2);self.assertEqual(first["collection"]["id"],"Album001")
            p.metadata_client.album.get_song=lambda identity,**kwargs:{"album_mid":identity,"total_num":51,"song_list":[self.song(mid="Song002")]}
            second=await p.dispatch("related_catalog",{"track":local,"page":2});self.assertIsNone(second["next_page"])
            p.metadata_cache.clear()
            p.metadata_client.album.get_song=lambda identity,**kwargs:{"album_mid":identity,"total_num":1,"song_list":[self.song(album="WrongAlbum")]}
            with self.assertRaises(ProviderError):await p.dispatch("related_catalog",{"track":local})
            p.metadata_client.singer.get_songs_list=lambda identity,**kwargs:{"singer_mid":identity,"total_num":1,"song_list":[self.song(singer="WrongSinger")]}
            with self.assertRaises(ProviderError):await p.dispatch("related_catalog",{"track":local,"kind":"artist"})
            with self.assertRaises(ProviderError):await p.dispatch("related_catalog",{"track":local,"kind":"artist","artist_mid":"WrongSinger"})
    async def test_catalog_profiles_are_scoped_cached_and_optional(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            cloud=track_from(self.song())
            p.metadata=AsyncMock(return_value={"matched":True,"source":"qqmusic","track":cloud,"artists":[{"mid":"Artist001","name":"歌手"}]})
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            p.metadata_client.singer.get_songs_list=lambda identity,**kw:{"singer_mid":identity,"total_num":1,"song_list":[self.song()]}
            calls=[]
            def desc(ids,**kw):
                calls.append(ids)
                return {"singer_list":[{"singer_mid":"Artist001","ex_info":{"desc":"<p>真实简介</p>","country":"JP"}}]}
            p.metadata_client.singer.get_desc=desc
            first=await p.dispatch("related_catalog",{"track":cloud,"kind":"artist","artist_mid":"Artist001"})
            self.assertIn("真实简介",first["profile"]["description"])
            self.assertEqual(first["profile"]["country"],"JP")
            await p.dispatch("related_catalog",{"track":cloud,"kind":"artist","artist_mid":"Artist001"})
            self.assertEqual(len(calls),1)
            p.metadata_cache.clear()
            p.metadata_client.singer.get_desc=lambda *args,**kw:(_ for _ in ()).throw(RuntimeError("offline"))
            partial=await p.dispatch("related_catalog",{"track":cloud,"kind":"artist"})
            self.assertEqual(len(partial["items"]),1)
            self.assertIn("notice",partial["profile"])

    async def test_uncertain_or_different_editions_stay_unlinked(self):
        from provider import same_edition,metadata_match
        cloud=track_from(self.song());local={**cloud,"id":"local:one","source":"local","provider_data":{}}
        for other in [self.song(subtitle="Live"),self.song(subtitle="Remastered"),self.song(subtitle="Cover")]:
            self.assertFalse(same_edition(local,track_from(other)));self.assertIsNone(metadata_match(local,[other]))
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True);p.metadata=AsyncMock(return_value={"matched":True,"track":track_from(self.song(subtitle="Live"))})
            result=await p.dispatch("related_catalog",{"track":local});self.assertFalse(result["matched"]);self.assertEqual(result["items"],[])

    async def test_local_catalog_checks_more_candidates_before_linking(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            cloud=track_from(self.song());local={**cloud,"id":"local:one","source":"local","provider_data":{}}
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            def search(**options):
                self.assertEqual(options['num'],50)
                return SimpleNamespace(song=[self.song(),self.song(mid='Different002')])
            p.metadata_client.search.search_by_type=search
            value=await p.dispatch('related_catalog',{'track':local})
            self.assertFalse(value['matched']);self.assertEqual(value['items'],[])

class Discography(unittest.IsolatedAsyncioTestCase):
    async def test_artist_albums_are_identity_scoped_paged_and_open_without_fake_tracks(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            song={"mid":"Song001","title":"Song","interval":120,"singer":[{"mid":"Artist001","name":"Artist"}],"album":{"mid":"Album001","title":"Album"},"file":{"size_flac":100}}
            track=track_from(song)
            p.metadata=AsyncMock(return_value={"matched":True,"track":track})
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            p.metadata_client.singer.get_album_list=lambda mid,**kw:{"singer_mid":mid,"total":31,"album_list":[{"mid":"Album002","title":"Another Album","time_public":"2026-01-01"}]}
            result=await p.dispatch('artist_albums',{"track":track,"artist_id":"Artist001"})
            self.assertEqual(result['next_page'],2);self.assertEqual(result['items'][0]['release_id'],'Album002');self.assertNotIn('recording_id',result['items'][0])
            with self.assertRaises(ProviderError):await p.dispatch('artist_albums',{"track":track,"artist_id":"OtherArtist"})
            p.metadata_client.album.get_song=lambda mid,**kw:{"album_mid":mid,"total_num":1,"song_list":[{**song,"album":{"mid":mid,"title":"Another Album"}}]}
            album=await p.dispatch('album_catalog',{"release_id":"Album002"});self.assertEqual(album['items'][0]['provider_data']['album_mid'],'Album002')
            p.metadata_cache.clear();p.metadata_client.album.get_song=lambda mid,**kw:{"album_mid":"ForeignAlbum","song_list":[]}
            with self.assertRaises(ProviderError):await p.dispatch('album_catalog',{"release_id":"Album002"})

class CatalogOrdering(unittest.IsolatedAsyncioTestCase):
    async def test_oldest_first_reverses_complete_catalog_with_partial_last_page(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            calls=[]
            def endpoint(mid,**kw):
                calls.append(kw)
                rows=[{'mid':f'Album{i:03}','title':str(i)} for i in range(66)]
                return {'singer_mid':mid,'total':66,'album_list':rows[(kw['page']-1)*30:kw['page']*30]}
            p.metadata_client.singer.get_album_list=endpoint
            p.metadata_request=AsyncMock(side_effect=lambda request:request())
            a=await p.ordered_singer_page('Artist001',1,'release_old',albums=True)
            b=await p.ordered_singer_page('Artist001',2,'release_old',albums=True)
            self.assertEqual([x['title']for x in a['album_list']],[str(i)for i in range(65,35,-1)])
            self.assertEqual([x['title']for x in b['album_list']],[str(i)for i in range(35,5,-1)])
            self.assertTrue(all(int(x['order'])==0 for x in calls))
            with self.assertRaises(ProviderError):await p.ordered_singer_page('Artist001',1,'fake_heat',albums=True)
            await p.close()

    async def test_platform_sort_and_release_date_are_real_metadata(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            calls=[]
            def endpoint(mid,**kw):calls.append(kw);return {'singer_mid':mid,'total_num':0,'song_list':[]}
            p.metadata_client.singer.get_songs_list=endpoint;p.metadata_request=AsyncMock(side_effect=lambda request:request())
            await p.ordered_singer_page('Artist001',1,'hot',albums=False)
            await p.ordered_singer_page('Artist001',1,'release_new',albums=False)
            self.assertEqual([int(x['order'])for x in calls],[1,0])
            self.assertEqual(track_from({'mid':'Song001','time_public':'2026-10-20'})['provider_data']['release_date'],'2026-10-20')
            self.assertEqual(track_from({'mid':'Song001'})['provider_data']['release_date'],'')
            await p.close()

class SongArtwork(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    def document(mid='Song001',image='https://y.qq.com/music/photo_new/T062R500x500M000PhotoDifferent01.jpg'):
        return 'window.__ssrFirstPageData__='+json.dumps(json.dumps({'song':{'mid':mid,'img':image,'playUrl':'must-not-be-retained','id':123}},ensure_ascii=False))+';'

    def test_share_image_is_bound_to_exact_song_not_the_filename_or_title(self):
        from qq_artwork import share_cover
        self.assertIn('PhotoDifferent01',share_cover(self.document(),'Song001'))
        self.assertIsNone(share_cover(self.document(),'Other001'))
        for url in ['http://y.qq.com/music/photo_new/T062R500x500M000PhotoDifferent01.jpg','https://evil.example/T062R500x500M000PhotoDifferent01.jpg','https://y.qq.com@evil.example/music/photo_new/T062R500x500M000PhotoDifferent01.jpg','https://y.qq.com/music/photo_new/T001R500x500M000PhotoDifferent01.jpg']:
            self.assertIsNone(share_cover(self.document(image=url),'Song001'))
        self.assertIsNone(share_cover('window.__ssrFirstPageData__={broken','Song001'))
        self.assertIsNone(share_cover('x'*(1024*1024+1),'Song001'))

    async def test_full_metadata_recovers_song_artwork_while_basic_stays_fast(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Provider(Path(folder),anonymous=True)
            song={'mid':'Song001','id':123,'title':'Song','singer':[], 'file':{},'album':{}}
            p.metadata_client.song.get_detail=lambda mid:{'track':song}
            async def request(fn): return await fn()
            p.metadata_request=request
            # Use an awaitable for SDK metadata, preserving real asynchronous control flow.
            async def detail(mid):return {'track':song}
            p.metadata_client.song.get_detail=detail
            with patch('provider.fetch_cover',new=AsyncMock(return_value='https://y.qq.com/music/photo_new/T062R500x500M000PhotoDifferent01.jpg')) as fetch:
                basic=await p.metadata({'mid':'Song001','section':'basic'});self.assertIsNone(basic['track']['cover_url']);fetch.assert_not_awaited()
                # Optional sections may fail; they must not discard a confirmed picture.
                p.metadata_client.song.get_producer=AsyncMock(return_value={})
                p.metadata_client.lyric.get_lyric=AsyncMock(return_value={})
                full=await p.metadata({'mid':'Song001','section':'all'})
                self.assertIn('PhotoDifferent01',full['track']['cover_url']);self.assertEqual(full['track']['provider_data']['artwork_basis'],'qq_mobile_share_song_mid')
                fetch.assert_awaited_once_with('Song001');self.assertNotIn('playUrl',str(full))
                await p.metadata({'mid':'Song001','section':'all'});fetch.assert_awaited_once()
            await p.close()
