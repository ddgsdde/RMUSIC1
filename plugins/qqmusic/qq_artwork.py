"""Public, identity-bound QQ share artwork. Never retain playback URLs or cookies."""
import asyncio
import json
import re
import time
import urllib.parse
import urllib.request

LIMIT = 1024 * 1024
SHARE_HOSTS = {"i.y.qq.com", "i2.y.qq.com", "y.qq.com"}


def share_cover(document: str, mid: str) -> str | None:
    if len(document.encode("utf-8")) > LIMIT:
        return None
    match = re.search(r"window\.__ssrFirstPageData__\s*=\s*", document)
    if not match:
        return None
    try:
        encoded, _ = json.JSONDecoder().raw_decode(document[match.end():])
        value = json.loads(encoded) if isinstance(encoded, str) else encoded
        song = value["song"]
        if song.get("mid") != mid:
            return None
        image = song.get("img")
        if not isinstance(image, str) or len(image) > 2048:
            return None
        url = urllib.parse.urlsplit(image)
        if (url.scheme != "https" or url.hostname not in {"y.qq.com", "y.gtimg.cn"}
                or url.username or url.password or url.port or url.fragment
                or not re.fullmatch(r"/music/photo_new/T(?:062|002)R\d{2,4}x\d{2,4}M000[A-Za-z0-9]{5,64}\.jpg", url.path)):
            return None
        # The image MID is often DIFFERENT from the song MID. The enclosing
        # official song identity, not filename guessing or title search, binds it.
        return image
    except (ValueError, TypeError, KeyError, AttributeError):
        return None


class PublicRedirect(urllib.request.HTTPRedirectHandler):
    def __init__(self, deadline):
        self.deadline = deadline

    def redirect_request(self, request, fp, code, message, headers, newurl):
        url = urllib.parse.urlsplit(newurl)
        if time.monotonic() > self.deadline or url.scheme != "https" or url.hostname not in SHARE_HOSTS or url.port or url.username or url.password:
            raise ValueError("QQ share redirect left the public metadata origin")
        return super().redirect_request(request, fp, code, message, headers, newurl)


def _read(mid: str) -> str | None:
    if not re.fullmatch(r"[A-Za-z0-9]{5,64}", mid):
        return None
    request = urllib.request.Request("https://i.y.qq.com/v8/playsong.html?songmid=" + mid,
        headers={"User-Agent": "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/131.0 Mobile Safari/537.36"})
    # A fresh, cookie-free opener: no user account or media credentials leave here.
    deadline = time.monotonic() + 10
    with urllib.request.build_opener(PublicRedirect(deadline)).open(request, timeout=8) as response:
        if response.length is not None and response.length > LIMIT:
            return None
        body = bytearray()
        while time.monotonic() <= deadline:
            part = response.read1(min(16384, LIMIT + 1 - len(body)))
            if not part:
                break
            body.extend(part)
            if len(body) > LIMIT:
                return None
        else:
            return None
    return share_cover(body.decode("utf-8", errors="replace"), mid)


async def fetch_cover(mid: str) -> str | None:
    # Socket timeout bounds the worker; the page body is dropped after extraction.
    return await asyncio.wait_for(asyncio.to_thread(_read, mid), timeout=20)
