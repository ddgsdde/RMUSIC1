"""Disposable million-track HTTP benchmark (Python 3.11+, Docker).

python scripts/benchmark.py --image rmusic-core:0.1.0
python scripts/benchmark.py --prepare-only --tracks 1000
python scripts/benchmark.py --run-fixture TEMP/rmusic-benchmark-... --image rmusic-core:0.1.0

Always creates a NEW tempfile directory and refuses an existing Docker container
named rmusic-benchmark. Never opens production data or accepts a database path.
Fixtures and reports remain for inspection; only this run's labelled container is
removed. Synthetic SQL insertion measures queries, NOT scanning or metadata APIs.
The four WAV streams measure bytes/Range transport, NOT DAC playback or DSP.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
from pathlib import Path
import random
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import threading
import time
import traceback
import urllib.parse
import urllib.request
import uuid
import wave


NAME = "rmusic-benchmark"
LABEL = "org.rmusic.disposable-benchmark"
BASE = "http://127.0.0.1:17801"
REPO = Path(__file__).resolve().parent.parent
MARKER = ".rmusic-disposable-benchmark.json"
PCM_RATE = 192000 * 2 * 3
TITLES = [
    ("夜航", "yehang", "yh", "Night Sail"),
    ("海岸线", "haianxian", "hax", "Coastline"),
    ("山与回声", "shanyuhuisheng", "syhs", "Mountain Echoes"),
    ("月光森林", "yueguangsenlin", "ygsl", "Moonlit Forest"),
    ("蓝色时刻", "lanseshike", "lssk", "Blue Hours"),
    ("雨后清晨", "yuhouqingchen", "yhqc", "Morning Rain"),
    ("星河旅人", "xinghelvren", "xhlr", "Stellar Voyage"),
    ("四季之歌", "sijizhige", "sjzg", "Four Seasons"),
]
VERSIONS = ["录音室 Studio", "重制 Remastered", "现场 Live", "原声 Acoustic"]


def docker(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(
        ["docker", *args], check=False, capture_output=True, text=True,
        encoding="utf-8", errors="replace", timeout=90,
    )
    if check and result.returncode:
        raise RuntimeError(f"Docker {args[0]} 失败（exit {result.returncode}）：{result.stderr.strip() or result.stdout.strip()}")
    return result


def inspected(identity: str) -> dict | None:
    result = docker("inspect", identity, check=False)
    return json.loads(result.stdout)[0] if result.returncode == 0 else None


def preflight(image: str) -> None:
    docker("info", "--format", "{{.ServerVersion}}")
    if inspected(NAME):
        raise RuntimeError(f"已有容器 {NAME}，拒绝覆盖；请先自行确认其用途。")
    docker("image", "inspect", image)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 17801))


def safe_fixture(root: Path, run_id: str) -> None:
    """Mutation guard: only our newly allocated, marked tempfile directory."""
    resolved = root.resolve(strict=True)
    marker = json.loads((resolved / MARKER).read_text(encoding="utf-8"))
    if (marker.get("run_id") != run_id or not resolved.name.startswith("rmusic-benchmark-")
            or resolved.parent != Path(tempfile.gettempdir()).resolve()
            or marker.get("purpose") != "DISPOSABLE SYNTHETIC BENCHMARK"):
        raise RuntimeError("测试目录所有权校验失败，停止写入。")
    if root.is_symlink() or (root / "data").is_symlink() or (root / "music").is_symlink():
        raise RuntimeError("测试目录不允许符号链接。")


def wav_fixture(path: Path, seconds: int, frequency: int) -> dict:
    period = 192000 // frequency
    frame = bytearray()
    for index in range(period):
        left = round(math.sin(2 * math.pi * index / period) * 0.25 * (2**23 - 1))
        right = -left
        frame.extend(left.to_bytes(3, "little", signed=True))
        frame.extend(right.to_bytes(3, "little", signed=True))
    block = bytes(frame) * max(1, 32768 // len(frame))
    frames_left = seconds * 192000
    with wave.open(str(path), "wb") as output:
        output.setparams((2, 3, 192000, 0, "NONE", "not compressed"))
        while frames_left:
            count = min(frames_left, len(block) // 6)
            output.writeframesraw(block[:count * 6])
            frames_left -= count
    return {"id": path.stem, "file": path.name, "size": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "sample_rate": 192000, "bit_depth": 24, "channels": 2,
            "duration": seconds}


def fixture(root: Path, run_id: str, count: int, seconds: int) -> dict:
    safe_fixture(root, run_id)
    data, music = root / "data", root / "music"
    data.mkdir()
    music.mkdir()
    wavs = [wav_fixture(music / f"pcm-{i}.wav", seconds, frequency)
            for i, frequency in enumerate([300, 400, 600, 800])]
    schema = (REPO / "crates/core/src/schema.sql").read_text(encoding="utf-8")
    started = time.perf_counter()
    connection = sqlite3.connect(data / "library.db")
    connection.executescript(schema)
    connection.execute("PRAGMA cache_size=-65536")
    connection.execute("INSERT INTO settings(key,value) VALUES('name',?)",
                       (json.dumps("DISPOSABLE BENCHMARK", ensure_ascii=False),))
    insert = """INSERT INTO tracks
        (id,path,title,artist,album,album_artist,duration,track_number,disc_number,
         source,format,sample_rate,bit_depth,channels,bitrate,recording_id,
         available,created_at,provider_data,album_id,artist_id,size)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"""
    now = int(time.time())
    for offset in range(0, count, 5000):
        tracks, searches, recordings, releases = [], [], [], []
        for index in range(offset, min(count, offset + 5000)):
            cn, roman, initials, english = TITLES[index % len(TITLES)]
            version = VERSIONS[(index // len(TITLES)) % len(VERSIONS)]
            number = index // (len(TITLES) * len(VERSIONS))
            track_id = f"fixture-{index:07d}"
            artist_number = index % 5000
            artist = f"测试艺术家 Artist {artist_number:04d}"
            album = f"基准合成专辑 Album {index // 10:06d}"
            title = f"{cn} {english} {number:05d} · {version}"
            album_id, artist_id = f"album-{index // 10:06d}", f"artist-{artist_number:04d}"
            recording = f"recording-{index:07d}"
            # Deliberately 'benchmark', never claim nonexistent local audio files.
            tracks.append((track_id, None, title, artist, album, artist, 180.0,
                           index % 10 + 1, 1, "benchmark", "metadata", 0, 0, 0, 0,
                           recording, 1, now, '{"synthetic_benchmark":true}',
                           album_id, artist_id, 0))
            # Precomputed tokens from a fixed, explicit vocabulary; not scanner output.
            tokens = f"{title} {artist} {album} {cn} {roman} {initials} "
            tokens += " ".join(cn) + " " + " ".join(cn[j:j+2] for j in range(len(cn)-1))
            searches.append((index + 1, track_id, tokens.lower()))
            recordings.append((recording, title, version))
            releases.append((album_id, album, artist))
        with connection:
            connection.executemany(insert, tracks)
            connection.executemany("INSERT INTO track_search(rowid,track_id,tokens) VALUES(?,?,?)", searches)
            connection.executemany("INSERT INTO recordings(id,title,version) VALUES(?,?,?)", recordings)
            connection.executemany("INSERT OR IGNORE INTO releases(id,title,artist) VALUES(?,?,?)", releases)
        if offset % 100000 == 0:
            print(f"合成 metadata/FTS: {min(count, offset + 5000):,}/{count:,}", flush=True)
    for item in wavs:
        values = (item["id"], f"/music/{item['file']}", f"合成 PCM 24/192 {item['id']}",
                  "Benchmark", "Transport fixtures", "Benchmark", seconds, 1, 1,
                  "local", "wav", 192000, 24, 2, PCM_RATE * 8, item["id"], 1,
                  now, '{"synthetic_benchmark":true}', "pcm-album", "pcm-artist", item["size"])
        connection.execute(insert, values)
        rowid = connection.execute("SELECT rowid FROM tracks WHERE id=?", (item["id"],)).fetchone()[0]
        connection.execute("INSERT INTO track_search(rowid,track_id,tokens) VALUES(?,?,?)",
                           (rowid, item["id"], "synthetic pcm benchmark transport"))
    connection.commit()
    connection.execute("INSERT INTO track_search(track_search) VALUES('optimize')")
    connection.execute("ANALYZE")
    connection.commit()
    plans = {}
    queries = {
        "list_count": ("SELECT COUNT(*) FROM tracks t WHERE t.available=1 AND (?1='' OR t.source=?1) AND (?2='' OR t.album_id=?2) AND (?3='' OR t.artist_id=?3)", ("", "", "")),
        "deep_offset": ("SELECT t.* FROM tracks t WHERE t.available=1 AND (?1='' OR t.source=?1) AND (?2='' OR t.album_id=?2) AND (?3='' OR t.artist_id=?3) ORDER BY t.title,t.id LIMIT ?4 OFFSET ?5", ("", "", "", 100, max(0, count - 100))),
        "search_count": ("SELECT COUNT(*) FROM track_search JOIN tracks t ON t.rowid=track_search.rowid WHERE t.available=1 AND track_search MATCH ?1", ('"yehang"*',)),
    }
    for name, (sql, parameters) in queries.items():
        plans[name] = [list(row) for row in connection.execute("EXPLAIN QUERY PLAN " + sql, parameters)]
    checks = {"tracks": connection.execute("SELECT COUNT(*) FROM tracks").fetchone()[0],
              "fts": connection.execute("SELECT COUNT(*) FROM track_search").fetchone()[0],
              "roman_matches": connection.execute("SELECT COUNT(*) FROM track_search WHERE track_search MATCH '\"yehang\"*'").fetchone()[0]}
    connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    connection.close()
    # Only this new fixture, needed for the image's non-root UID 10001 on Linux.
    data.chmod(0o777)
    music.chmod(0o755)
    for item in data.iterdir():
        if item.is_file():
            item.chmod(0o666)
    return {"kind": "direct SQL synthetic metadata + precomputed FTS tokens; NOT scanner throughput",
            "count": count, "validation": checks, "generation_seconds": time.perf_counter() - started,
            "database_bytes": (data / "library.db").stat().st_size,
            "pcm": wavs, "query_plans": plans}


def request(path: str, token: str = "", body: dict | None = None,
            headers: dict | None = None) -> tuple[object, float]:
    hdr = {"Accept": "application/json", **(headers or {})}
    if token:
        hdr["Authorization"] = "Bearer " + token
    encoded = None
    if body is not None:
        hdr["Content-Type"] = "application/json"
        encoded = json.dumps(body).encode()
    started = time.perf_counter()
    with urllib.request.urlopen(urllib.request.Request(BASE + path, data=encoded, headers=hdr), timeout=60) as response:
        result = json.load(response)
    return result, (time.perf_counter() - started) * 1000


def timings(values: list[float]) -> dict:
    ordered = sorted(values)
    return {"count": len(values), "p50_ms": round(statistics.median(values), 2),
            "p95_ms": round(ordered[max(0, math.ceil(len(values) * 0.95) - 1)], 2),
            "max_ms": round(max(values), 2), "samples_ms": [round(v, 2) for v in values]}


def resource_sample(identity: str) -> dict:
    stats = json.loads(docker("stats", "--no-stream", "--format", "{{json .}}", identity).stdout)
    # Container working set is not process RSS. Record both and label correctly.
    program = """import os,json,pathlib
rows=[]
for p in pathlib.Path('/proc').glob('[0-9]*'):
 if p.name==str(os.getpid()): continue
 try:
  lines=(p/'status').read_text().splitlines(); fields=dict(x.split(':',1) for x in lines if ':' in x)
  command=[v.decode('utf-8','replace') for v in (p/'cmdline').read_bytes().split(b'\\0') if v]
  rows.append({'pid':int(p.name),'name':fields.get('Name','').strip(),'command':command,'rss_bytes':int(fields.get('VmRSS','0 kB').split()[0])*1024})
 except (OSError,ValueError): pass
print(json.dumps({'processes':rows,'rss_sum_bytes':sum(x['rss_bytes'] for x in rows)}))"""
    proc = json.loads(docker("exec", identity, "python3", "-c", program).stdout)
    return {"at": time.time(), "docker_stats": stats, **proc}


def idle_with_qqmusic(identity: str, token: str) -> tuple[dict, dict]:
    """Start the real QQ process via local unauthenticated health, then sample RSS.

    A new labelled benchmark volume contains no provider credentials. This health
    method only inspects local credential state; it does not call QQ services.
    Report a missing/failed provider explicitly instead of calling core-only RSS
    a core+plugin measurement.
    """
    provider = {"id": "qqmusic", "endpoint": "/api/v1/plugins/qqmusic/health",
                "network_scope": "local health only; fresh volume without QQ credentials"}
    try:
        result, elapsed = request(provider["endpoint"], token)
        provider.update(status=result.get("status", "unknown"), response=result,
                        elapsed_ms=round(elapsed, 2))
    except Exception as error:
        provider.update(status="failed", error=str(error))
    sample = resource_sample(identity)
    qq = [p for p in sample["processes"]
          if any(str(arg).replace("\\", "/").endswith("/qqmusic/provider.py")
                 for arg in p.get("command", []))]
    core = [p for p in sample["processes"] if p["name"] == "rmusic-core"]
    provider["process_ids"] = [p["pid"] for p in qq]
    provider["included_in_idle_rss"] = provider["status"] == "ready" and bool(qq) and bool(core)
    sample["core_and_qq_rss_bytes"] = (sum(p["rss_bytes"] for p in core + qq)
                                        if provider["included_in_idle_rss"] else None)
    sample["measurement_scope"] = ("core + started QQ plugin (no logged-in account)"
                                    if provider["included_in_idle_rss"]
                                    else "QQ plugin inclusion NOT verified; inspect idle_provider")
    return provider, sample


def range_read(url: str, start: int, end: int) -> bytes:
    request_ = urllib.request.Request(url, headers={"Range": f"bytes={start}-{end}"})
    with urllib.request.urlopen(request_, timeout=30) as response:
        if response.status != 206 or response.headers.get("Content-Range", "") != f"bytes {start}-{end}/{response.headers.get('Content-Range','').rsplit('/',1)[-1]}":
            raise RuntimeError("Range 响应没有正确返回 206/Content-Range")
        content = response.read()
        if len(content) != end - start + 1:
            raise RuntimeError("Range 字节长度不符")
        return content


def streams(identity: str, token: str, wavs: list[dict], seconds: int) -> dict:
    urls = []
    for item in wavs:
        ticket, _ = request(f"/api/v1/library/tracks/{item['id']}/ticket", token)
        urls.append(urllib.parse.urljoin(BASE, ticket["url"]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        started = time.perf_counter()
        bodies = list(pool.map(lambda args: range_read(*args),
                               [(url, 0, item["size"] - 1) for url, item in zip(urls, wavs)]))
        burst_seconds = time.perf_counter() - started
    for body, item in zip(bodies, wavs):
        if hashlib.sha256(body).hexdigest() != item["sha256"]:
            raise RuntimeError("原样 Range 返回与已知 PCM 文件 SHA256 不同")
    burst_bytes = sum(len(body) for body in bodies)
    del bodies
    samples, errors = [], []
    stopping = threading.Event()

    def sampler():
        while not stopping.is_set():
            try:
                samples.append(resource_sample(identity))
            except Exception as error:
                errors.append(str(error))
            stopping.wait(2)

    sampling = threading.Thread(target=sampler, daemon=True)
    sampling.start()
    barrier = threading.Barrier(4)

    def paced(index: int) -> dict:
        barrier.wait(timeout=30)
        started = time.perf_counter()
        position, count, late = 44, 0, 0
        durations = []
        chunk = PCM_RATE // 4  # Quarter-second stereo PCM payload.
        while time.perf_counter() - started < seconds:
            size = min(chunk, wavs[index]["size"] - position)
            clock = time.perf_counter()
            range_read(urls[index], position, position + size - 1)
            durations.append((time.perf_counter() - clock) * 1000)
            count += size
            position += size
            if position >= wavs[index]["size"]:
                position = 44
            next_read = started + count / PCM_RATE
            delay = next_read - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            else:
                late += 1
        elapsed = time.perf_counter() - started
        return {"id": wavs[index]["id"], "bytes": count, "seconds": elapsed,
                "mib_per_second": count / elapsed / 2**20, "late_reads": late,
                "range_latency": timings(durations)}

    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(paced, range(4)))
    finally:
        stopping.set()
        sampling.join(timeout=90)
    return {"kind": "4 concurrent HTTP Range streams paced at stereo PCM24/192; no decoder/DAC/stability claim",
            "burst_bytes": burst_bytes, "burst_seconds": burst_seconds,
            "burst_mib_per_second": burst_bytes / burst_seconds / 2**20,
            "byte_identical_sha256": True, "streams": results,
            "resources": samples, "resource_errors": errors,
            "peak_process_rss_bytes": max((s["rss_sum_bytes"] for s in samples), default=None)}


def run(root: Path, run_id: str, image: str, metadata: dict, seconds: int, report: dict) -> None:
    safe_fixture(root, run_id)
    preflight(image)
    container_id = ""
    volume_name = f"{NAME}-{uuid.uuid4().hex}"
    helper_name = f"{NAME}-copy-{uuid.uuid4().hex}"
    volume_created = False
    try:
        docker("volume", "create", "--label", f"{LABEL}={run_id}", volume_name)
        volume_created = True
        # SQLite must use Docker-local storage, not Docker Desktop's Windows bind mount.
        docker("run", "--rm", "--name", helper_name, "--label", f"{LABEL}={run_id}",
               "--user", "0:0", "--network", "none", "--cap-drop", "ALL",
               "--cap-add", "CHOWN", "--cap-add", "DAC_OVERRIDE", "--security-opt", "no-new-privileges:true",
               "--mount", f"type=bind,source={root / 'data'},target=/fixture,readonly",
               "--mount", f"type=volume,source={volume_name},target=/data",
               "--entrypoint", "python3", image, "-c",
               "import pathlib,shutil,os; p=pathlib.Path('/data'); "
               "assert not any(p.iterdir()), 'benchmark volume must be empty'; "
               "shutil.copyfile('/fixture/library.db','/data/library.db'); "
               "os.chown('/data',10001,10001); os.chown('/data/library.db',10001,10001)")
        container_id = docker(
            "run", "--detach", "--name", NAME, "--label", f"{LABEL}={run_id}",
            "--cpus", "4", "--memory", "4g", "--pids-limit", "128",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
            "--read-only", "--tmpfs", "/tmp:rw,size=268435456,mode=1777",
            "--publish", "127.0.0.1:17801:7800",
            "--mount", f"type=volume,source={volume_name},target=/data",
            "--mount", f"type=bind,source={root / 'music'},target=/music,readonly",
            "--env", "RMUSIC_PUBLIC_URL=http://127.0.0.1:17801", image,
        ).stdout.strip()
        instance = inspected(container_id)
        report["container"] = {"id": container_id, "name": NAME, "image": image,
                               "image_id": instance["Image"], "nano_cpus": instance["HostConfig"]["NanoCpus"],
                               "memory_limit": instance["HostConfig"]["Memory"], "port": 17801,
                               "data_storage": "Docker-local disposable named volume", "volume": volume_name}
        for _ in range(90):
            try:
                status, _ = request("/api/v1/status")
                break
            except (OSError, ValueError):
                time.sleep(1)
        else:
            raise RuntimeError("独立测试核心未在 90 秒内启动")
        credentials = root / ".benchmark-auth.json"
        if status.get("initialized"):
            if not credentials.exists():
                raise RuntimeError("fixture 已初始化但无本脚本的独立凭证，拒绝继续")
            auth, _ = request("/api/v1/auth/login", body=json.loads(credentials.read_text()))
        else:
            login = {"username": "benchmark", "password": uuid.uuid4().hex + uuid.uuid4().hex}
            credentials.write_text(json.dumps(login), encoding="utf-8")
            credentials.chmod(0o600)
            bootstrap = docker("exec", container_id, "python3", "-c", "print(open('/data/bootstrap.token').read().strip())").stdout.strip()
            auth, _ = request("/api/v1/setup", body={"bootstrap_token": bootstrap, **login})
        token = auth["token"]
        report["core_version"] = status.get("version")
        report["idle_provider"], report["idle_resources"] = idle_with_qqmusic(container_id, token)
        if not report["idle_provider"]["included_in_idle_rss"]:
            report["limitations"].append("Idle RSS does not verify core+QQ plugin: plugin health/process failed; see idle_provider.")
        print(f"QQ idle health: {report['idle_provider']['status']}; actual process included={report['idle_provider']['included_in_idle_rss']}", flush=True)
        rng = random.Random(20261012)
        queries = [(kind, row[index]) for row in TITLES for kind, index in [("chinese", 0), ("pinyin", 1), ("initials", 2)]]
        for _, query in queries:  # Fixed vocabulary warm-up; timed requests come afterwards.
            request("/api/v1/library/tracks?" + urllib.parse.urlencode({"q": query, "limit": 100}), token)
        searches, by_kind = [], {"chinese": [], "pinyin": [], "initials": []}
        for index in range(100):
            kind, query = rng.choice(queries)
            value, elapsed = request("/api/v1/library/tracks?" + urllib.parse.urlencode({"q": query, "limit": 100}), token)
            if not value.get("total") or not value.get("items"):
                raise RuntimeError(f"真实HTTP搜索未匹配合成已知词 {query}")
            searches.append(elapsed)
            by_kind[kind].append(elapsed)
            if (index + 1) % 10 == 0:
                print(f"HTTP 暖搜索: {index + 1}/100", flush=True)
        report["warm_search"] = {"overall": timings(searches), "by_kind": {key: timings(values) for key, values in by_kind.items()}, "target_p95_ms": 500}
        page_times, pages = [], []
        offsets = [0, 100, min(10000, metadata["count"] - 100), max(0, metadata["count"] - 100)]
        offsets += [rng.randrange(max(1, metadata["count"] - 100)) for _ in range(46)]
        for offset in offsets:
            value, elapsed = request(f"/api/v1/library/tracks?limit=100&offset={offset}", token)
            if value.get("total") != metadata["count"] + 4 or not value.get("items"):
                raise RuntimeError("HTTP 分页记录总数与fixture不一致")
            page_times.append(elapsed)
            pages.append({"offset": offset, "milliseconds": round(elapsed, 2)})
        report["pagination"] = {**timings(page_times), "pages": pages, "target_p95_ms": 300}
        print("HTTP 四路 PCM24/192 Range 传输与资源采样", flush=True)
        report["audio_transport"] = streams(container_id, token, metadata["pcm"], seconds)
    finally:
        if container_id:
            current = inspected(container_id)
            if current and current["Config"].get("Labels", {}).get(LABEL) == run_id:
                mounts = current.get("Mounts", [])
                data_mount = next((m for m in mounts if m["Destination"] == "/data"), {})
                matches = data_mount.get("Type") == "volume" and data_mount.get("Name") == volume_name
                if not matches:
                    report["cleanup_error"] = "label匹配但挂载路径校验失败；保留容器供人工检查"
                else:
                    logs = docker("logs", "--tail", "200", container_id, check=False)
                    (root / "container.log").write_text(logs.stdout + logs.stderr, encoding="utf-8")
                    docker("rm", "--force", container_id)
                    report["container_removed"] = True
            elif current:
                report["cleanup_error"] = "容器身份label不匹配；拒绝移除"
        helper = inspected(helper_name)
        if helper and helper["Config"].get("Labels", {}).get(LABEL) == run_id:
            docker("rm", "--force", helper["Id"])
        if volume_created:
            found = docker("volume", "inspect", volume_name, check=False)
            if found.returncode == 0:
                volume = json.loads(found.stdout)[0]
                if volume.get("Name") == volume_name and (volume.get("Labels") or {}).get(LABEL) == run_id:
                    cleanup = docker("volume", "rm", volume_name, check=False)
                    report["volume_removed"] = cleanup.returncode == 0
                    if cleanup.returncode:
                        report["volume_cleanup_error"] = cleanup.stderr
                else:
                    report["volume_cleanup_error"] = "volume身份label不匹配；拒绝移除"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--image", default="rmusic-core:0.1.0")
    parser.add_argument("--tracks", type=int, default=1000000, help="合成metadata数量（默认百万）")
    parser.add_argument("--stream-seconds", type=int, default=30, help="四路传输持续时间；不是24h稳定性验收")
    parser.add_argument("--wave-seconds", type=int, default=8)
    parser.add_argument("--prepare-only", action="store_true", help="只生成新fixture，不创建Docker容器")
    parser.add_argument("--run-fixture", type=Path, help="复测本脚本生成的已标记Temp目录（不重新生成）")
    args = parser.parse_args()
    if not 100 <= args.tracks <= 10000000 or not 1 <= args.stream_seconds <= 86400 or not 1 <= args.wave_seconds <= 60:
        parser.error("tracks需100..10000000，stream-seconds需1..86400，wave-seconds需1..60")
    if args.prepare_only and args.run_fixture:
        parser.error("prepare-only 与 run-fixture 不能同时使用")
    if not args.prepare_only:
        preflight(args.image)
    if args.run_fixture:
        root = args.run_fixture.resolve(strict=True)
        run_id = json.loads((root / MARKER).read_text())["run_id"]
        safe_fixture(root, run_id)
        print(f"复用已校验的独立fixture：{root}", flush=True)
    else:
        run_id = str(uuid.uuid4())
        root = Path(tempfile.mkdtemp(prefix="rmusic-benchmark-"))
        root.chmod(0o755)
        (root / MARKER).write_text(json.dumps({"run_id": run_id, "purpose": "DISPOSABLE SYNTHETIC BENCHMARK"}), encoding="utf-8")
        print(f"新建独立测试目录（保留，不自动删除）：{root}", flush=True)
    report = {"run_id": run_id, "fixture_root": str(root), "started_at": time.time(),
              "limitations": ["Synthetic SQL/FTS fixtures are not scan throughput or external metadata enrichment.",
                              "Fixture query_plans were captured during generation and may not describe the current core SQL.",
                              "Loopback HTTP timings include Python client overhead; no NAS/device claim.",
                              "PCM transport is not decoder, DSP, DAC output, gapless, synchronization, or 24h stability.",
                              "Docker working set and process RSS are separately labelled."]}
    code = 0
    try:
        if args.run_fixture:
            report["fixture"] = json.loads((root / "fixture.json").read_text(encoding="utf-8"))
        else:
            report["fixture"] = fixture(root, run_id, args.tracks, args.wave_seconds)
            (root / "fixture.json").write_text(json.dumps(report["fixture"], ensure_ascii=False, indent=2), encoding="utf-8")
        if not args.prepare_only:
            run(root, run_id, args.image, report["fixture"], args.stream_seconds, report)
    except Exception as error:
        report["error"] = str(error)
        report["traceback"] = traceback.format_exc()
        print(f"基准失败：{error}", flush=True)
        code = 1
    finally:
        safe_fixture(root, run_id)
        report["finished_at"] = time.time()
        serialized = json.dumps(report, ensure_ascii=False, indent=2)
        (root / "report.json").write_text(serialized, encoding="utf-8")
        (root / f"report-{time.time_ns()}.json").write_text(serialized, encoding="utf-8")
        print(f"报告：{root / 'report.json'}", flush=True)
    if "warm_search" in report and "pagination" in report:
        print(f"搜索 P95：{report['warm_search']['overall']['p95_ms']}ms；分页 P95：{report['pagination']['p95_ms']}ms")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
