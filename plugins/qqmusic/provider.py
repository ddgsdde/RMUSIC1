"""QQ 音乐 JSON Lines provider. Stdout is exclusively the RPC channel."""

from __future__ import annotations

import asyncio
from lyric_timing import line_lyrics, lyric_document
from contextvars import ContextVar
import base64
import json
import logging
import math
import html
import hashlib
import os
import re
import sqlite3
import sys
import time
import urllib.parse
import urllib.request
import uuid
from contextlib import closing
from copy import deepcopy
from dataclasses import replace
from pathlib import Path
from typing import Any

from qqmusic_api import Client, Credential
from qq_artwork import fetch_cover
from public_metadata import artwork_identity, singer_image, singer_person
from qqmusic_api.core.exceptions import LoginDeviceLimitError, RatelimitedError
from qqmusic_api.models.login import QRCodeLoginEvents, QRLoginType
from qqmusic_api.modules.login_utils import QRCodeLoginSession
from qqmusic_api.modules.singer import OrderType
from qqmusic_api.modules.song import SongFileInfo, SongFileType, SongQueryInfo
from qqmusic_api.modules.song import BaseSongFileType
from qqmusic_api.core.request import BaseRequest
from qqmusic_api.models.base import File, Song
from qqmusic_api.models.song import GetSongDetailResponse, QuerySongResponse
from qqmusic_api.models.search import SearchByTypeResponse, SongSearch
from qqmusic_api.models.recommend import GuessRecommendResponse, RadarRecommendResponse, RecommendNewSongResponse
from qqmusic_api.models.songlist import GetSonglistDetailResponse
from pydantic import create_model

LOG = logging.getLogger("rmusic.qqmusic")
MANIFEST = json.loads(Path(__file__).with_name("manifest.json").read_text("utf-8"))
MID = re.compile(r"^[A-Za-z0-9]{5,64}$")


class OriginalFileType(BaseSongFileType):
    HIRES = ("RS01", ".flac")


class OriginalFile(File):
    size_hires: int = 0
    hires_sample: int = 0
    hires_bitdepth: int = 0


class OriginalSong(Song):
    file: OriginalFile
    volume: Any = None


class OriginalSearchSong(SongSearch):
    file: OriginalFile
    volume: Any = None


# The upstream SDK 0.8.2 omits Hi-Res fields. Preserve aliases and JSONPath
# extraction while extending only the song-bearing responses we consume.
AUDIO_MODELS = {
    model: create_model(f"Original{model.__name__}", __base__=model,
                        **{field: (list[song], deepcopy(model.model_fields[field]))})
    for model, field, song in (
        (QuerySongResponse, "tracks", OriginalSong),
        (SearchByTypeResponse, "song", OriginalSearchSong),
        (GuessRecommendResponse, "songs", OriginalSong),
        (RadarRecommendResponse, "songs", OriginalSong),
        (RecommendNewSongResponse, "songs", OriginalSong),
        (GetSonglistDetailResponse, "songs", OriginalSong),
    )
}
AUDIO_MODELS[GetSongDetailResponse] = create_model(
    "OriginalGetSongDetailResponse", __base__=GetSongDetailResponse,
    track=(OriginalSong, deepcopy(GetSongDetailResponse.model_fields["track"])),
)


def audio_fields(request):
    if isinstance(request, BaseRequest) and request.response_model in AUDIO_MODELS:
        return replace(request, response_model=AUDIO_MODELS[request.response_model])
    return request


def file_type(name: str):
    return OriginalFileType.HIRES if name == "HIRES" else SongFileType[name]


def lyric_line_document(result) -> dict:
    text = line_lyrics(result.lyric)
    return {"format": "lrc" if re.search(r"\[\d+:\d+", text) else "plain",
            "text": text, "translation": line_lyrics(result.trans), "romanization": line_lyrics(result.roma),
            "offset_ms": 0, "source": "qqmusic", "timing_source": "line"}


class ProviderError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def as_dict(value: Any, explicit=False) -> dict:
    return value if isinstance(value, dict) else value.model_dump(mode="json", exclude_unset=explicit)


def mid_from(params: dict) -> str:
    mid = str(params.get("mid") or params.get("track_id") or "").removeprefix("qqmusic:")
    if not MID.fullmatch(mid):
        raise ProviderError("invalid_track", "QQ 歌曲 MID 无效")
    return mid


def catalog_order(params: dict):
    value = params.get("sort", "default")
    if value not in {"", "default", "hot", "release_new", "release_old"}:
        raise ProviderError("invalid_sort", "平台不支持此排序")
    return OrderType.LATEST if value in {"release_new", "release_old"} else OrderType.HOT


def track_from(song: Any) -> dict:
    value = as_dict(song)
    mid = str(value["mid"])
    album = value.get("album") or {}
    artists = " / ".join(item["name"] for item in value.get("singer", []) if item.get("name"))
    cover_mid = album.get("mid") or album.get("pmid")
    choices = quality_candidates(value)
    gain = replay_gain(value)
    return {
        "id": f"qqmusic:{mid}", "title": value.get("title") or value.get("name", ""),
        "artist": artists, "album": album.get("title") or album.get("name", ""),
        "album_artist": artists, "duration": value.get("interval", 0),
        "track_number": max(0, value.get("index_album") or 0), "disc_number": max(0, value.get("index_cd", 1) or 0),
        "source": "qqmusic", "format": "unknown", "sample_rate": 0,
        "bit_depth": 0, "channels": 0, "bitrate": 0,
        "cover_url": f"https://y.gtimg.cn/music/photo_new/T002R300x300M000{cover_mid}.jpg" if cover_mid else None,
        # A provider MID is a recording boundary until an explicit metadata match exists.
        "recording_id": f"qqmusic:{mid}", "available": bool(choices),
        "created_at": int(time.time()),
        "provider_data": {"release_date": value.get("time_public") or album.get("time_public") or "", "mid": mid, "song_id": value.get("id"), "type": value.get("type", 0),
                          "artist_credits": [{"id": item.get("mid"), "name": item["name"], **(singer_image(item, item["mid"], 300) if item.get("mid") else {})} for item in value.get("singer", []) if item.get("name")],
                          "album_mid": album.get("mid"), "artist_mids": [item["mid"] for item in value.get("singer", []) if item.get("mid")], "subtitle": value.get("subtitle", ""),
                          "upstream_status": value.get("status"), "authorization": "unverified",
                          **({"qq_replay_gain": gain} if gain else {}),
                          "quality_hint": {"label": choices[0]["label"], "quality": choices[0]["type"],
                                           "verified": False, "basis": "provider_file_sizes"} if choices else None},
    }

def replay_gain(song: dict) -> dict | None:
    volume = song.get("volume")
    if not isinstance(volume, dict):
        return None
    gain, peak = volume.get("gain"), volume.get("peak")
    if type(gain) not in (int, float) or not -40 <= gain <= 30:
        return None
    if type(peak) not in (int, float) or not 0 < peak <= 2:
        return None
    return {"gain_db": gain, "sample_peak": peak, "reference_lufs": -18}


def quality_candidates(song: dict) -> list[dict]:
    """Original tiers only: never MASTER, Atmos, vinyl, AI or TRY variants."""
    file = song.get("file") or {}
    modern = file.get("size_new") or []
    media_mid = file.get("media_mid") or song["mid"]
    candidates = []
    for name, size, label, lossless in (
        ("HIRES", file.get("size_hires", 0), "Hi-Res", True),
        ("FLAC", file.get("size_flac", 0), "FLAC 无损", True),
        ("OGG_640", modern[5] if len(modern) > 5 else 0, "OGG 640 kbps", False),
        ("OGG_320", modern[3] if len(modern) > 3 else 0, "OGG 320 kbps", False),
        ("MP3_320", file.get("size_320mp3", 0), "MP3 320 kbps", False),
        ("OGG_192", file.get("size_192ogg", 0), "OGG 192 kbps", False),
        ("ACC_192", file.get("size_192aac", 0), "AAC 192 kbps", False),
        ("MP3_128", file.get("size_128mp3", 0), "MP3 128 kbps", False),
        ("ACC_96", file.get("size_96aac", 0), "AAC 96 kbps", False),
        ("ACC_48", file.get("size_48aac", 0), "AAC 48 kbps", False),
    ):
        if size:
            candidates.append({"type": name, "size": size, "media_mid": media_mid,
                               "label": label, "lossless": lossless})
    return candidates


def normalized(text: str) -> str:
    return "".join(str(text).casefold().split())


VERSION = re.compile(r"live|remaster(?:ed)?|remix|acoustic|instrumental|karaoke|cover|radio.?edit|现场|翻唱|重制|重录|伴奏|纯音乐|不插电", re.I)

def same_edition(local: dict, remote: dict) -> bool:
    # No punctuation stripping or removal of version suffixes; uncertainty stays unlinked.
    for key in ("title", "artist", "album"):
        if not local.get(key) or normalized(local[key]) in {"未知专辑", "未知艺术家", "unknown"} or normalized(local[key]) != normalized(remote.get(key, "")):
            return False
    duration = float(local.get("duration") or 0)
    if not math.isfinite(duration) or duration <= 0 or abs(duration - float(remote.get("duration") or 0)) > 2:
        return False
    for key in ("track_number", "disc_number"):
        if local.get(key) and remote.get(key) and int(local[key]) != int(remote[key]):
            return False
    signature = lambda track: set(VERSION.findall(normalized(track.get("title", "") + " " + track.get("provider_data", {}).get("subtitle", ""))))
    return signature(local) == signature(remote)


def metadata_match(local: dict, candidates: list[dict]) -> dict | None:
    """Do not conflate live/remastered titles, albums, or ambiguous editions."""
    matches = []
    for song in candidates:
        track = track_from(song)
        if normalized(track["title"]) != normalized(local.get("title", "")):
            continue
        if normalized(track["artist"]) != normalized(local.get("artist", "")):
            continue
        album = local.get("album", "")
        if album and album != "未知专辑" and normalized(album) != normalized(track["album"]):
            continue
        duration = float(local.get("duration") or 0)
        if duration <= 0 or abs(track["duration"] - duration) > 2:
            continue
        if not same_edition(local, track):
            continue
        matches.append(song)
    unique = {song["mid"]: song for song in matches}
    return next(iter(unique.values())) if len(unique) == 1 else None


def plain(text: Any) -> str:
    return html.unescape(re.sub(r"<[^>]*>", "", str(text or "")))[:20000].strip()


def inspect_header(data: bytes) -> dict:
    """Verify actual format. FLAC STREAMINFO carries exact PCM parameters."""
    if data.startswith(b"fLaC"):
        offset = 4
        while offset + 4 <= len(data):
            kind = data[offset] & 0x7F
            size = int.from_bytes(data[offset + 1:offset + 4], "big")
            block = data[offset + 4:offset + 4 + size]
            if kind == 0 and len(block) >= 34:
                packed = int.from_bytes(block[10:18], "big")
                rate = packed >> 44
                channels = ((packed >> 41) & 7) + 1
                bits = ((packed >> 36) & 31) + 1
                samples = packed & ((1 << 36) - 1)
                if rate == 0 or bits < 4:
                    raise ProviderError("invalid_audio", "FLAC STREAMINFO 参数无效")
                return {"format": "flac", "sample_rate": rate, "channels": channels,
                        "bit_depth": bits, "duration": samples / rate if rate else None,
                        "verified": True, "hires": bits > 16 or rate > 48000}
            offset += 4 + size
        raise ProviderError("invalid_audio", "FLAC 文件缺少有效 STREAMINFO")
    if data.startswith(b"OggS"):
        result = {"format": "ogg", "verified": True, "hires": False}
        if len(data) >= 27:
            start = 27 + data[26]
            packet = data[start:]
            if packet.startswith(b"\x01vorbis") and len(packet) >= 16:
                rate = int.from_bytes(packet[12:16], "little")
                channels = packet[11]
                if 8000 <= rate <= 768000 and 1 <= channels <= 64:
                    result.update(sample_rate=rate, channels=channels)
            elif packet.startswith(b"OpusHead") and len(packet) >= 19 and 1 <= packet[9] <= 64:
                # Opus decodes at 48 kHz; its header's input-rate field is informational.
                result.update(format="opus", sample_rate=48000, channels=packet[9])
        return result
    if data.startswith(b"ID3") or (len(data) > 1 and data[0] == 0xFF and data[1] & 0xE0 == 0xE0):
        return {"format": "mp3", "verified": True, "hires": False}
    if len(data) >= 12 and data[4:8] == b"ftyp":
        return {"format": "m4a", "verified": True, "hires": False}
    raise ProviderError("invalid_audio", "返回内容不是受支持的明文音频，未缓存或解密")


def probe_url(url: str) -> dict:
    req = urllib.request.Request(url, headers={"Range": "bytes=0-65535", "User-Agent": "R-Music/0.1", "Referer": "https://y.qq.com/"})
    with urllib.request.urlopen(req, timeout=20) as response:
        # Even when the CDN ignores Range, close after this bounded probe.
        data = response.read(65536)
        result = inspect_header(data)
        content_range = response.headers.get("Content-Range", "")
        total = content_range.rsplit("/", 1)[-1]
        if total.isdecimal():
            result["content_length"] = int(total)
        elif response.status == 200 and response.headers.get("Content-Length", "").isdecimal():
            result["content_length"] = int(response.headers["Content-Length"])
        return result


class PublicRequestBudget:
    """Shared by the foreground and background anonymous processes."""

    def __init__(self, directory: Path, background: bool = False):
        self.path = directory / "public-requests.sqlite"
        self.background = background
        self.interval = 1.1
        self.cooldown_seconds = 60
        with closing(sqlite3.connect(self.path, timeout=2)) as db, db:
            db.execute("CREATE TABLE IF NOT EXISTS budget(id INTEGER PRIMARY KEY CHECK(id=1), next_at REAL NOT NULL, cooldown REAL NOT NULL)")
            db.execute("INSERT OR IGNORE INTO budget VALUES(1,0,0)")
            db.execute("CREATE TABLE IF NOT EXISTS foreground(id INTEGER PRIMARY KEY CHECK(id=1), until REAL NOT NULL)")
            db.execute("INSERT OR IGNORE INTO foreground VALUES(1,0)")

    def reserve(self) -> float:
        with closing(sqlite3.connect(self.path, timeout=2)) as db, db:
            db.execute("BEGIN IMMEDIATE")
            now = time.time()
            if self.background:
                until = db.execute("SELECT until FROM foreground WHERE id=1").fetchone()[0]
                if until > now:
                    return min(.1, until - now)
            else:
                db.execute("UPDATE foreground SET until=? WHERE id=1", (now + 5,))
            next_at, cooldown = db.execute("SELECT next_at,cooldown FROM budget WHERE id=1").fetchone()
            if cooldown > now:
                raise ProviderError("metadata_rate_limited", "匿名资料请求已暂停，稍后重试")
            if next_at > now:
                return next_at - now
            db.execute("UPDATE budget SET next_at=? WHERE id=1", (now + self.interval,))
            return 0.0

    def finish(self, limited: bool) -> None:
        with closing(sqlite3.connect(self.path, timeout=2)) as db, db:
            now = time.time()
            db.execute("UPDATE budget SET next_at=MAX(next_at,?),cooldown=MAX(cooldown,?) WHERE id=1",
                       (now + self.interval, now + self.cooldown_seconds if limited else 0))
            if not self.background:
                db.execute("UPDATE foreground SET until=0 WHERE id=1")


REQUEST_CATEGORY = ContextVar("qq_request_category", default=None)


class PacedClient(Client):
    """Pace each upstream request, including pagination and quality fallback calls."""
    def __init__(self, directory: Path, limits: dict, background: bool = False, **kwargs):
        super().__init__(**kwargs)
        self.request_directory = directory
        self.request_limits = limits
        self.background = background
        self.request_budgets = {}
        self.request_locks = {}

    async def execute(self, request):
        category = REQUEST_CATEGORY.get()
        # Metadata has its own shared anonymous budget; login polling stays independent.
        if category is None or category == "metadata":
            return await super().execute(request)
        if category not in self.request_budgets:
            directory = self.request_directory / "request-limits" / category
            directory.mkdir(parents=True, exist_ok=True)
            self.request_budgets[category] = PublicRequestBudget(directory, self.background)
            self.request_locks[category] = asyncio.Lock()
        budget = self.request_budgets[category]
        budget.interval = self.request_limits.get(category + "_ms", 600) / 1000
        budget.cooldown_seconds = self.request_limits.get("cooldown_seconds", 60)
        async with self.request_locks[category]:
            while True:
                try:
                    delay = await asyncio.to_thread(budget.reserve)
                except ProviderError as exc:
                    raise ProviderError("rate_limited", "QQ 音乐接口冷却中，请稍后重试") from exc
                if delay <= 0:
                    break
                await asyncio.sleep(delay)
            limited = False
            try:
                return await super().execute(request)
            except Exception as exc:
                status = getattr(exc, "status_code", None) or getattr(getattr(exc, "response", None), "status_code", None)
                limited = isinstance(exc, RatelimitedError) or status == 429
                raise
            finally:
                await asyncio.to_thread(budget.finish, limited)


class Provider:
    def __init__(self, data_dir: Path, anonymous: bool = False, background: bool = False):
        self.data_dir = data_dir
        self.background = background
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.credential_path = data_dir / "credential.json"
        credential = None
        if not anonymous and self.credential_path.exists():
            credential = Credential.model_validate_json(self.credential_path.read_text("utf-8"))
        self.request_limits = {}
        self.client = PacedClient(data_dir, self.request_limits, background=background, credential=credential)
        # Public indexing never borrows the account client or its cookies.
        self.metadata_client = PacedClient(data_dir, self.request_limits, background=background, credential=Credential())
        self.metadata_lock = asyncio.Lock()
        self.metadata_budget = PublicRequestBudget(data_dir, background)
        self.sessions: dict[str, dict] = {}
        self.login_tasks: dict[str, asyncio.Task] = {}
        self.last_check = 0.0
        self.login_expired = False
        self.auth_lock = asyncio.Lock()
        self.metadata_cache: dict[str, dict] = {}
        self.metadata_cache_at = time.monotonic()
        self.comment_cache: dict[str, dict] = {}
        self.comment_cache_sizes: dict[str, int] = {}
        self.recommendation_task: asyncio.Task | None = None
        self.recommendation_account: str | None = None

    def save_credential(self, credential: Credential) -> None:
        self.login_expired = False
        temp = self.credential_path.with_suffix(".tmp")
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(credential.model_dump_json())
            stream.flush()
            os.fsync(stream.fileno())
        temp.replace(self.credential_path)
        self.client.credential = credential
        self.last_check = time.monotonic()

    async def ensure_login(self) -> None:
        async with self.auth_lock:
            if not self.client.credential.is_valid():
                raise ProviderError("login_required", "请先扫码登录 QQ 音乐")
            if self.background:
                return  # The foreground process alone refreshes and saves credentials.
            if time.monotonic() - self.last_check < 300:
                return
            if await self.client.login.check_expired():
                try:
                    refreshed = await self.client.login.refresh_credential()
                    self.save_credential(refreshed)
                except Exception as exc:
                    self.login_expired = True
                    LOG.warning("Credential refresh failed: %s", type(exc).__name__)
                    raise ProviderError("login_expired", "QQ 音乐登录已过期，请重新扫码") from exc
            self.last_check = time.monotonic()

    async def _watch_login(self, sid: str, session: QRCodeLoginSession) -> None:
        try:
            states = {QRCodeLoginEvents.SCAN: "waiting", QRCodeLoginEvents.CONF: "confirming",
                      QRCodeLoginEvents.DONE: "authenticated", QRCodeLoginEvents.REFUSE: "refused",
                      QRCodeLoginEvents.TIMEOUT: "expired"}
            async for event in session:
                state = self.sessions.get(sid)
                if state is None:
                    return
                if event.event == QRCodeLoginEvents.DONE:
                    if event.credential is None:
                        raise ProviderError("login_failed", "登录响应缺少凭证")
                    async with self.auth_lock:
                        self.save_credential(event.credential)
                state["status"] = states[event.event]
                if event.event in {QRCodeLoginEvents.DONE, QRCodeLoginEvents.REFUSE, QRCodeLoginEvents.TIMEOUT}:
                    state.pop("qr_data_url", None)
                    break
            else:
                raise ProviderError("login_failed", "扫码事件流意外结束")
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            message = "扫码登录失败，请重试"
            if isinstance(exc, LoginDeviceLimitError):
                message = "QQ 音乐登录设备数量超限（20279），请在官方 QQ 音乐客户端的设备管理中移除不再使用的设备，再重新扫码"
            if state := self.sessions.get(sid):
                state.update(status="error", error=message)
                state.pop("qr_data_url", None)
            LOG.warning("QR login failed: %s", type(exc).__name__)
        finally:
            self.login_tasks.pop(sid, None)

    async def dispatch(self, method: str, params: dict) -> Any:
        if self.background:
            self.client.credential = Credential.model_validate_json(self.credential_path.read_text("utf-8")) if self.credential_path.exists() else Credential()
        category = {"qualities": "resolve", "playlist_tracks": "playlists", "playlist_snapshot": "playlists", "playlist_apply": "playlists", "sync": "playlists", "related_catalog": "metadata", "artist_albums": "metadata", "album_catalog": "metadata", "artwork_probe": "metadata"}.get(method, method)
        token = REQUEST_CATEGORY.set(category if category in {"search", "resolve", "lyrics", "playlists", "recommendations", "metadata", "comments"} else None)
        try:
            return await self._dispatch(method, params)
        finally:
            REQUEST_CATEGORY.reset(token)

    async def _dispatch(self, method: str, params: dict) -> Any:
        if method == "manifest":
            return MANIFEST
        if method == "health":
            return {"status": "ready", "authenticated": self.client.credential.is_valid() and not self.login_expired,
                    "auth_status": "expired" if self.login_expired else "authenticated" if self.client.credential.is_valid() else "anonymous",
                    "login_pending": bool(self.login_tasks),
                    "upstream_version": "0.8.2", "device_tested": False,
                    "account_context": hashlib.sha256(f"{self.client.credential.login_type}:{self.client.credential.musicid}".encode()).hexdigest()[:24] if self.client.credential.is_valid() and not self.login_expired else "anonymous"}
        if method == "login_start":
            requested_type = params.get("type", "qq")
            if not isinstance(requested_type, str) or requested_type not in {"qq", "wx", "mobile"}:
                raise ProviderError("invalid_login_type", "扫码类型须为 qq 或 wx（兼容 mobile）")
            login_type = QRLoginType(requested_type)
            old_tasks = list(self.login_tasks.values())
            for task in old_tasks:
                task.cancel()
            await asyncio.gather(*old_tasks, return_exceptions=True)
            self.login_tasks.clear()
            self.sessions.clear()
            session = QRCodeLoginSession(self.client.login, login_type, interval=1.5, timeout_seconds=180)
            qr = await session.get_qrcode()
            sid = uuid.uuid4().hex
            state = {"session_id": sid, "status": "waiting", "login_type": login_type.value, "expires_at": int(time.time()) + 180,
                     "qr_data_url": f"data:{qr.mimetype or 'image/png'};base64,{base64.b64encode(qr.data).decode()}"}
            self.sessions[sid] = state
            self.login_tasks[sid] = asyncio.create_task(self._watch_login(sid, session))
            return dict(state)
        if method == "login_poll":
            sid = params.get("session_id")
            if sid not in self.sessions:
                raise ProviderError("session_missing", "扫码会话不存在或插件已重启")
            return dict(self.sessions[sid])
        if method == "logout":
            for task in self.login_tasks.values():
                task.cancel()
            await asyncio.gather(*self.login_tasks.values(), return_exceptions=True)
            self.login_tasks.clear()
            self.sessions.clear()
            self.login_expired = False
            self.client.credential = Credential()
            self.credential_path.unlink(missing_ok=True)
            return {"authenticated": False}
        if method == "search":
            query = str(params.get("q") or params.get("query") or "").strip()
            if not query or len(query) > 256:
                raise ProviderError("invalid_query", "搜索词长度须为 1–256 字符")
            result = await audio_fields(self.client.search.search_by_type(keyword=query, num=min(max(int(params.get("limit", 50)), 1), 100),
                                                            page=max(int(params.get("page", 1)), 1), highlight=False))
            return {"items": [track_from(song) for song in result.song], "total": result.total_num}
        if method == "public_metadata":
            from public_metadata import dispatch
            return dispatch(params)
        if method == "artwork_probe":
            url = params.get("url")
            identity = artwork_identity(url)
            if not identity:
                return {"matched": False}
            detail = as_dict(await self.metadata_request(lambda: self.metadata_client.singer.get_desc([identity], photos=False)), explicit=True)
            person = singer_person(detail, identity)
            return {"matched": True, "source": "qqmusic", "url": url, "kind": "artist", "id": identity,
                    "name": plain((person.get("basic_info") or {}).get("name")), **singer_image(person, identity)}
        if method == "artist_albums":
            return await self.artist_albums(params)
        if method == "album_catalog":
            return await self.album_catalog(params)
        if method == "related_catalog":
            return await self.related_catalog(params)
        if method == "metadata":
            return await self.metadata(params)
        if method == "comments":
            return await self.comments(params)
        if method == "sync":
            await self.ensure_login()
            return await self.sync(params)
        if method in {"playlists", "playlist_tracks", "recommendations", "playlist_snapshot", "playlist_apply"}:
            await self.ensure_login()
            return await getattr(self, method)(params)
        if method in {"resolve", "qualities"}:
            await self.ensure_login()
            result = await audio_fields(self.client.song.query_song([SongQueryInfo(mid=mid_from(params))]))
            if not result.tracks:
                raise ProviderError("unavailable", "QQ 音乐未返回该歌曲")
            song = as_dict(result.tracks[0])
            choices = quality_candidates(song)
            if method == "qualities":
                return {"items": [{k: v for k, v in item.items() if k != "media_mid"} for item in choices],
                        "requires_permission_check": True}
            return await self.resolve(song, choices, params)
        if method == "lyrics":
            mid = mid_from(params)
            try:
                result = await self.client.lyric.get_lyric(mid, trans=True, roma=True, qrc=True)
                document = lyric_document(result)
                if document is not None:
                    return document
                # Some tracks have no QRC. A successful LRC response already
                # contains the real line times; a second request adds no data.
                text = line_lyrics(result.lyric)
                if re.search(r"\[\d+:\d+", text) or "纯音乐" in text or "instrumental" in text.lower():
                    return lyric_line_document(result)
            except RatelimitedError:
                raise
            except Exception:
                pass  # Unsupported QRC must still allow ordinary LRC; never infer words.
            result = await self.client.lyric.get_lyric(mid, trans=True, roma=True, qrc=False)
            return lyric_document(result) or lyric_line_document(result)
        raise ProviderError("method_not_found", f"未实现的方法：{method}")

    async def comments(self, params: dict) -> dict:
        """Read-only public comments; never return upstream account or credential objects."""
        mid = mid_from(params)
        kind = params.get("kind", "recommend")
        page, size = int(params.get("page", 1)), int(params.get("page_size", 20))
        cursor = str(params.get("cursor", ""))
        if kind not in {"recommend", "hot", "new"} or not 1 <= page <= 1000 or not 1 <= size <= 50 or len(cursor) > 128 or (cursor and not cursor.isdecimal()):
            raise ProviderError("invalid_comments", "评论类型或分页参数无效")
        key = json.dumps([mid, kind, page, size, cursor])
        cached = self.comment_cache.get(key)
        ttl = 30 if kind == "new" else 300
        if cached and not params.get("refresh") and time.time() - cached["fetched_at"] < ttl:
            return {**cached, "cached": True, "stale": False}
        try:
            # Resolve MID ourselves; do not trust a caller-supplied numeric ID.
            song = await self.client.song.query_song([SongQueryInfo(mid=mid)])
            if not song.tracks:
                raise ProviderError("unavailable", "未找到对应歌曲的评论")
            song_id = int(as_dict(song.tracks[0])["id"])
            request = getattr(self.client.comment, f"get_{kind}_comments")
            result = await request(song_id, page_num=page, page_size=size, last_comment_seq_no=cursor)
            def item(value):
                value = as_dict(value)
                replies = value.get("sub_comments") or []
                return {"id": str(value.get("cmid", "")), "author": plain(value.get("nick"))[:120],
                        "text": plain(value.get("content")), "published_at": max(0, int(value.get("pub_time") or 0)),
                        "likes": max(0, int(value.get("praise_num") or 0)), "reply_count": max(0, int(value.get("reply_cnt") or 0)),
                        "replies": [{"id": str(reply.get("CmId", "")), "author": plain(reply.get("Nick"))[:120],
                                     "text": plain(reply.get("Content")), "published_at": max(0, int(reply.get("PubTime") or 0)),
                                     "reply_to": plain((reply.get("ParentComment") or {}).get("Nick"))[:120]}
                                    for reply in replies[:20] if isinstance(reply, dict)]}
            value = {"supported": True, "source": "qqmusic", "kind": kind, "page": page, "page_size": size,
                     "items": [item(v) for v in result.comments[:size]], "total": max(0, int(result.total)),
                     "has_more": bool(result.has_more), "next_cursor": str(result.comments[-1].seq_no) if result.comments else "",
                     "fetched_at": int(time.time()), "cached": False, "stale": False}
            # Recommendation lists sometimes report Total=0 despite returning rows.
            if value["items"] and not value["total"]:
                value.pop("total")
            value_size = len(json.dumps(value, ensure_ascii=False).encode("utf-8"))
            if value_size > 8 * 1024 * 1024:
                return value  # A large reply page must not overrun the cache budget.
            self.comment_cache.pop(key, None)
            self.comment_cache_sizes.pop(key, None)
            while self.comment_cache and (len(self.comment_cache) >= 64 or sum(self.comment_cache_sizes.values()) + value_size > 8 * 1024 * 1024):
                oldest = next(iter(self.comment_cache))
                self.comment_cache.pop(oldest)
                self.comment_cache_sizes.pop(oldest)
            self.comment_cache[key] = value
            self.comment_cache_sizes[key] = value_size
            return value
        except Exception:
            if cached:
                return {**cached, "cached": True, "stale": True, "notice": "暂时无法获取最新乐评，保留上次内容，请稍后刷新"}
            raise

    async def metadata_request(self, request):
        async with self.metadata_lock:
            while True:
                delay = await asyncio.to_thread(self.metadata_budget.reserve)
                if delay <= 0:
                    break
                await asyncio.sleep(delay)
            limited = False
            try:
                return await request()
            except Exception as exc:
                status = getattr(exc, "status_code", None) or getattr(getattr(exc, "response", None), "status_code", None)
                if isinstance(exc, RatelimitedError) or status in {403, 429}:
                    limited = True
                    raise ProviderError("metadata_rate_limited", f"匿名资料服务限制请求，已暂停 {self.metadata_budget.cooldown_seconds} 秒") from exc
                raise
            finally:
                await asyncio.to_thread(self.metadata_budget.finish, limited)

    async def ordered_singer_page(self, identity: str, page: int, sort: str, *, albums: bool) -> dict:
        order = catalog_order({"sort": sort})
        endpoint = self.metadata_client.singer.get_album_list if albums else self.metadata_client.singer.get_songs_list
        async def fetch(p):
            return as_dict(await self.metadata_request(lambda: endpoint(identity, num=30, page=p, order=order)))
        if sort != "release_old":
            return await fetch(page)
        # Reverse the entire latest-first catalogue, not the current page. At most
        # two contiguous source pages are needed for a 30-item oldest-first page.
        head = await fetch(1)
        total = max(0, int(head.get("total" if albums else "total_num") or 0))
        end = max(0, total - (page - 1) * 30); start = max(0, end - 30)
        field = "album_list" if albums else "song_list"
        rows = []
        if end > start:
            for p in range(start // 30 + 1, (end - 1) // 30 + 2):
                value = head if p == 1 else await fetch(p)
                if int(value.get("total" if albums else "total_num") or 0) != total or (value.get("singer_mid") and value["singer_mid"] != identity):
                    raise ProviderError("catalog_changed", "平台目录发生变化，请刷新后重试")
                offset = (p - 1) * 30
                rows.extend((value.get(field) or [])[max(0,start-offset):end-offset])
        return head | {field: list(reversed(rows))}

    async def artist_albums(self, params: dict) -> dict:
        artist, page = str(params.get("artist_id") or ""), int(params.get("page", 1))
        order = catalog_order(params)
        if not MID.fullmatch(artist) or not 1 <= page <= 100000:
            raise ProviderError("invalid_catalog", "歌手或页码无效")
        seed = params.get("track") or {}
        confirmed = await self.metadata({"track": seed, "section": "basic", "refresh": bool(params.get("refresh"))})
        track = confirmed.get("track") or {}
        if not confirmed.get("matched") or seed.get("source") != "qqmusic" or seed.get("id") != track.get("id") or artist not in track.get("provider_data", {}).get("artist_mids", []):
            raise ProviderError("catalog_identity_changed", "歌手身份与已确认曲目不一致")
        cache_key = f"artist-albums:{artist}:{page}:{params.get('sort', 'default')}"
        if params.get("refresh"):
            self.metadata_cache.pop(cache_key, None)
        cached = self.metadata_cache.get(cache_key)
        if cached is None:
            raw = await self.ordered_singer_page(artist, page, params.get("sort", "default"), albums=True)
            if raw.get("singer_mid") and raw["singer_mid"] != artist:
                raise ProviderError("catalog_identity_changed", "平台返回了其他歌手的专辑")
            rows = raw.get("album_list") or []
            if len(rows) > 30:
                raise ProviderError("catalog_too_large", "专辑目录分页超出限制")
            items = []
            for row in rows:
                mid = str(row.get("mid") or "")
                if not MID.fullmatch(mid):
                    raise ProviderError("catalog_identity_changed", "专辑没有有效的平台身份")
                items.append({"release_id": mid, "title": row.get("title") or row.get("name") or "", "source": "qqmusic",
                              "cover_url": f"https://y.gtimg.cn/music/photo_new/T002R300x300M000{mid}.jpg",
                              "release_date": row.get("time_public") or "", "track_count": max(0, int(row.get("total_num") or 0))})
            total = max(0, int(raw.get("total") or 0))
            cached = {"items": items, "total": total, "page": page, "next_page": page + 1 if items and page * 30 < total else None}
            if len(self.metadata_cache) >= 128:
                self.metadata_cache.clear()
            self.metadata_cache[cache_key] = cached
        return cached | {"source": "qqmusic", "artist_id": artist, "anchor": track,"sorts":["default","hot","release_new","release_old"],"sort":params.get("sort","default")}

    async def album_catalog(self, params: dict) -> dict:
        mid, page = str(params.get("release_id") or ""), int(params.get("page", 1))
        if not MID.fullmatch(mid) or not 1 <= page <= 100000:
            raise ProviderError("invalid_catalog", "专辑身份或页码无效")
        key = f"catalog:album:{mid}:{page}"
        if params.get("refresh"):
            self.metadata_cache.pop(key, None)
        cached = self.metadata_cache.get(key)
        if cached is None:
            raw = as_dict(await self.metadata_request(lambda: self.metadata_client.album.get_song(mid, num=30, page=page)))
            if raw.get("album_mid") and raw["album_mid"] != mid:
                raise ProviderError("catalog_identity_changed", "平台返回了其他专辑")
            songs = raw.get("song_list") or []
            if len(songs) > 30:
                raise ProviderError("catalog_too_large", "专辑分页超出限制")
            tracks = [track_from(song) for song in songs]
            if any(track["provider_data"].get("album_mid") != mid for track in tracks):
                raise ProviderError("catalog_identity_changed", "专辑中含有其他发行版本的歌曲")
            total = max(0, int(raw.get("total_num") or 0))
            cached = {"items": tracks, "total": total, "page": page, "next_page": page + 1 if tracks and page * 30 < total else None}
            if len(self.metadata_cache) >= 128:
                self.metadata_cache.clear()
            self.metadata_cache[key] = cached
        return cached | {"source": "qqmusic", "release_id": mid}

    async def related_catalog(self, params: dict) -> dict:
        kind, page = params.get("kind", "album"), int(params.get("page", 1))
        order = catalog_order(params)
        if kind not in {"album", "artist"} or not 1 <= page <= 100000:
            raise ProviderError("invalid_catalog", "目录分类或页码无效")
        local = params.get("track") or {}
        anchor = await self.metadata({"track": local, "section": "basic", "catalog_strict": True, "refresh": bool(params.get("refresh"))})
        cloud = anchor.get("track") or {}
        direct = str(local.get("id", "")).startswith("qqmusic:") and local.get("id") == cloud.get("id")
        if not anchor.get("matched") or not (direct or same_edition(local, cloud)):
            return {"matched": False, "source": "qqmusic", "items": [], "notice": "无法唯一确认歌曲版本，保留本地专辑。"}
        artists = [a for a in anchor.get("artists", []) if MID.fullmatch(str(a.get("mid", "")))]
        album = cloud.get("provider_data", {}).get("album_mid")
        selected = params.get("artist_mid") or (artists[0]["mid"] if len(artists) == 1 else None)
        if kind == "artist" and not selected and params.get("artist_name"):
            matches = [a for a in artists if a.get("name") == params["artist_name"]]
            if len(matches) == 1:
                selected = matches[0]["mid"]
        result = {"matched": True, "source": "qqmusic", "anchor": cloud, "artists": artists, "kind": kind,
                  "identity_basis": "provider_id" if direct else "exact_tags_duration_unique_candidate", "page": page, "items": [], "total": 0, "next_page": None}
        if kind == "artist" and selected is None:
            return result | {"notice": "请选择一位参与演唱的歌手。"}
        if kind == "album":
            if not MID.fullmatch(str(album or "")):
                raise ProviderError("catalog_unavailable", "音源没有提供可确认的专辑标识")
            identity, name = album, cloud.get("album", "")
        else:
            singer = next((a for a in artists if a["mid"] == selected), None)
            if singer is None:
                raise ProviderError("invalid_catalog", "歌手标识不属于当前歌曲")
            identity, name = selected, singer["name"]
        size = 30  # The singer endpoint caps pages at 30; its offset must use the same stride.
        key = f"catalog:{kind}:{identity}:{page}" + (f":{params.get('sort', 'default')}" if kind == "artist" else "")
        if params.get("refresh"):
            self.metadata_cache.pop(key, None)
            self.metadata_cache.pop(f"catalog-profile:{kind}:{identity}", None)
        cached = self.metadata_cache.get(key)
        if cached is None:
            fetch = (lambda: self.metadata_client.album.get_song(identity, num=size, page=page)) if kind == "album" else (lambda: self.metadata_client.singer.get_songs_list(identity, num=size, page=page, order=order))
            raw = as_dict(await self.metadata_request(fetch)) if kind == "album" else await self.ordered_singer_page(identity,page,params.get("sort","default"),albums=False)
            declared = raw.get("album_mid") if kind == "album" else raw.get("singer_mid")
            if declared and declared != identity:
                raise ProviderError("catalog_identity_changed", "平台返回了其他专辑或歌手")
            songs = raw.get("song_list") or []
            if len(songs) > size:
                raise ProviderError("catalog_too_large", "平台目录分页超出限制")
            tracks = [track_from(song) for song in songs]
            for track in tracks:
                data = track["provider_data"]
                if (kind == "album" and data.get("album_mid") != identity) or (kind == "artist" and identity not in data.get("artist_mids", [])):
                    raise ProviderError("catalog_identity_changed", "目录中含有不属于所选专辑或歌手的歌曲")
            total = max(0, int(raw.get("total_num") or 0))
            cached = {"items": tracks, "total": total, "next_page": page + 1 if tracks and page * size < total else None, "page_size": size}
            if len(self.metadata_cache) >= 128:
                self.metadata_cache.clear()
            self.metadata_cache[key] = cached
        profile_key = f"catalog-profile:{kind}:{identity}"
        profile = self.metadata_cache.get(profile_key)
        if params.get("skip_profile"):
            return result | cached | {"collection": {"id": identity, "name": name}, "profile": {"id": identity, "name": name, "source": "qqmusic"}}
        if profile is None:
            profile = {"id": identity, "name": name, "source": "qqmusic"}
            try:
                if kind == "artist":
                    detail = as_dict(await self.metadata_request(lambda: self.metadata_client.singer.get_desc([identity], photos=False)), explicit=True)
                    person = singer_person(detail, identity)
                    if person:
                        info = person.get("ex_info") or {}
                        profile["description"] = plain(info.get("desc"))
                        profile.update(singer_image(person, identity))
                        # Keep optional structured facts; never turn raw HTML into client markup.
                        for key in ("country", "genre", "birthday", "birthplace", "alias", "company", "foreign_name", "identity", "instrument", "enter", "tag", "area"):
                            value = info.get(key) or (person.get("basic_info") or {}).get(key)
                            if isinstance(value, (str, int)) and str(value).strip():
                                # Numeric category codes are not descriptive text. Their namespaces
                                # differ from singer-index enums; never guess a misleading label.
                                if key != "birthday" and re.fullmatch(r"[\d,;| ]+", str(value)):
                                    continue
                                profile[key] = plain(str(value))
                        profile["url"] = f"https://y.qq.com/n/ryqq/singer/{identity}"
                else:
                    detail = as_dict(await self.metadata_request(lambda: self.metadata_client.album.get_detail(identity)))
                    album_info = detail.get("album") or {}
                    declared = album_info.get("mid") or album_info.get("album_mid")
                    if declared and declared != identity:
                        raise ProviderError("catalog_identity_changed", "专辑资料身份不一致")
                    for key, field in (("description", "desc"), ("release_date", "time_public"), ("language", "language"), ("genre", "genre")):
                        if album_info.get(field):
                            profile[key] = plain(str(album_info[field]))
                    profile["label"] = plain((detail.get("company") or {}).get("name"))
                    profile["image_url"] = f"https://y.gtimg.cn/music/photo_new/T002R500x500M000{identity}.jpg"
                    profile["url"] = f"https://y.qq.com/n/ryqq/albumDetail/{identity}"
                if len(self.metadata_cache) >= 128:
                    self.metadata_cache.clear()
                self.metadata_cache[profile_key] = profile
            except Exception as exc:
                # Profile failure must not discard successfully validated songs. Retry on next visit.
                profile["notice"] = "平台简介暂时无法读取"
                LOG.info("Catalog profile unavailable: %s", type(exc).__name__)
        return result | cached | {"collection": {"id": identity, "name": name}, "profile": profile}

    async def metadata(self, params: dict) -> dict:
        if time.monotonic() - self.metadata_cache_at >= 86400:
            self.metadata_cache.clear()
            self.metadata_cache_at = time.monotonic()
        local = params.get("track") or {}
        mid = params.get("mid") or local.get("provider_data", {}).get("mid")
        if not mid and str(local.get("id", "")).startswith("qqmusic:"):
            mid = local["id"][8:]
        if not mid:
            query = f"{local.get('title', '')} {local.get('artist', '')}".strip()
            if not query or len(query) > 256:
                return {"matched": False, "source": "qqmusic"}
            search = await self.metadata_request(lambda: self.metadata_client.search.search_by_type(keyword=query, num=50 if params.get("catalog_strict") else 10, page=1, highlight=False))
            match = metadata_match(local, [as_dict(song) for song in search.song])
            if not match:
                return {"matched": False, "source": "qqmusic", "candidates": [track_from(as_dict(song)) for song in search.song[:5]]}
            mid = match["mid"]
        if not MID.fullmatch(str(mid)):
            raise ProviderError("invalid_track", "QQ 歌曲 MID 无效")
        section = params.get("section", "all")
        if section not in {"basic", "all"}:
            raise ProviderError("invalid_section", "资料范围须为 basic 或 all")
        key = f"song:{mid}:{section}"
        if params.get("refresh"):
            detail = self.metadata_cache.get(f"song-detail:{mid}", {}).get("track", {})
            # Refresh associated album/artist text too, rather than rebuilding from stale sections.
            self.metadata_cache.pop(f"album:{detail.get('album', {}).get('mid')}", None)
            for singer in detail.get("singer", []):
                self.metadata_cache.pop(f"artist:{singer.get('mid')}", None)
            for name in (f"song:{mid}:basic", f"song:{mid}:all", f"song-detail:{mid}"):
                self.metadata_cache.pop(name, None)
        if key in self.metadata_cache:
            return self.metadata_cache[key]
        detail_key = f"song-detail:{mid}"
        detail = self.metadata_cache.get(detail_key)
        if detail is None:
            detail = as_dict(await self.metadata_request(lambda: audio_fields(self.metadata_client.song.get_detail(mid))))
            if len(self.metadata_cache) >= 128:
                self.metadata_cache.clear()
            self.metadata_cache[detail_key] = detail
        song = detail["track"]
        track = track_from(song)
        if not track.get("cover_url") and section == "all":
            try:
                cover = await self.metadata_request(lambda: fetch_cover(mid))
                if cover:
                    track["cover_url"] = cover
                    track["provider_data"]["artwork_basis"] = "qq_mobile_share_song_mid"
            except ProviderError:
                raise
            except Exception as exc:
                LOG.info("Optional song artwork unavailable: %s", type(exc).__name__)
        def content(name: str) -> str:
            return " / ".join(plain(item.get("value")) for item in detail.get(name, []) if item.get("value"))
        background = {"description": content("intro"), "genre": content("genre"),
                      "language": content("lan"), "release_date": content("pub_time"),
                      "label": content("company"), "credits": []}
        if section == "basic":
            result = {"matched": True, "source": "qqmusic", "mid": mid, "track": track,
                      "background": background, "artists": [{"name": value.get("name"), "mid": value.get("mid")} for value in song.get("singer", [])],
                      "partial": True, "fetched_at": int(time.time()), "url": f"https://y.qq.com/n/ryqq/songDetail/{mid}"}
            self.metadata_cache[key] = result
            return result
        warnings = []
        async def optional(name: str, request, explicit=False):
            try:
                return as_dict(await self.metadata_request(request), explicit=explicit)
            except ProviderError:
                raise
            except Exception as exc:
                warnings.append(name)
                LOG.info("Optional metadata %s unavailable: %s", name, type(exc).__name__)
                return {}
        producer = await optional("credits", lambda: self.metadata_client.song.get_producer(mid))
        background["credits"] = [{"role": plain(group.get("title")),
                                   "names": [plain(p.get("name")) for p in group.get("producers", [])]}
                                  for group in producer.get("data", [])]
        album_mid = (song.get("album") or {}).get("mid")
        if album_mid:
            album_key = f"album:{album_mid}"
            album_data = self.metadata_cache.get(album_key)
            if album_data is None:
                album_data = await optional("album", lambda: self.metadata_client.album.get_detail(album_mid))
                self.metadata_cache[album_key] = album_data
            album = album_data.get("album") or {}
            for field, source_field in [("album_description", "desc"), ("release_date", "time_public"),
                                        ("language", "language"), ("genre", "genre")]:
                if album.get(source_field):
                    background[field] = plain(album[source_field])
            if (album_data.get("company") or {}).get("name"):
                background["label"] = plain(album_data["company"]["name"])
        artists = []
        for singer in song.get("singer", [])[:5]:
            artist = {"name": singer.get("name"), "mid": singer.get("mid")}
            if singer.get("mid"):
                artist_key = f"artist:{singer['mid']}"
                artist_data = self.metadata_cache.get(artist_key)
                if artist_data is None:
                    artist_data = await optional("artist", lambda: self.metadata_client.singer.get_desc([singer["mid"]], photos=False), explicit=True)
                    self.metadata_cache[artist_key] = artist_data
                if artist_data.get("singer_list"):
                    person = singer_person(artist_data, singer["mid"])
                    artist["description"] = plain((person.get("ex_info") or {}).get("desc"))
                    artist.update(singer_image(person, singer["mid"], 300))
            artists.append(artist)
        lyric = await optional("lyrics", lambda: self.metadata_client.lyric.get_lyric(mid, trans=True, qrc=False))
        result = {"matched": True, "source": "qqmusic", "mid": mid, "track": track,
                  "background": background, "artists": artists, "lyrics": lyric.get("lyric", ""),
                  "translation": lyric.get("trans", ""), "warnings": warnings,
                  "url": f"https://y.qq.com/n/ryqq/songDetail/{mid}", "fetched_at": int(time.time())}
        if len(self.metadata_cache) >= 128:
            self.metadata_cache.clear()
        self.metadata_cache[key] = result
        return result

    async def sync(self, params: dict) -> dict:
        """One bounded page per call lets the core persist progress, pause and resume."""
        sid = params.get("playlist_id")
        if sid is not None:
            return await self.playlist_tracks(params)
        collection = "collected" if params.get("collection") == "favorites" else params.get("collection", "created")
        result = await self.playlists({**params, "collection": collection})
        return {"items": [], "playlists": result["items"], "next_page": result["next_page"]}

    def account_euin(self) -> str:
        euin = self.client.credential.encrypt_uin
        if not euin:
            raise ProviderError("account_missing", "登录凭证缺少歌单账号标识，请重新扫码")
        return euin

    @staticmethod
    def page_from(params: dict) -> int:
        page = int(params.get("page", 1))
        if not 1 <= page <= 100000:
            raise ProviderError("invalid_page", "页码须为 1–100000")
        return page

    @staticmethod
    def playlist_summary(item: Any, kind: str) -> dict:
        return {"id": str(item.id), "name": item.title, "track_count": item.songnum,
                "cover_url": item.picurl or None, "source": "qqmusic", "kind": kind,
                "writable": kind == "created", "write_capabilities": ["add", "remove"] if kind == "created" else []}

    @staticmethod
    def check_playlist_result(result: Any) -> None:
        if result.code != 0 or result.subcode != 0:
            raise ProviderError("playlist_unavailable", "QQ 歌单不可访问或账号没有权限")
        if result.hasmore and not result.songs:
            raise ProviderError("incomplete_response", "QQ 返回空分页但仍标记更多歌曲，请稍后重试")

    async def playlists(self, params: dict) -> dict:
        page = self.page_from(params)
        collection = params.get("collection", "all")
        if collection not in {"all", "created", "collected"}:
            raise ProviderError("invalid_collection", "歌单类型须为 all、created 或 collected")
        entries = []
        more = False
        if collection == "all" and page == 1:
            # Liked songs are dirid=201, not the external-playlist favorites API.
            liked = await self.client.user.get_fav_song(self.account_euin(), num=1, page=1)
            self.check_playlist_result(liked)
            entries.append({"id": "liked", "name": "我喜欢", "track_count": liked.total,
                            "cover_url": liked.info.picurl or None, "source": "qqmusic", "kind": "liked", "writable": True, "write_capabilities": ["add", "remove"]})
        if collection in {"all", "created"}:
            created = await self.client.user.get_created_songlist(self.client.credential.musicid)
            # The upstream endpoint has no cursor argument. Never pretend an unfinished response is complete.
            if not created.finished:
                raise ProviderError("incomplete_response", "QQ 自建歌单响应未完成，上游接口未提供续页游标")
            values = [item for item in created.playlists if item.dirid != 201 and item.id > 0 and not item.invalid]
            entries.extend(self.playlist_summary(item, "created") for item in values[(page - 1) * 100:page * 100])
            more = page * 100 < len(values)
        if collection in {"all", "collected"}:
            collected = await self.client.user.get_fav_songlist(self.account_euin(), num=100, page=page)
            if collected.hide:
                raise ProviderError("playlist_private", "QQ 收藏歌单不可见，请检查账号权限")
            entries.extend(self.playlist_summary(item, "collected") for item in collected.playlists if item.id > 0)
            more = more or bool(collected.hasmore)
        return {"items": entries, "next_page": page + 1 if more else None}

    async def playlist_tracks(self, params: dict) -> dict:
        page = self.page_from(params)
        sid = str(params.get("playlist_id", ""))
        if sid == "liked":
            result = await audio_fields(self.client.user.get_fav_song(self.account_euin(), num=100, page=page))
        elif sid.isascii() and sid.isdecimal() and 0 < int(sid) <= 2**63 - 1:
            result = await audio_fields(self.client.songlist.get_detail(int(sid), num=100, page=page))
        else:
            raise ProviderError("invalid_playlist", "QQ 歌单 ID 无效")
        self.check_playlist_result(result)
        return {"items": [track_from(song) for song in result.songs], "playlist_id": sid, "name": "我喜欢" if sid == "liked" else getattr(getattr(result, "info", None), "title", None),
                "total": result.total, "next_page": page + 1 if result.hasmore else None}

    async def playlist_snapshot(self, params: dict) -> dict:
        sid = str(params.get("playlist_id", ""))
        tracks = []
        name = None
        for page in range(1, 102):
            result = await self.playlist_tracks({"playlist_id": sid, "page": page})
            name = name or result.get("name")
            tracks.extend(result["items"])
            if len(tracks) > 10000:
                raise ProviderError("playlist_too_large", "同步歌单最多一万首，未提交修改")
            if result["next_page"] is None:
                if len(tracks) != result["total"]:
                    raise ProviderError("incomplete_response", "歌单曲目尚未读取完整，未提交修改")
                ids = [track["id"] for track in tracks]
                revision = hashlib.sha256(json.dumps({"name": name, "track_ids": ids}, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
                snapshot = {"playlist_id": sid, "name": name, "items": tracks, "track_ids": ids, "revision": revision}
                if sid == "liked":
                    snapshot["favorite_order"] = "newest_first"
                return snapshot
        raise ProviderError("incomplete_response", "歌单分页未结束，未提交修改")

    async def playlist_apply(self, params: dict) -> dict:
        async with self.auth_lock:
            context = hashlib.sha256(f"{self.client.credential.login_type}:{self.client.credential.musicid}".encode()).hexdigest()[:24]
            if params.get("account_context") and params["account_context"] != context:
                raise ProviderError("account_changed", "音源账号已切换，未提交歌单修改")
            return await self._playlist_apply(params)

    async def _playlist_apply(self, params: dict) -> dict:
        sid = str(params.get("playlist_id", ""))
        action = params.get("action")
        ids = params.get("track_ids")
        if action not in {"add", "remove"} or not isinstance(ids, list) or not 1 <= len(ids) <= 100 or any(not isinstance(value, str) or not value.startswith("qqmusic:") or not MID.fullmatch(value[8:]) for value in ids):
            raise ProviderError("invalid_playlist_change", "每次歌单修改须包含 1–100 首本音源歌曲及 add 或 remove")
        if not isinstance(params.get("revision"), str) or not params["revision"]:
            raise ProviderError("invalid_playlist_change", "修改歌单需要已确认的云端版本")
        owned = None
        if sid != "liked":
            created = await self.client.user.get_created_songlist(self.client.credential.musicid)
            if not created.finished:
                raise ProviderError("incomplete_response", "自建歌单列表未完整，未提交修改")
            owned = next((item for item in created.playlists if str(item.id) == sid and item.dirid != 201 and not item.invalid), None)
            if owned is None:
                raise ProviderError("playlist_read_only", "只能修改当前账号的自建歌单或我喜欢，收藏歌单为只读")
        before = await self.playlist_snapshot({"playlist_id": sid})
        present = set(before["track_ids"])
        satisfied = all(value in present for value in ids) if action == "add" else all(value not in present for value in ids)
        if satisfied:
            return before
        if before["revision"] != params["revision"]:
            raise ProviderError("playlist_conflict", "云端歌单已变化，请先同步并解决冲突")
        data = await self.client.song.query_song([SongQueryInfo(mid=value[8:]) for value in ids])
        songs = {str(as_dict(value).get("mid")): as_dict(value) for value in data.tracks}
        if any(value[8:] not in songs or not songs[value[8:]].get("id") for value in ids):
            raise ProviderError("unavailable", "部分歌曲没有有效的平台编号，未提交修改")
        if any(int(songs[value[8:]].get("type", 0)) not in {0, 1} for value in ids):
            raise ProviderError("unsupported_playlist_track_type", "此写入路径仅支持普通歌曲，未修改歌单")
        # Song queries report normal music as type 1; the asset writer expects type 0.
        # In particular DelSonglist can acknowledge type 1 without removing anything.
        numeric = [(int(songs[value[8:]]["id"]), 0) for value in ids]
        if sid == "liked":
            accepted = await (self.client.songlist.like_song(numeric) if action == "add" else self.client.songlist.unlike_song(numeric))
        else:
            write = self.client.songlist.add_songs if action == "add" else self.client.songlist.del_songs
            accepted = await write(owned.dirid, numeric, tid=int(owned.id))
        if not accepted:
            raise ProviderError("playlist_write_denied", "QQ 音乐未接受歌单修改，请检查账号或平台限制")
        after = await self.playlist_snapshot({"playlist_id": sid})
        result = set(after["track_ids"])
        verified = all(value in result for value in ids) if action == "add" else all(value not in result for value in ids)
        if not verified:
            raise ProviderError("playlist_write_unconfirmed", "平台回复成功但读回未确认，保留待处理操作")
        return after

    async def recommendations(self, params: dict) -> dict:
        limit = int(params.get("limit", 20))
        excluded = params.get("exclude_track_ids", [])
        excluded_recordings = params.get("exclude_recording_ids", [])
        if not 1 <= limit <= 100 or any(not isinstance(values, list) or len(values) > 10000 or not all(isinstance(v, str) for v in values) for values in (excluded, excluded_recordings)):
            raise ProviderError("invalid_recommendation", "推荐数量须为 1–100，排除列表最多 10000 个曲目 ID")
        seen = set(excluded) | set(excluded_recordings)
        seed = params.get("seed_track_id")
        if seed and (str(seed).startswith("qqmusic:") or MID.fullmatch(str(seed))):
            seen.add(f"qqmusic:{mid_from({'track_id': seed})}")
        account = hashlib.sha256(f"{self.client.credential.login_type}:{self.client.credential.musicid}".encode()).hexdigest()
        path = self.data_dir / "recommendations.json"
        cached = None
        try:
            if path.stat().st_size <= 2 * 1024 * 1024:
                stored = json.loads(path.read_text("utf-8"))
                if stored.get("account") == account and stored.get("version") == 2:
                    cached = stored
        except (OSError, ValueError):
            pass

        async def refresh():
            result = await asyncio.wait_for(self.fetch_recommendations(), timeout=20)
            # An account may change while the upstream request is in flight.
            current = hashlib.sha256(f"{self.client.credential.login_type}:{self.client.credential.musicid}".encode()).hexdigest()
            if current == account:
                temp = path.with_suffix(".tmp")
                temp.write_text(json.dumps({"version": 2, "account": account, "day": time.strftime("%Y-%m-%d"), "updated_at": time.time(), "result": result}, ensure_ascii=False), "utf-8")
                temp.replace(path)
            return result

        if cached is None or cached.get("day") != time.strftime("%Y-%m-%d"):
            if self.recommendation_task and self.recommendation_account != account:
                self.recommendation_task.cancel()
                await asyncio.gather(self.recommendation_task, return_exceptions=True)
                self.recommendation_task = None
            if self.recommendation_task is None or self.recommendation_task.done():
                self.recommendation_account = account
                self.recommendation_task = asyncio.create_task(refresh())
                # Observe background failures and retain the last successful snapshot.
                self.recommendation_task.add_done_callback(lambda task: LOG.warning("Recommendation refresh failed: %s", type(task.exception()).__name__) if not task.cancelled() and task.exception() else None)
        if cached is None or cached.get("day") != time.strftime("%Y-%m-%d"):
            result = await self.recommendation_task
        else:
            result = cached["result"]
        current = hashlib.sha256(f"{self.client.credential.login_type}:{self.client.credential.musicid}".encode()).hexdigest()
        if current != account:
            raise ProviderError("account_changed", "音源账号已切换，请重新获取该账号的推荐")
        return {**result, "items": [track for track in result["items"] if track["id"] not in seen and track["recording_id"] not in seen][:limit]}

    async def fetch_recommendations(self) -> dict:
        async def fetch(name, request):
            try:
                return name, await asyncio.wait_for(audio_fields(request()), timeout=8)
            except Exception as exc:
                LOG.warning("QQ %s unavailable: %s", name, type(exc).__name__)
                return name, None

        results = dict(await asyncio.gather(
            fetch("guess", lambda: self.client.recommend.get_guess_recommend()),
            fetch("radar", lambda: self.client.recommend.get_radar_recommend(page=1)),
            fetch("new", lambda: self.client.recommend.get_recommend_newsong()),
            fetch("playlists", lambda: self.client.recommend.get_recommend_songlist(num=12, page=1)),
        ))
        sections, entries, seen = [], [], set()
        for key, title in (("guess", "猜你喜欢"), ("radar", "音乐雷达"), ("new", "新歌推荐")):
            response = results[key]
            if response is None:
                continue
            tracks = [track_from(song) for song in response.songs[:100]]
            tracks = [track for track in tracks if track["available"]]
            sections.append({"id": key, "title": title, "items": tracks[:24]})
            for track in tracks:
                if track["id"] not in seen:
                    seen.add(track["id"])
                    entries.append(track)
        response = results["playlists"]
        if response is not None:
            playlists = [{"id": str(item.id), "name": item.title,
                          "cover_url": item.picurl or None, "track_count": item.songnum,
                          "source": "qqmusic", "kind": "recommended", "writable": False}
                         for item in response.songlists[:12] if item.id > 0]
            sections.append({"id": "playlists", "title": "为你精选歌单", "items": [], "playlists": playlists})
        if all(response is None for response in results.values()):
            raise ProviderError("recommendations_unavailable", "QQ 推荐接口暂不可用，请稍后重试")
        return {"items": entries[:100], "sections": sections, "source": "qqmusic", "basis": "qq-personalized",
                "seed_applied": False, "upstream_methods": [name for name, result in results.items() if result is not None],
                "warnings": ["部分 QQ 推荐栏目暂不可用，已保留可用内容"] if any(result is None for result in results.values()) else []}

    async def resolve(self, song: dict, choices: list[dict], params: dict) -> dict:
        requested = params.get("quality")
        if requested == "HIRES":
            choices = [item for item in choices if item["type"] == "HIRES"]
        elif requested == "FLAC":
            choices = [item for item in choices if item["type"] == "FLAC"]
        elif requested and requested not in {"best", "native"}:
            choices = [item for item in choices if item["type"] == requested]
        if not choices:
            raise ProviderError("quality_unavailable", "该歌曲未提供所选的完整明文音质")
        infos = [SongFileInfo(mid=song["mid"], file_type=file_type(item["type"]),
                              song_type=song.get("type", 0), media_mid=item["media_mid"]) for item in choices]
        response = await self.client.song.get_song_urls(infos)
        cdn = await self.client.song.get_cdn_dispatch()
        base = next((url for url in cdn.sip if url.startswith("https://")), "https://isure.stream.qqmusic.qq.com/")
        by_filename = {info.filename: info for info in response.data}
        for choice in choices:
            kind = file_type(choice["type"])
            info = by_filename.get(f"{kind.s}{choice['media_mid']}{kind.e}")
            if info is None:
                continue
            if info.result != 0 or not info.purl:
                continue
            if info.ekey:
                continue  # An encrypted payload is never advertised as playable FLAC.
            url = urllib.parse.urljoin(base, info.purl)
            hostname = urllib.parse.urlsplit(url).hostname or ""
            if not hostname.endswith(".qq.com"):
                raise ProviderError("invalid_cdn", "音源返回了非 QQ CDN 地址")
            try:
                actual = await asyncio.to_thread(probe_url, url)
            except Exception as exc:
                LOG.warning("Audio verification failed: %s", type(exc).__name__)
                continue
            if choice["lossless"] and actual["format"] != "flac":
                continue  # Do not label an upstream lossy substitution as lossless.
            if choice["type"] == "HIRES" and not actual.get("hires"):
                continue
            if actual.get("duration") and actual["duration"] < song.get("interval", 0) * .9:
                continue
            total = actual.get("content_length")
            if total is not None and total < choice["size"] * .9:
                continue
            label = choice["label"]
            if actual.get("hires"):
                label = f"Hi-Res {actual['bit_depth']} bit / {actual['sample_rate'] / 1000:g} kHz"
            elif actual["format"] == "flac":
                label = f"FLAC {actual.get('bit_depth')} bit / {actual.get('sample_rate', 0) / 1000:g} kHz"
            pay_down = (song.get("pay") or {}).get("pay_down")
            gain = replay_gain(song) if choice["type"] != "HIRES" else None
            return {"url": url, "headers": {"Referer": "https://y.qq.com/"},
                    "format": actual["format"], "quality": choice["type"], "quality_label": label,
                    "duration": actual.get("duration") or song.get("interval"), "expected_size": total or choice["size"], "sample_rate": actual.get("sample_rate"),
                    "bit_depth": actual.get("bit_depth"), "channels": actual.get("channels"),
                    "hires": actual.get("hires", False), "verified": actual["verified"],
                    "protected": False, "preview": False, "cache_allowed": True,
                    **({"qq_replay_gain": gain} if gain else {}),
                    # Playback authorization does not prove a paid download entitlement.
                    "download_allowed": pay_down == 0, "expires_in": response.expiration,
                    "recording_id": f"qqmusic:{song['mid']}"}
        if requested == "HIRES":
            raise ProviderError("hires_unavailable", "这首歌暂时没有账号可用的 Hi-Res 版本，可在播放设置选择自动或 FLAC 无损")
        raise ProviderError("permission_or_unavailable", "账号无法取得经过校验的完整音频，请检查会员、版权区域或重新登录")

    async def close(self) -> None:
        if self.recommendation_task:
            self.recommendation_task.cancel()
            await asyncio.gather(self.recommendation_task, return_exceptions=True)
        for task in self.login_tasks.values():
            task.cancel()
        await asyncio.gather(*self.login_tasks.values(), return_exceptions=True)
        await self.metadata_client.close()


async def main() -> None:
    data_dir = Path(os.environ.get("RMUSIC_PROVIDER_DATA_DIR", Path(__file__).parent / "data"))
    provider = Provider(data_dir, anonymous=os.environ.get("RMUSIC_PROVIDER_ANONYMOUS") == "1", background=os.environ.get("RMUSIC_PROVIDER_BACKGROUND") == "1")
    async with provider.client:
        try:
            while line := await asyncio.to_thread(sys.stdin.readline):
                request_id = None
                try:
                    if len(line) > 1024 * 1024:
                        raise ProviderError("request_too_large", "RPC 请求超过 1 MiB")
                    request = json.loads(line)
                    if not isinstance(request, dict) or not isinstance(request.get("params", {}), dict):
                        raise ProviderError("invalid_request", "RPC 请求和 params 必须是对象")
                    request_id = request.get("id")
                    limits = request.get("request_limits", {})
                    if isinstance(limits, dict):
                        provider.request_limits.update(limits)
                        interval = limits.get("metadata_ms", 1100)
                        cooldown = limits.get("cooldown_seconds", 60)
                        if isinstance(interval, (int, float)) and 50 <= interval <= 60000:
                            provider.metadata_budget.interval = interval / 1000
                        if isinstance(cooldown, int) and 1 <= cooldown <= 3600:
                            provider.metadata_budget.cooldown_seconds = cooldown
                    result = await provider.dispatch(str(request["method"]), request.get("params") or {})
                    output = {"id": request_id, "result": result}
                except ProviderError as exc:
                    output = {"id": request_id, "error": {"code": exc.code, "message": str(exc)}}
                except (ValueError, KeyError, TypeError):
                    output = {"id": request_id, "error": {"code": "invalid_request", "message": "RPC 参数无效"}}
                except Exception as exc:
                    LOG.warning("Upstream request failed: %s", type(exc).__name__)
                    status = getattr(exc, "status_code", None) or getattr(getattr(exc, "response", None), "status_code", None)
                    limited = isinstance(exc, RatelimitedError) or status == 429
                    output = {"id": request_id, "error": {"code": "rate_limited" if limited else "upstream_error", "message": "QQ 音乐限制请求，已进入核心限流冷却队列" if limited else "QQ 音乐接口暂时不可用"}}
                print(json.dumps(output, ensure_ascii=False), flush=True)
        finally:
            await provider.close()


if __name__ == "__main__":
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    logging.basicConfig(stream=sys.stderr, level=logging.WARNING)
    asyncio.run(main())
