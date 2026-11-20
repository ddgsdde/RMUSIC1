import asyncio
import math
import shutil
import struct
import tempfile
import unittest
import wave
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

from pyatv.const import DeviceState, FeatureState
from pyatv.protocols.raop.audio_source import open_source

from adapter import Adapter, AdapterError, ExactReader, finite_number, media_url, fade_filters


class Validation(unittest.TestCase):
    def test_fades_use_current_seek_and_skip_unknown_tail(self):
        self.assertEqual(fade_filters({"transitions":{"fade_out_ms":2000}}),[])
        self.assertEqual(fade_filters({"position":30,"duration":120,"transitions":{"fade_in_ms":2000,"fade_out_ms":2000}}),["afade=t=out:st=88.0:d=2.0:curve=qsin"])
    def test_non_finite_values_rejected(self):
        for value in (float("nan"), float("inf"), -1):
            with self.assertRaises(AdapterError):
                finite_number(value)

    def test_media_urls_no_local_paths_or_credentials(self):
        for value in ("file:///secret", "C:/music/test.flac", "http://user:password@core/audio"):
            with self.assertRaises(AdapterError):
                media_url(value)
        self.assertEqual(media_url("http://127.0.0.1:7800/api/v1/media/1?ticket=short"),
                         "http://127.0.0.1:7800/api/v1/media/1?ticket=short")


class PlaybackState(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.adapter = Adapter(Path(self.temp.name))
        self.connection = SimpleNamespace(
            metadata=SimpleNamespace(playing=AsyncMock(return_value=SimpleNamespace(
                device_state=DeviceState.Idle, position=0, total_time=120))),
            remote_control=SimpleNamespace(stop=AsyncMock()),
            features=SimpleNamespace(get_feature=lambda _: SimpleNamespace(state=FeatureState.Available)),
            audio=SimpleNamespace(volume=50, set_volume=AsyncMock()),
            stream=SimpleNamespace(stream_file=AsyncMock()), close=lambda: set(),
        )
        self.adapter.connections["test"] = self.connection

    async def asyncTearDown(self):
        await self.adapter.close()
        self.temp.cleanup()

    async def test_natural_end_flag(self):
        result = await self.adapter.play({"device_id": "test", "url": "http://localhost/audio", "zone_id": "z1"})
        self.assertEqual(result["status"], "buffering")
        await self.adapter.tasks["test"]
        result = await self.adapter.state("test")
        self.assertTrue(result["ended"])
        self.assertEqual(result["zone_id"], "z1")

    async def test_async_failure_reported_not_success(self):
        self.connection.stream.stream_file.side_effect = RuntimeError("receiver lost")
        await self.adapter.play({"device_id": "test", "url": "http://localhost/audio"})
        await self.adapter.tasks["test"]
        result = await self.adapter.state("test")
        self.assertEqual(result["status"], "error")
        self.assertFalse(result["ended"])

    async def test_pause_saves_position_without_ended(self):
        blocker = asyncio.Event()
        async def block(*args, **kwargs):
            await blocker.wait()
        self.connection.stream.stream_file.side_effect = block
        self.connection.metadata.playing.return_value = SimpleNamespace(device_state=DeviceState.Playing, position=12, total_time=120)
        await self.adapter.play({"device_id": "test", "url": "http://localhost/audio"})
        await asyncio.sleep(0)
        result = await self.adapter.dispatch("pause", {"device_id": "test"})
        self.assertEqual((result["status"], result["position"], result["ended"]), ("paused", 12, False))

    async def test_dsp_rejected_before_play(self):
        with self.assertRaises(AdapterError) as error:
            await self.adapter.play({"device_id": "test", "url": "http://localhost/audio", "dsp": {"enabled": True}})
        self.assertEqual(error.exception.code, "dsp_unsupported")
        self.connection.stream.stream_file.assert_not_called()
    async def test_stop_cancels_local_stream_before_waiting_for_receiver(self):
        cancelled=asyncio.Event()
        async def stream(*args,**kwargs):
            try: await asyncio.Event().wait()
            finally: cancelled.set()
        async def remote_stop():
            self.assertTrue(cancelled.is_set())
            await asyncio.Event().wait()
        self.connection.stream.stream_file.side_effect=stream
        await self.adapter.play({"device_id":"test","url":"http://localhost/audio"})
        await asyncio.sleep(0)
        self.connection.remote_control.stop.side_effect=remote_stop
        await asyncio.wait_for(self.adapter._stop("test"),2)
        self.assertTrue(cancelled.is_set())
        self.connection.remote_control.stop.side_effect=None
    async def test_new_url_does_not_inherit_previous_seek_when_checking_fades(self):
        self.adapter.ffmpeg=None
        self.adapter.requests["test"]={"position":30}
        with self.assertRaises(AdapterError) as error:
            await self.adapter.play({"device_id":"test","url":"http://localhost/audio","transitions":{"fade_in_ms":2000}})
        self.assertEqual(error.exception.code,"fade_unsupported")

    async def test_volume_scaled_to_pyatv_percentage(self):
        await self.adapter.dispatch("volume", {"device_id": "test", "volume": .3})
        self.connection.audio.set_volume.assert_awaited_once_with(30)


class DecoderCompatibility(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(shutil.which("ffmpeg"), "FFmpeg unavailable")
    async def test_actual_ffmpeg_seek_lossless_pipe_decodes_in_pyatv(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "known.wav"
            with wave.open(str(source), "wb") as output:
                output.setnchannels(2)
                output.setsampwidth(2)
                output.setframerate(44100)
                output.writeframes(b"".join(struct.pack("<hh", int(math.sin(i / 44100 * 440 * 2 * math.pi) * 16000),
                                                       int(math.sin(i / 44100 * 440 * 2 * math.pi) * 16000)) for i in range(44100)))
            process = await asyncio.create_subprocess_exec(
                shutil.which("ffmpeg"), "-nostdin", "-hide_banner", "-loglevel", "error",
                "-ss", "0.25", "-i", str(source), "-vn", "-f", "flac", "-acodec", "flac", "pipe:1",
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL)
            audio = None
            try:
                # Test the exact pinned decoder path consumed by stream_file.
                audio = await open_source(ExactReader(process.stdout), 44100, 2, 2)
                frames = await audio.readframes(352)
                self.assertTrue(frames)
                self.assertEqual(len(frames) % 4, 0)
                self.assertTrue(any(frames))
            finally:
                if audio:
                    await audio.close()
                if process.returncode is None:
                    process.terminate()
                await process.wait()


if __name__ == "__main__":
    unittest.main()
