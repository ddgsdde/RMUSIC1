"""Validate a scoped API token without sending account passwords to HA."""

from __future__ import annotations

import voluptuous as vol

from homeassistant.config_entries import ConfigFlow, ConfigFlowResult
from homeassistant.helpers.aiohttp_client import async_get_clientsession

from .api import RMusicAuthError, RMusicClient, RMusicError, normalize_url
from .const import CONF_TOKEN, CONF_URL, CONF_VERIFY_SSL, DOMAIN


def schema(defaults: dict | None = None) -> vol.Schema:
    values = defaults or {}
    return vol.Schema({vol.Required(CONF_URL, default=values.get(CONF_URL, "http://192.168.1.100:7800")): str,
                       vol.Required(CONF_TOKEN): str,
                       vol.Optional(CONF_VERIFY_SSL, default=values.get(CONF_VERIFY_SSL, True)): bool})


class RMusicConfigFlow(ConfigFlow, domain=DOMAIN):
    VERSION = 1

    async def _validate(self, data: dict) -> dict:
        result = {**data, CONF_URL: normalize_url(data[CONF_URL])}
        client = RMusicClient(async_get_clientsession(self.hass), result[CONF_URL], result[CONF_TOKEN], result.get(CONF_VERIFY_SSL, True))
        await client.zones()
        return result

    async def async_step_user(self, user_input: dict | None = None) -> ConfigFlowResult:
        errors = {}
        if user_input is not None:
            try:
                data = await self._validate(user_input)
                await self.async_set_unique_id(data[CONF_URL])
                self._abort_if_unique_id_configured()
                return self.async_create_entry(title=f"R-Music · {data[CONF_URL]}", data=data)
            except RMusicAuthError:
                errors["base"] = "invalid_auth"
            except RMusicError:
                errors["base"] = "cannot_connect"
            except ValueError:
                errors["base"] = "invalid_url"
        return self.async_show_form(step_id="user", data_schema=schema(user_input), errors=errors)

    async def async_step_reauth(self, entry_data: dict) -> ConfigFlowResult:
        return await self.async_step_reauth_confirm()

    async def async_step_reauth_confirm(self, user_input: dict | None = None) -> ConfigFlowResult:
        entry = self._get_reauth_entry()
        errors = {}
        if user_input is not None:
            try:
                data = await self._validate({**entry.data, **user_input})
                return self.async_update_reload_and_abort(entry, data_updates=data)
            except RMusicAuthError:
                errors["base"] = "invalid_auth"
            except RMusicError:
                errors["base"] = "cannot_connect"
            except ValueError:
                errors["base"] = "invalid_url"
        return self.async_show_form(step_id="reauth_confirm", data_schema=schema(entry.data), errors=errors)

    async def async_step_reconfigure(self, user_input: dict | None = None) -> ConfigFlowResult:
        entry = self._get_reconfigure_entry()
        errors = {}
        if user_input is not None:
            try:
                data = await self._validate(user_input)
                await self.async_set_unique_id(data[CONF_URL])
                self._abort_if_unique_id_mismatch(reason="wrong_core")
                return self.async_update_reload_and_abort(entry, data_updates=data)
            except RMusicAuthError:
                errors["base"] = "invalid_auth"
            except RMusicError:
                errors["base"] = "cannot_connect"
            except ValueError:
                errors["base"] = "invalid_url"
        return self.async_show_form(step_id="reconfigure", data_schema=schema(entry.data), errors=errors)
