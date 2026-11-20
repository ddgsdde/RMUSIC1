"""Real aiohttp HTTP/WS tests; no Home Assistant installation required."""

import asyncio
import importlib.util
import unittest
from pathlib import Path

import aiohttp
from aiohttp import web

module_path = Path(__file__).parents[1] / "custom_components" / "rmusic" / "api.py"
spec = importlib.util.spec_from_file_location("rmusic_api", module_path)
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)


class URLValidation(unittest.TestCase):
    def test_prefix_preserved(self):
        self.assertEqual(api.normalize_url(" https://CORE.local:7800/music/ "), "https://core.local:7800/music")

    def test_credentials_and_query_not_accepted(self):
        for url in ("https://user:secret@core.local", "ftp://core.local", "https://core.local?ticket=secret", "https://core.local:abc"):
            with self.assertRaises(ValueError):
                api.normalize_url(url)


class ProgressEvents(unittest.TestCase):
    def test_delta_preserves_queue_and_other_zones(self):
        queue = ["one", "two"]
        zones = {"z1": {"id": "z1", "queue": queue, "position": 1, "current_track": {"id": "one"}},
                 "z2": {"id": "z2", "position": 10}}
        updated = api.merge_zone_progress(zones, {"id": "z1", "position": 2, "status": "playing", "updated_at": 123})
        self.assertEqual(updated["z1"]["position"], 2)
        self.assertEqual(updated["z1"]["current_track"], {"id": "one"})
        self.assertIs(updated["z1"]["queue"], queue)
        self.assertIs(updated["z2"], zones["z2"])
        self.assertEqual(zones["z1"]["position"], 1)

    def test_unknown_zone_requests_snapshot(self):
        self.assertIsNone(api.merge_zone_progress({}, {"id": "new", "position": 2}))
        self.assertIsNone(api.merge_zone_progress({}, None))

    def test_old_playback_delta_cannot_rewind_a_replayed_track(self):
        zones = {"z1": {"id": "z1", "playback_id": "current", "position": 2}}
        self.assertIs(api.merge_zone_progress(zones, {"id": "z1", "playback_id": "old", "position": 29}), zones)
        self.assertEqual(api.merge_zone_progress(zones, {"id": "z1", "playback_id": "current", "position": 3})["z1"]["position"], 3)


class APICommunication(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.requests = []
        self.app = web.Application()
        self.app.router.add_route("*", "/music/api/v1/{path:.*}", self.handle)
        self.runner = web.AppRunner(self.app)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, "127.0.0.1", 0)
        await self.site.start()
        port = self.site._server.sockets[0].getsockname()[1]
        self.session = aiohttp.ClientSession()
        self.client = api.RMusicClient(self.session, f"http://127.0.0.1:{port}/music", "scoped-test-token")

    async def asyncTearDown(self):
        await self.session.close()
        await self.runner.cleanup()

    async def handle(self, request):
        path = request.match_info["path"]
        self.requests.append((request.method, path, request.headers.get("Authorization"), dict(request.query)))
        if path == "events":
            self.assertEqual(request.query["ticket"], "ephemeral-event-ticket")
            ws = web.WebSocketResponse()
            await ws.prepare(request)
            await ws.send_json({"type": "zones", "data": {"items": [{"id": "z1", "status": "playing"}]}})
            await ws.close()
            return ws
        self.assertEqual(request.headers.get("Authorization"), "Bearer scoped-test-token")
        if path == "auth_error":
            return web.json_response({}, status=401)
        if path == "scope_error":
            return web.json_response({}, status=403)
        if path == "events/ticket":
            return web.json_response({"ticket": "ephemeral-event-ticket"})
        if path == "zones":
            return web.json_response({"items": [{"id": "z1", "name": "Living room"}]})
        if path == "library/tracks":
            return web.json_response({"items": [], "total": 1000000})
        if path.startswith("covers/"):
            return web.Response(body=b"png-data", content_type="image/png")
        if path.endswith("/queue"):
            return web.json_response(await request.json())
        return web.json_response({"id": "z1"})

    async def test_zone_snapshot_and_bearer_auth(self):
        self.assertEqual((await self.client.zones())["z1"]["name"], "Living room")

    async def test_revoked_and_insufficient_scope(self):
        for path in ("auth_error", "scope_error"):
            with self.assertRaises(api.RMusicAuthError):
                await self.client.request("GET", path)

    async def test_pagination_does_not_fetch_entire_library(self):
        result = await self.client.tracks(offset=100, source="qqmusic")
        self.assertEqual(result["total"], 1000000)
        self.assertEqual(self.requests[-1][3]["limit"], "100")
        self.assertEqual(self.requests[-1][3]["offset"], "100")

    async def test_websocket_ticket_and_prefix(self):
        events = [event async for event in self.client.events()]
        self.assertEqual(events[0]["type"], "zones")
        self.assertNotIn("scoped-test-token", self.requests[-1][3].values())

    async def test_insert_queue_contract(self):
        result = await self.client.queue("z1", ["track1"], append=True, insertion_index=2)
        self.assertEqual(result, {"track_ids": ["track1"], "insertion_index": 2})
        self.assertEqual(self.requests[-1][0], "POST")

    async def test_authenticated_cover_fetch_and_prefix(self):
        content, mime = await self.client.image("/api/v1/covers/track1")
        self.assertEqual((content, mime), (b"png-data", "image/png"))
        self.assertTrue(self.client.media_url("/api/v1/covers/track1").endswith("/music/api/v1/covers/track1"))


if __name__ == "__main__":
    unittest.main()
