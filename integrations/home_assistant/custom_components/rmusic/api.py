"""Small, independently testable aiohttp client for the public R-Music API."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from typing import Any
from urllib.parse import quote, urlsplit, urlunsplit

import aiohttp


class RMusicError(Exception):
    """The core could not complete a request."""


class RMusicAuthError(RMusicError):
    """A revoked, expired or insufficiently scoped token."""


def normalize_url(value: str) -> str:
    parts = urlsplit(value.strip())
    if parts.scheme not in {"http", "https"} or not parts.hostname:
        raise ValueError("Use an http:// or https:// core URL")
    if parts.username or parts.password or parts.query or parts.fragment:
        raise ValueError("The core URL must not contain credentials, query or fragment")
    # Reading port validates malformed port numbers as well.
    _ = parts.port
    path = parts.path.rstrip("/")
    return urlunsplit((parts.scheme.lower(), parts.netloc.lower(), path, "", ""))


def merge_zone_progress(zones: dict[str, dict], data: Any) -> dict[str, dict] | None:
    """Progress deltas must preserve the queue, current track and device metadata."""
    if not isinstance(data, dict) or data.get("id") not in zones:
        return None
    zone_id = data["id"]
    if "playback_id" in data and data["playback_id"] != zones[zone_id].get("playback_id"):
        return zones
    update = {key: data[key] for key in ("position", "status", "updated_at") if key in data}
    return {**zones, zone_id: {**zones[zone_id], **update}}


class RMusicClient:
    def __init__(self, session: aiohttp.ClientSession, base_url: str, token: str, verify_ssl: bool = True):
        self.session = session
        self.base_url = normalize_url(base_url)
        self.token = token
        self.verify_ssl = verify_ssl

    async def request(self, method: str, path: str, *, data: dict | None = None, params: dict | None = None) -> Any:
        try:
            async with self.session.request(
                method, f"{self.base_url}/api/v1/{path.lstrip('/')}",
                headers={"Authorization": f"Bearer {self.token}"},
                json=data, params=params, ssl=self.verify_ssl,
                timeout=aiohttp.ClientTimeout(total=15),
            ) as response:
                if response.status in {401, 403}:
                    raise RMusicAuthError("API token is expired, revoked or lacks read/control permission")
                if response.status == 204:
                    return None
                if response.status >= 400:
                    raise RMusicError(f"Core returned HTTP {response.status}")
                return await response.json()
        except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
            raise RMusicError("Unable to reach the R-Music core") from exc

    async def zones(self) -> dict[str, dict]:
        result = await self.request("GET", "zones")
        return {item["id"]: item for item in result["items"]}

    async def command(self, zone_id: str, action: str, data: dict | None = None) -> dict:
        return await self.request("POST", f"zones/{quote(zone_id, safe='')}/{action}", data=data or {})

    async def patch_zone(self, zone_id: str, data: dict) -> dict:
        return await self.request("PATCH", f"zones/{quote(zone_id, safe='')}", data=data)

    async def queue(self, zone_id: str, track_ids: list[str], append: bool = False, insertion_index: int | None = None) -> dict:
        data = {"track_ids": track_ids}
        if insertion_index is not None:
            data["insertion_index"] = insertion_index
        return await self.request("POST" if append else "PUT", f"zones/{quote(zone_id, safe='')}/queue", data=data)

    def media_url(self, url: str) -> str:
        if url.startswith(("https://", "http://")):
            return url
        if url.startswith("/api/"):
            return self.base_url + url
        if url.startswith("/"):
            parts = urlsplit(self.base_url)
            return f"{parts.scheme}://{parts.netloc}{url}"
        return f"{self.base_url}/{url}"

    async def image(self, url: str) -> tuple[bytes | None, str | None]:
        url = self.media_url(url)
        core = urlsplit(self.base_url)
        parts = urlsplit(url)
        same_core = (parts.scheme, parts.netloc) == (core.scheme, core.netloc)
        headers = {"Authorization": f"Bearer {self.token}"} if same_core else {}
        try:
            async with self.session.get(url, headers=headers, allow_redirects=False,
                                        ssl=False if same_core and not self.verify_ssl else True,
                                        timeout=aiohttp.ClientTimeout(total=10)) as response:
                if response.status != 200 or not response.content_type.startswith("image/"):
                    return None, None
                chunks = []
                size = 0
                async for chunk in response.content.iter_chunked(65536):
                    size += len(chunk)
                    if size > 8 * 1024 * 1024:
                        return None, None
                    chunks.append(chunk)
                return b"".join(chunks), response.content_type
        except (aiohttp.ClientError, asyncio.TimeoutError):
            return None, None

    async def tracks(self, *, offset: int = 0, source: str = "", q: str = "", limit: int = 100) -> dict:
        return await self.request("GET", "library/tracks", params={"offset": offset, "source": source, "q": q, "limit": limit})

    async def events(self) -> AsyncIterator[dict]:
        result = await self.request("GET", "events/ticket")
        parts = urlsplit(self.base_url)
        url = urlunsplit(("wss" if parts.scheme == "https" else "ws", parts.netloc,
                          f"{parts.path}/api/v1/events", "", ""))
        try:
            async with self.session.ws_connect(
                url, params={"ticket": result["ticket"]},
                ssl=self.verify_ssl, heartbeat=30,
                timeout=aiohttp.ClientWSTimeout(ws_close=5),
            ) as ws:
                async for message in ws:
                    if message.type == aiohttp.WSMsgType.TEXT:
                        yield message.json()
                    elif message.type == aiohttp.WSMsgType.ERROR:
                        raise RMusicError("Core event stream failed")
        except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
            raise RMusicError("Core event stream disconnected") from exc
