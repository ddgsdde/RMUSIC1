"""Zone state with push updates, reconnect and a bounded polling fallback."""

from __future__ import annotations

import asyncio
import logging
from datetime import timedelta

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import ConfigEntryAuthFailed
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .api import RMusicAuthError, RMusicClient, RMusicError, merge_zone_progress

LOGGER = logging.getLogger(__name__)


class RMusicCoordinator(DataUpdateCoordinator[dict[str, dict]]):
    def __init__(self, hass: HomeAssistant, entry: ConfigEntry, client: RMusicClient):
        super().__init__(hass, LOGGER, config_entry=entry, name="R-Music", update_interval=timedelta(seconds=30))
        self.client = client
        self.entry = entry
        self._event_task: asyncio.Task | None = None

    async def _async_update_data(self) -> dict[str, dict]:
        try:
            return await self.client.zones()
        except RMusicAuthError as exc:
            raise ConfigEntryAuthFailed(str(exc)) from exc
        except RMusicError as exc:
            raise UpdateFailed(str(exc)) from exc

    def start(self) -> None:
        self._event_task = self.entry.async_create_background_task(self.hass, self._listen(), "R-Music events")

    async def _listen(self) -> None:
        delay = 1
        while True:
            try:
                # Fetch a snapshot on each reconnect to cover events missed offline.
                await self.async_refresh()
                async for event in self.client.events():
                    delay = 1
                    if not isinstance(event, dict):
                        continue
                    if event.get("type") == "zones":
                        data = event.get("data")
                        if isinstance(data, list):
                            self.async_set_updated_data({item["id"]: item for item in data})
                        elif isinstance(data, dict) and "items" in data:
                            self.async_set_updated_data({item["id"]: item for item in data["items"]})
                        elif isinstance(data, dict) and "id" in data:
                            self.async_set_updated_data({**(self.data or {}), data["id"]: data})
                        else:
                            await self.async_request_refresh()
                    elif event.get("type") == "zone_progress":
                        merged = merge_zone_progress(self.data or {}, event.get("data"))
                        if merged is None:
                            await self.async_request_refresh()
                        else:
                            self.async_set_updated_data(merged)
            except asyncio.CancelledError:
                raise
            except RMusicAuthError:
                self.entry.async_start_reauth(self.hass)
                return
            except (RMusicError, ValueError, KeyError):
                self.async_set_update_error(UpdateFailed("Core event connection lost"))
                LOGGER.debug("R-Music event connection lost; reconnect in %s seconds", delay)
            await asyncio.sleep(delay)
            delay = min(delay * 2, 60)

    async def async_stop(self) -> None:
        if self._event_task:
            self._event_task.cancel()
            await asyncio.gather(self._event_task, return_exceptions=True)
