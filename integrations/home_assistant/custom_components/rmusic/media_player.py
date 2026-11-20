"""Expose one media_player for every independent R-Music zone."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any
from urllib.parse import quote

from homeassistant.components.media_player import BrowseMedia, MediaPlayerEntity
from homeassistant.components.media_player.const import (
    MediaClass, MediaPlayerEntityFeature, MediaPlayerEnqueue, MediaPlayerState, MediaType, RepeatMode,
)
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity
from homeassistant.util import dt as dt_util

from .api import RMusicError
from .const import DOMAIN, PAGE_SIZE
from .coordinator import RMusicCoordinator

FEATURES = (
    MediaPlayerEntityFeature.PLAY | MediaPlayerEntityFeature.PAUSE | MediaPlayerEntityFeature.STOP
    | MediaPlayerEntityFeature.PLAY_MEDIA | MediaPlayerEntityFeature.NEXT_TRACK
    | MediaPlayerEntityFeature.PREVIOUS_TRACK | MediaPlayerEntityFeature.SEEK
    | MediaPlayerEntityFeature.VOLUME_SET | MediaPlayerEntityFeature.VOLUME_MUTE
    | MediaPlayerEntityFeature.BROWSE_MEDIA | MediaPlayerEntityFeature.CLEAR_PLAYLIST
    | MediaPlayerEntityFeature.MEDIA_ENQUEUE | MediaPlayerEntityFeature.REPEAT_SET
    | MediaPlayerEntityFeature.SHUFFLE_SET
)


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback) -> None:
    coordinator: RMusicCoordinator = entry.runtime_data
    known: set[str] = set()

    @callback
    def add_zones() -> None:
        new = set(coordinator.data or {}) - known
        known.update(new)
        async_add_entities([RMusicZone(coordinator, entry, zone_id) for zone_id in sorted(new)])

    add_zones()
    entry.async_on_unload(coordinator.async_add_listener(add_zones))


class RMusicZone(CoordinatorEntity[RMusicCoordinator], MediaPlayerEntity):
    _attr_has_entity_name = True
    _attr_supported_features = FEATURES
    _attr_icon = "mdi:music-circle"
    _attr_media_content_type = MediaType.MUSIC
    _attr_media_image_remotely_accessible = False

    def __init__(self, coordinator: RMusicCoordinator, entry: ConfigEntry, zone_id: str):
        super().__init__(coordinator)
        self.zone_id = zone_id
        self._attr_unique_id = f"{entry.entry_id}_{zone_id}"
        self._attr_device_info = {"identifiers": {(DOMAIN, entry.entry_id)}, "name": entry.title,
                                  "manufacturer": "R-Music", "model": "Music Core",
                                  "configuration_url": coordinator.client.base_url}

    @property
    def zone(self) -> dict:
        return (self.coordinator.data or {}).get(self.zone_id, {})

    @property
    def track(self) -> dict:
        return self.zone.get("track") or {}

    @property
    def name(self) -> str:
        return self.zone.get("name", self.zone_id)

    @property
    def available(self) -> bool:
        return super().available and self.zone_id in (self.coordinator.data or {})

    @property
    def state(self) -> MediaPlayerState:
        return {"playing": MediaPlayerState.PLAYING, "paused": MediaPlayerState.PAUSED,
                "stopped": MediaPlayerState.IDLE}.get(self.zone.get("status"), MediaPlayerState.IDLE)

    @property
    def volume_level(self) -> float | None:
        return self.zone.get("volume")

    @property
    def is_volume_muted(self) -> bool:
        return self.zone.get("muted", False)

    @property
    def shuffle(self) -> bool:
        return self.zone.get("shuffle", False)

    @property
    def repeat(self) -> RepeatMode:
        return RepeatMode(self.zone.get("repeat", "off"))

    @property
    def media_content_id(self) -> str | None:
        return self.track.get("id")

    @property
    def media_title(self) -> str | None:
        return self.track.get("title")

    @property
    def media_artist(self) -> str | None:
        return self.track.get("artist")

    @property
    def media_album_artist(self) -> str | None:
        return self.track.get("album_artist")

    @property
    def media_album_name(self) -> str | None:
        return self.track.get("album")

    @property
    def media_track(self) -> int | None:
        return self.track.get("track_number")

    @property
    def media_duration(self) -> int | None:
        return int(self.track["duration"]) if self.track.get("duration") is not None else None

    @property
    def media_position(self) -> float:
        return self.zone.get("position", 0)

    @property
    def media_position_updated_at(self) -> datetime | None:
        value = self.zone.get("updated_at")
        if isinstance(value, (float, int)):
            return datetime.fromtimestamp(value / 1000 if value > 100000000000 else value, timezone.utc)
        return dt_util.parse_datetime(value) if isinstance(value, str) else None

    @property
    def media_image_url(self) -> str | None:
        url = self.track.get("cover_url")
        return self.coordinator.client.media_url(url) if url else None

    async def async_get_media_image(self) -> tuple[bytes | None, str | None]:
        if not self.media_image_url:
            return None, None
        return await self.coordinator.client.image(self.media_image_url)

    @property
    def extra_state_attributes(self) -> dict:
        return {"endpoint_id": self.zone.get("endpoint_id"), "queue_index": self.zone.get("queue_index", 0),
                "queue_length": len(self.zone.get("queue", [])), "source": self.track.get("source"),
                "format": self.track.get("format"), "sample_rate": self.track.get("sample_rate"),
                "bit_depth": self.track.get("bit_depth")}

    async def _command(self, action: str, data: dict | None = None) -> None:
        try:
            zone = await self.coordinator.client.command(self.zone_id, action, data)
            self.coordinator.async_set_updated_data({**self.coordinator.data, self.zone_id: zone})
        except RMusicError as exc:
            raise HomeAssistantError(str(exc)) from exc

    async def _patch(self, data: dict) -> None:
        try:
            zone = await self.coordinator.client.patch_zone(self.zone_id, data)
            self.coordinator.async_set_updated_data({**self.coordinator.data, self.zone_id: zone})
        except RMusicError as exc:
            raise HomeAssistantError(str(exc)) from exc

    async def async_media_play(self) -> None:
        await self._command("play")

    async def async_media_pause(self) -> None:
        await self._command("pause")

    async def async_media_stop(self) -> None:
        await self._command("stop")

    async def async_media_next_track(self) -> None:
        await self._command("next")

    async def async_media_previous_track(self) -> None:
        await self._command("previous")

    async def async_media_seek(self, position: float) -> None:
        await self._command("seek", {"position": position})

    async def async_set_volume_level(self, volume: float) -> None:
        await self._patch({"volume": volume})

    async def async_mute_volume(self, mute: bool) -> None:
        await self._patch({"muted": mute})

    async def async_set_repeat(self, repeat: RepeatMode) -> None:
        await self._patch({"repeat": str(repeat)})

    async def async_set_shuffle(self, shuffle: bool) -> None:
        await self._patch({"shuffle": shuffle})

    async def async_clear_playlist(self) -> None:
        try:
            zone = await self.coordinator.client.request("DELETE", f"zones/{quote(self.zone_id, safe='')}/queue")
            if isinstance(zone, dict):
                self.coordinator.async_set_updated_data({**self.coordinator.data, self.zone_id: zone})
            else:
                await self.coordinator.async_request_refresh()
        except RMusicError as exc:
            raise HomeAssistantError(str(exc)) from exc

    async def async_play_media(self, media_type: str, media_id: str,
                               enqueue: MediaPlayerEnqueue | None = None, **kwargs: Any) -> None:
        if media_type not in {MediaType.MUSIC, MediaType.TRACK, "rmusic"}:
            raise HomeAssistantError("R-Music accepts library track IDs")
        if kwargs.get("announce"):
            raise HomeAssistantError("R-Music does not yet support announcements")
        if media_id.startswith("queue:"):
            await self._command("play", {"queue_index": int(media_id.removeprefix("queue:"))})
            return
        mode = enqueue or MediaPlayerEnqueue.REPLACE
        try:
            if mode == MediaPlayerEnqueue.ADD:
                zone = await self.coordinator.client.queue(self.zone_id, [media_id], append=True)
            elif mode in {MediaPlayerEnqueue.NEXT, MediaPlayerEnqueue.PLAY}:
                index = self.zone.get("queue_index", 0) + 1 if self.zone.get("queue") else 0
                zone = await self.coordinator.client.queue(self.zone_id, [media_id], append=True, insertion_index=index)
                if mode == MediaPlayerEnqueue.PLAY:
                    zone = await self.coordinator.client.command(self.zone_id, "play", {"queue_index": index})
            else:
                zone = await self.coordinator.client.queue(self.zone_id, [media_id])
                zone = await self.coordinator.client.command(self.zone_id, "play", {"queue_index": 0})
            self.coordinator.async_set_updated_data({**self.coordinator.data, self.zone_id: zone})
        except RMusicError as exc:
            raise HomeAssistantError(str(exc)) from exc

    def _track_item(self, track: dict, media_id: str | None = None) -> BrowseMedia:
        return BrowseMedia(media_class=MediaClass.TRACK, media_content_type=MediaType.MUSIC,
                           media_content_id=media_id or track["id"],
                           title=f"{track['title']} · {track.get('artist', '')}", can_play=track.get("available", True), can_expand=False,
                           thumbnail=track.get("cover_url") if str(track.get("cover_url", "")).startswith("https://") else None)

    async def async_browse_media(self, media_content_type: str | None = None,
                                  media_content_id: str | None = None) -> BrowseMedia:
        content_id = media_content_id or "root"
        children = []
        title = "R-Music"
        if content_id == "root":
            for key, label in (("tracks:all:0", "全部音乐"), ("tracks:local:0", "本地音乐"),
                               ("tracks:qqmusic:0", "QQ 音乐"), ("queue", "播放队列")):
                children.append(BrowseMedia(media_class=MediaClass.DIRECTORY, media_content_type="rmusic_directory",
                                             media_content_id=key, title=label, can_play=False, can_expand=True))
        elif content_id == "queue":
            title = "播放队列"
            children = [self._track_item(track, f"queue:{index}") for index, track in enumerate(self.zone.get("queue", [])[:1000])]
        elif content_id.startswith("tracks:"):
            try:
                _, source, offset_text = content_id.split(":", 2)
                offset = max(int(offset_text), 0)
                if source not in {"all", "local", "qqmusic"}:
                    raise ValueError
            except ValueError as exc:
                raise HomeAssistantError("Invalid library page") from exc
            try:
                result = await self.coordinator.client.tracks(offset=offset, source="" if source == "all" else source, limit=PAGE_SIZE)
            except RMusicError as exc:
                raise HomeAssistantError(str(exc)) from exc
            title = f"音乐 · {offset + 1}–{offset + len(result['items'])} / {result['total']}"
            children = [self._track_item(track) for track in result["items"]]
            if offset + len(result["items"]) < result["total"]:
                children.append(BrowseMedia(media_class=MediaClass.DIRECTORY, media_content_type="rmusic_directory",
                                             media_content_id=f"tracks:{source}:{offset + PAGE_SIZE}", title="下一页 →",
                                             can_play=False, can_expand=True))
        else:
            raise HomeAssistantError("Unknown R-Music library directory")
        return BrowseMedia(media_class=MediaClass.DIRECTORY, media_content_type="rmusic_directory",
                           media_content_id=content_id, title=title, can_play=False, can_expand=True, children=children)
