"""Credential-free request descriptions and strict parsers; no network or account state."""
import html
import re
from urllib.parse import urlsplit

MID = re.compile(r"[A-Za-z0-9]{1,64}\Z")


def singer_person(data, identity):
    """Bind a detail to the requested singer, including SDK-normalized field names."""
    people = data.get("singer_list") or []
    for person in people:
        basic = person.get("basic_info") or {}
        declared = person.get("singer_mid") or person.get("mid") or basic.get("singer_mid") or basic.get("mid")
        if declared == identity or (not declared and len(people) == 1):
            return person
    raise ValueError("singer identity changed or ambiguous")


def singer_image(person, identity, size=500):
    basic = person.get("basic_info") or person
    # QQ returns a shared silhouette URL even when has_photo is explicitly zero.
    # A missing flag is unknown, not evidence of absence (SDK defaults must be excluded).
    if basic.get("has_photo") in (0, "0", False):
        return {"image_url": None, "image_status": "unavailable"}
    pic = (person.get("pic") or {}).get("pic")
    if isinstance(pic, str):
        url = urlsplit(pic)
        if url.scheme in ("http", "https") and url.netloc == "y.gtimg.cn" and not url.query and not url.fragment and re.fullmatch(r"/music/photo_new/T001R\d+x\d+M000[A-Za-z0-9_-]{1,96}\.jpg", url.path):
            return {"image_url": "https://y.gtimg.cn" + url.path}
    pmid = basic.get("singer_pmid") or basic.get("pmid") or identity
    if not isinstance(pmid, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,96}", pmid):
        pmid = identity
    return {"image_url": f"https://y.gtimg.cn/music/photo_new/T001R{size}x{size}M000{pmid}.jpg"}


def artwork_identity(url):
    if not isinstance(url, str):
        return None
    parsed = urlsplit(url)
    if parsed.scheme != "https" or parsed.netloc != "y.gtimg.cn" or parsed.query or parsed.fragment:
        return None
    match = re.fullmatch(r"/music/photo_new/T001R(?:300x300|500x500)M000([A-Za-z0-9]{5,64})\.jpg", parsed.path)
    return match.group(1) if match else None


def plain(value):
    return html.unescape(re.sub(r"<[^>]*>", "", str(value or ""))).strip()[:80000]


def describe(kind, identity):
    if kind not in ("artist", "album") or not isinstance(identity, str) or not MID.fullmatch(identity):
        raise ValueError("invalid public metadata identity")
    if kind == "artist":
        module, method = "music.musichallSinger.SingerInfoInter", "GetSingerDetail"
        param = {"singer_mids": [identity], "group_singer": 1, "wiki_singer": 1, "ex_singer": 1, "pic": 1, "photos": 0}
    else:
        module, method = "music.musichallAlbum.AlbumInfoServer", "GetAlbumDetail"
        param = {"albumMId": identity}
    return {"version": 1, "url": "https://u.y.qq.com/cgi-bin/musicu.fcg", "method": "POST",
            "json": {"comm": {"ct": 24, "cv": 0, "uin": 0}, "req": {"module": module, "method": method, "param": param}}}


def parse(kind, identity, raw):
    describe(kind, identity)
    if not isinstance(raw, dict) or raw.get("code", 0) != 0 or not isinstance(raw.get("req"), dict) or raw["req"].get("code") != 0:
        raise ValueError("public metadata unavailable")
    data = raw["req"].get("data") or {}
    result = {"id": identity, "source": "qqmusic"}
    if kind == "artist":
        people = data.get("singer_list") or []
        if len(people) != 1:
            raise ValueError("ambiguous singer response")
        person = singer_person(data, identity)
        basic, extra = person.get("basic_info") or {}, person.get("ex_info") or {}
        declared = person.get("singer_mid") or person.get("mid") or basic.get("singer_mid")
        # A single anonymous request/result is the documented legacy binding; an explicit ID must agree.
        if declared and declared != identity:
            raise ValueError("singer identity changed")
        result["description"] = plain(extra.get("desc"))
        for key in ("country", "genre", "birthday", "birthplace", "alias", "company", "foreign_name", "instrument", "tag"):
            value = extra.get(key) or basic.get(key)
            if isinstance(value, (str, int)) and str(value).strip() and (key == "birthday" or not re.fullmatch(r"[\d,;| ]+", str(value))):
                result[key] = plain(value)
        result.update(singer_image(person, identity))
    else:
        album = data.get("basicInfo") or data.get("album") or {}
        if (album.get("albumMid") or album.get("mid") or album.get("album_mid")) != identity:
            raise ValueError("album identity changed")
        for key, field in (("description", "desc"), ("release_date", "publishDate"), ("language", "language"), ("genre", "genre")):
            if album.get(field):
                result[key] = plain(album[field])
        result["label"] = plain((data.get("company") or {}).get("name"))
        result["image_url"] = f"https://y.gtimg.cn/music/photo_new/T002R500x500M000{identity}.jpg"
    return result


def dispatch(params):
    kind, identity = params.get("kind"), params.get("id")
    if params.get("action") == "describe":
        return describe(kind, identity)
    if params.get("action") == "parse":
        return parse(kind, identity, params.get("response"))
    raise ValueError("invalid public metadata action")
