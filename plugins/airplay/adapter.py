"""Host-side AirPlay 1/2 adapter, using the public pyatv interface.

JSON Lines on stdin/stdout. LAN discovery stays outside the Docker network.
RAOP pause ends a stream; resuming/seeking reopens it at the saved position.
"""

from __future__ import annotations

import asyncio
import ipaddress
import json
import logging
import math
import os
from pathlib import Path
import shutil
import sys
import time
import urllib.parse
import uuid

import pyatv
from pyatv.const import DeviceState, FeatureName, FeatureState, PairingRequirement, Protocol
from pyatv.interface import MediaMetadata

LOG = logging.getLogger("rmusic.airplay")
MANIFEST = json.loads(Path(__file__).with_name("manifest.json").read_text("utf-8"))


class AdapterError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def finite_number(value, minimum: float = 0, maximum: float = 864000) -> float:
    result = float(value)
    if not math.isfinite(result) or not minimum <= result <= maximum:
        raise AdapterError("invalid_parameter", "数值超出范围")
    return result


def media_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password:
        raise AdapterError("invalid_url", "播放地址必须是无账号密码的 HTTP(S) 地址")
    return value

def fade_filters(request: dict) -> list[str]:
    transitions=request.get("transitions") or {}
    fade_in=finite_number(transitions.get("fade_in_ms",0),0,10000)/1000
    fade_out=finite_number(transitions.get("fade_out_ms",0),0,10000)/1000
    position=finite_number(request.get("position",0))
    duration=finite_number(request.get("duration") or 0)
    filters=[]
    if fade_in and position==0:
        filters.append(f"afade=t=in:d={fade_in}:curve=qsin")
    if fade_out and duration>position:
        remaining=duration-position
        filters.append(f"afade=t=out:st={max(0,remaining-fade_out)}:d={min(fade_out,remaining)}:curve=qsin")
    return filters


class ExactReader(asyncio.StreamReader):
    """pyatv's header parser needs full reads across OS pipe chunk boundaries."""

    def __init__(self, source: asyncio.StreamReader):
        super().__init__()
        self.source = source

    async def read(self, n: int = -1) -> bytes:
        if n < 0:
            return await self.source.read(n)
        try:
            return await self.source.readexactly(n)
        except asyncio.IncompleteReadError as exc:
            return exc.partial


class Adapter:
    def __init__(self, data_dir: Path):
        self.data_dir = data_dir
        data_dir.mkdir(parents=True, exist_ok=True)
        self.path = data_dir / "credentials.json"
        self.credentials = json.loads(self.path.read_text("utf-8")) if self.path.exists() else {}
        self.configs = {}
        self.connections = {}
        self.pairings = {}
        self.tasks: dict[str, asyncio.Task] = {}
        self.processes = {}
        self.states: dict[str, dict] = {}
        self.requests: dict[str, dict] = {}
        self.ffmpeg = shutil.which(os.environ.get("RMUSIC_FFMPEG", "ffmpeg"))

    def _save(self) -> None:
        temp = self.path.with_suffix(".tmp")
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(self.credentials, stream)
            stream.flush()
            os.fsync(stream.fileno())
        temp.replace(self.path)

    def _apply_credentials(self, config) -> None:
        for identifier in config.all_identifiers:
            saved = self.credentials.get(identifier, {})
            for name, value in saved.items():
                if name == "password":
                    service = config.get_service(Protocol.RAOP)
                    if service:
                        service.password = value
                    continue
                protocol = {"raop": Protocol.RAOP, "airplay": Protocol.AirPlay}.get(name)
                if protocol:
                    config.set_credentials(protocol, value)

    async def discover(self, params: dict) -> dict:
        hosts = params.get("hosts")
        if hosts is not None:
            hosts = [str(ipaddress.IPv4Address(host)) for host in hosts]
        configs = await pyatv.scan(asyncio.get_running_loop(),
                                   timeout=int(finite_number(params.get("timeout", 5), 1, 30)),
                                   protocol={Protocol.RAOP, Protocol.AirPlay}, hosts=hosts)
        items = []
        for config in configs:
            device_id = config.identifier
            if not device_id or not config.get_service(Protocol.RAOP):
                # The audio adapter requires the RAOP service. AirPlay video-only
                # devices must not appear as playable audio endpoints.
                continue
            self._apply_credentials(config)
            self.configs[device_id] = config
            services = [service for service in config.services if service.protocol in {Protocol.RAOP, Protocol.AirPlay}]
            mandatory = [service for service in services if service.pairing == PairingRequirement.Mandatory]
            items.append({"id": device_id, "name": config.name, "address": str(config.address),
                          "protocols": [service.protocol.name.lower() for service in services],
                          "paired": all(bool(service.credentials) for service in mandatory),
                          "pairing_required": bool(mandatory), "verified": False,
                          "password_required": any(service.requires_password for service in services),
                          "capabilities": {"play": True, "volume": True, "pause": True,
                                           "seek": bool(self.ffmpeg), "resume": bool(self.ffmpeg),
                                           "strict_sync": False, "dsp": False, "bit_perfect": False,
                                           "protocol_version": "negotiated_by_pyatv",
                                           "signal_path": ["pyatv decode / receiver-compatible PCM", "AirPlay negotiated transport"]}})
        return {"items": items}

    async def _config(self, device_id: str):
        if device_id not in self.configs:
            await self.discover({})
        if device_id not in self.configs:
            raise AdapterError("device_unavailable", "未发现此 AirPlay 设备，请确认处于同一局域网")
        return self.configs[device_id]

    async def _connect(self, device_id: str):
        if device_id not in self.connections:
            config = await self._config(device_id)
            self.connections[device_id] = await pyatv.connect(config, asyncio.get_running_loop())
        return self.connections[device_id]

    async def state(self, device_id: str) -> dict:
        state = self.states.setdefault(device_id, {"device_id": device_id, "status": "stopped", "position": 0,
                                                   "volume": 1.0, "ended": False})
        atv = self.connections.get(device_id)
        if atv is not None and state["status"] in {"playing", "buffering"}:
            try:
                playing = await atv.metadata.playing()
                if playing.device_state == DeviceState.Playing:
                    state["status"] = "playing"
                    if playing.position is not None:
                        state["position"] = state.get("offset", 0) + playing.position
                if playing.total_time is not None:
                    state["duration"] = state.get("offset", 0) + playing.total_time
                if atv.features.get_feature(FeatureName.Volume).state == FeatureState.Available:
                    state["volume"] = atv.audio.volume / 100
            except Exception as exc:
                LOG.debug("Device state unavailable: %s", type(exc).__name__)
        return {key: value for key, value in state.items() if key != "offset"}

    async def _stop(self, device_id: str, paused: bool = False) -> dict:
        current = self.states.setdefault(device_id, {"device_id": device_id,"position":0})
        state = dict(current)
        if paused:
            try:
                state = await asyncio.wait_for(self.state(device_id), 1)
            except asyncio.TimeoutError:
                pass
        # Set status first so completion/cancellation never becomes an ended event.
        current.update(status="paused" if paused else "stopped", ended=False, position=state["position"] if paused else 0)
        task = self.tasks.pop(device_id, None)
        if task is not None and not task.done():
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        atv = self.connections.get(device_id)
        if atv is not None:
            try:
                await asyncio.wait_for(atv.remote_control.stop(), 1)
            except Exception as exc:
                LOG.debug("Remote stop unavailable: %s", type(exc).__name__)
        return await self.state(device_id)

    async def _stream(self, device_id: str, request: dict) -> None:
        atv = self.connections[device_id]
        process = None
        state = self.states[device_id]
        try:
            source = request["url"]
            position = request.get("position", 0)
            filters=fade_filters(request)
            state["fades_applied"]=bool(filters)
            if position or filters:
                # FLAC pipes decode reliably with the pinned pyatv version; WAV
                # pipes with an indefinite data size did not pass decoder tests.
                process = await asyncio.create_subprocess_exec(
                    self.ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error",
                    "-ss", str(position), "-i", source, "-vn", *(["-af",",".join(filters)] if filters else []), "-f", "flac", "-acodec", "flac", "-compression_level", "0", "pipe:1",
                    stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
                    creationflags=0x08000000 if sys.platform == "win32" else 0,
                )
                self.processes[device_id] = process
                source = ExactReader(process.stdout)
            metadata = MediaMetadata(title=request.get("title"), artist=request.get("artist"), album=request.get("album"))
            await atv.stream.stream_file(source, metadata=metadata)
            if process is not None:
                await process.wait()
                if process.returncode:
                    raise AdapterError("decode_error", "FFmpeg 无法解码或定位此音频")
            if state["status"] in {"playing", "buffering"}:
                state.update(status="stopped", ended=True)
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            state.update(status="error", error="AirPlay 播放失败，请检查配对、音频格式或网络", ended=False)
            LOG.warning("AirPlay stream failed: %s", type(exc).__name__)
        finally:
            if process is not None and process.returncode is None:
                process.terminate()
                await process.wait()
            self.processes.pop(device_id, None)

    async def play(self, params: dict) -> dict:
        device_id = str(params["device_id"])
        if (params.get("dsp") or {}).get("enabled"):
            raise AdapterError("dsp_unsupported", "此 AirPlay 适配器未接入共享 DSP，请关闭 DSP 或选择其他输出")
        previous = self.requests.get(device_id, {})
        request = {**previous, **params}
        if "url" not in request:
            raise AdapterError("no_media", "请指定播放地址")
        request["url"] = media_url(request["url"])
        if "position" in params:
            position = finite_number(params["position"])
        elif "url" not in params:
            position = (await self.state(device_id))["position"]
        else:
            position = 0
        if position and not self.ffmpeg:
            raise AdapterError("seek_unsupported", "恢复或定位需要 FFmpeg，请配置 RMUSIC_FFMPEG")
        request["position"] = position
        if fade_filters(request) and not self.ffmpeg:
            raise AdapterError("fade_unsupported","淡入淡出需要 FFmpeg")
        await self._stop(device_id)
        await self._connect(device_id)
        state = self.states[device_id]
        state.update(status="buffering", position=position, offset=position, ended=False)
        state.pop("error", None)
        if "zone_id" in params:
            state["zone_id"] = params["zone_id"]
        self.requests[device_id] = request
        self.tasks[device_id] = asyncio.create_task(self._stream(device_id, request))
        return await self.state(device_id)

    async def dispatch(self, method: str, params: dict) -> dict:
        if method == "manifest":
            return MANIFEST
        if method == "health":
            return {"status": "ready", "upstream_version": "0.18.0", "ffmpeg": bool(self.ffmpeg),
                    "device_tested": False, "strict_sync": False}
        if method == "discover":
            return await self.discover(params)
        if method == "password":
            config = await self._config(str(params["device_id"]))
            password = str(params["password"])
            if len(password) > 256:
                raise AdapterError("invalid_parameter", "密码过长")
            for identifier in config.all_identifiers:
                self.credentials.setdefault(identifier, {})["password"] = password
            self._save()
            self._apply_credentials(config)
            await self._stop(str(params["device_id"]))
            connection = self.connections.pop(str(params["device_id"]), None)
            if connection:
                pending = connection.close()
                if pending:
                    await asyncio.gather(*pending, return_exceptions=True)
            return {"saved": True}
        if method == "pair_start":
            config = await self._config(str(params["device_id"]))
            name = str(params.get("protocol", "raop")).lower()
            protocol = {"airplay": Protocol.AirPlay, "raop": Protocol.RAOP}.get(name)
            if protocol is None:
                raise AdapterError("invalid_protocol", "配对协议应为 raop 或 airplay")
            service = config.get_service(protocol)
            if service is None:
                raise AdapterError("protocol_unavailable", "设备不支持此配对协议")
            if service.pairing == PairingRequirement.NotNeeded:
                return {"paired": True, "pin_required": False}
            if service.pairing in {PairingRequirement.Disabled, PairingRequirement.Unsupported}:
                raise AdapterError("pairing_unavailable", "设备禁用了此配对协议")
            # At most one outstanding pairing per adapter; close old sessions.
            for session in self.pairings.values():
                await session["handler"].close()
            self.pairings.clear()
            handler = await pyatv.pair(config, protocol, asyncio.get_running_loop())
            try:
                await handler.begin()
            except Exception:
                await handler.close()
                raise
            sid = uuid.uuid4().hex
            self.pairings[sid] = {"handler": handler, "config": config, "protocol": name,
                                  "created_at": time.monotonic()}
            return {"session_id": sid, "pin_required": True, "device_provides_pin": handler.device_provides_pin}
        if method == "pair_finish":
            session = self.pairings.pop(str(params["session_id"]), None)
            if session is None:
                raise AdapterError("session_missing", "配对会话不存在")
            handler = session["handler"]
            try:
                if time.monotonic() - session["created_at"] > 180:
                    raise AdapterError("session_expired", "配对会话已过期")
                pin = str(params["pin"])
                if not pin.isdecimal() or len(pin) not in {4, 6}:
                    raise AdapterError("invalid_pin", "请输入设备要求的 4 位或 6 位 PIN")
                handler.pin(pin)
                await handler.finish()
                if not handler.has_paired or not handler.service.credentials:
                    raise AdapterError("pairing_failed", "配对未成功")
                for identifier in session["config"].all_identifiers:
                    self.credentials.setdefault(identifier, {})[session["protocol"]] = handler.service.credentials
                self._save()
                self._apply_credentials(session["config"])
                return {"paired": True}
            finally:
                await handler.close()
        if method in {"play", "resume"}:
            return await self.play(params)
        if method == "seek":
            if str(params["device_id"]) not in self.requests:
                raise AdapterError("no_media", "设备没有可定位的曲目")
            return await self.play(params)
        if method in {"pause", "stop"}:
            return await self._stop(str(params["device_id"]), paused=method == "pause")
        if method == "volume":
            device_id = str(params["device_id"])
            level = finite_number(params["volume"], 0, 1)
            atv = await self._connect(device_id)
            if atv.features.get_feature(FeatureName.SetVolume).state != FeatureState.Available:
                raise AdapterError("volume_unsupported", "设备暂不支持音量控制")
            await atv.audio.set_volume(level * 100)
            state = await self.state(device_id)
            self.states[device_id]["volume"] = level
            return {**state, "volume": level}
        if method == "state":
            return await self.state(str(params["device_id"]))
        raise AdapterError("method_not_found", "未知适配器方法")

    async def close(self) -> None:
        for device_id in list(self.states):
            await self._stop(device_id)
        for connection in self.connections.values():
            pending = connection.close()
            if pending:
                await asyncio.gather(*pending, return_exceptions=True)
        for session in self.pairings.values():
            await session["handler"].close()


async def main() -> None:
    adapter = Adapter(Path(os.environ.get("RMUSIC_AIRPLAY_DATA_DIR", Path(__file__).parent / "data")))
    try:
        while line := await asyncio.to_thread(sys.stdin.readline):
            request_id = None
            try:
                if len(line) > 1024 * 1024:
                    raise AdapterError("request_too_large", "RPC 请求超过 1 MiB")
                request = json.loads(line)
                if not isinstance(request, dict) or not isinstance(request.get("params", {}), dict):
                    raise AdapterError("invalid_request", "RPC 请求和 params 必须是对象")
                request_id = request.get("id")
                result = await adapter.dispatch(request["method"], request.get("params") or {})
                output = {"id": request_id, "result": result}
            except AdapterError as exc:
                output = {"id": request_id, "error": {"code": exc.code, "message": str(exc)}}
            except (KeyError, ValueError, TypeError):
                output = {"id": request_id, "error": {"code": "invalid_request", "message": "RPC 参数无效"}}
            except Exception as exc:
                LOG.warning("AirPlay operation failed: %s", type(exc).__name__)
                output = {"id": request_id, "error": {"code": "device_error", "message": "AirPlay 操作失败，请检查设备、网络及配对状态"}}
            print(json.dumps(output, ensure_ascii=False), flush=True)
    finally:
        await adapter.close()


if __name__ == "__main__":
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    logging.basicConfig(stream=sys.stderr, level=logging.WARNING)
    asyncio.run(main())
