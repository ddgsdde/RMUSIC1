#!/usr/bin/env python3
"""Offline R-Music plugin scaffold, bounded RPC contract checks and packaging."""
import argparse
import json
import pathlib
import re
import selectors
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile

ASSETS = pathlib.Path(__file__).resolve().parent
MAX_REPLY = 128 * 1024
ID = re.compile(r"^[a-z][a-z0-9_-]{0,63}$")


def validate(manifest):
    if not ID.fullmatch(manifest.get("id", "")):
        raise ValueError("invalid plugin id")
    if manifest.get("api_version") != "1" or manifest.get("entrypoint") != "provider.py":
        raise ValueError("expected API 1 and provider.py")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[\w.-]+)?", manifest.get("version", "")):
        raise ValueError("expected SemVer version")
    if not manifest.get("core_api"):
        raise ValueError("core_api is required")
    if manifest.get("kind", "source") == "extension":
        if manifest.get("extension_api") != "1" or len(manifest.get("extensions", [])) > 32:
            raise ValueError("invalid extension contract")
        methods = set()
        for method in manifest.get("extensions", []):
            name = method.get("method")
            if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name) or name in methods:
                raise ValueError("invalid or duplicate method")
            methods.add(name)
            if method.get("scope") not in ("read", "library", "control", "admin") or method.get("effect") not in ("none", "proposal"):
                raise ValueError("invalid scope or effect")
            if method.get("scope") == "read" and method.get("effect") == "proposal":
                raise ValueError("read methods cannot propose mutations")
            for key in ("input", "output"):
                schema(method[key])


SOURCE_METHODS = {name: name for name in ("search", "resolve", "qualities", "metadata", "lyrics", "playlists", "playlist_tracks", "liked_songs", "recommendations")}
SOURCE_METHODS.update({"public_metadata_v1": "public_metadata", "artwork_probe_v1": "artwork_probe", "related_catalog_v1": "related_catalog", "artist_albums_v1": "artist_albums", "album_catalog_v1": "album_catalog", "comments_read_v1": "comments"})


def source_contract(manifest, report=None):
    if manifest.get("kind", "source") == "extension":
        return
    capabilities = manifest.get("capabilities")
    if capabilities is None:
        return  # Legacy API v1 packages keep their previous contract.
    if not isinstance(capabilities, list) or len(capabilities) > 64 or any(not isinstance(c, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", c) for c in capabilities) or len(set(capabilities)) != len(capabilities):
        raise ValueError("invalid or duplicate source capabilities")
    options = manifest.get("quality_options", [])
    identifiers = [item.get("id", "") for item in options]
    if len(identifiers) > 32 or len(set(identifiers)) != len(identifiers) or any(not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", i) for i in identifiers):
        raise ValueError("invalid or duplicate quality ids")
    if report is not None:
        methods = report.get("methods", [])
        if report.get("version") != 1 or len(methods) > 128 or any(SOURCE_METHODS[c] not in methods for c in capabilities if c in SOURCE_METHODS):
            raise ValueError("declared capability has no implemented method")


def exercise(directory, cases_file):
    manifest = check(directory)
    raw = pathlib.Path(cases_file).read_bytes()
    if len(raw) > 128 * 1024:
        raise ValueError("scenario file exceeds 128 KiB")
    cases = json.loads(raw)
    if not isinstance(cases, list) or not 1 <= len(cases) <= 32:
        raise ValueError("expected 1..32 scenarios")
    requests = [(case["method"], case.get("params", {})) for case in cases]
    replies = rpc(directory, requests, timeout=25, allow_errors=True)
    for case, reply in zip(cases, replies):
        if "expected_error" in case:
            if reply.get("error", {}).get("code") != case["expected_error"]:
                raise ValueError("scenario error classification mismatch")
            continue
        if "error" in reply:
            raise ValueError("scenario returned an unexpected plugin error")
        expected = case.get("expected", {})
        if not isinstance(reply, dict) or any(reply.get(key) != value for key, value in expected.items()):
            raise ValueError("scenario result mismatch")
        if case["method"] in ("search", "playlist_tracks", "liked_songs", "recommendations", "album_catalog", "related_catalog"):
            for track in reply.get("items", []):
                if not track.get("id", "").startswith(manifest["id"] + ":") or track.get("source") != manifest["id"]:
                    raise ValueError("scenario returned foreign track identity")
    return manifest


def schema(value, depth=0):
    allowed = {"type", "properties", "required", "additionalProperties", "items", "maxItems",
               "minimum", "maximum", "enum", "maxLength", "description", "secret"}
    if depth > 8 or not isinstance(value, dict) or set(value) - allowed:
        raise ValueError("unsupported or deeply nested schema")
    kind = value.get("type")
    if kind == "object":
        if value.get("additionalProperties") is not False or len(value.get("properties", {})) > 32:
            raise ValueError("objects must be closed and bounded")
        for item in value.get("properties", {}).values():
            schema(item, depth + 1)
    elif kind == "array":
        if not 0 <= value.get("maxItems", -1) <= 1000:
            raise ValueError("arrays need maxItems <= 1000")
        schema(value["items"], depth + 1)
    elif kind == "string":
        if not 0 <= value.get("maxLength", -1) <= 8192:
            raise ValueError("strings need maxLength <= 8192")
    elif kind not in ("integer", "number", "boolean"):
        raise ValueError("unsupported schema type")


def rpc(directory, requests, timeout=8, allow_errors=False):
    """No installed credentials, bounded pipe, deadline, kill-and-wait on every exit."""
    with tempfile.TemporaryDirectory(prefix="rmusic-plugin-check-") as data:
        import os
        env = {k: v for k, v in os.environ.items()
               if not any(word in k.upper() for word in ("TOKEN", "SECRET", "PASSWORD", "API_KEY"))}
        env["RMUSIC_PROVIDER_DATA_DIR"] = data
        process = subprocess.Popen([sys.executable, "-u", "provider.py"], cwd=directory, env=env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        selector = selectors.DefaultSelector()
        try:
            encoded = b"".join(json.dumps({"id": str(i), "method": method, "params": params}).encode() + b"\n"
                               for i, (method, params) in enumerate(requests))
            process.stdin.write(encoded)
            process.stdin.close()
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + timeout
            buffer = b""
            while time.monotonic() < deadline:
                ready = selector.select(max(0, deadline - time.monotonic()))
                if not ready:
                    raise ValueError("RPC timed out")
                chunk = process.stdout.read1(4096)
                if not chunk:
                    break
                buffer += chunk
                if len(buffer) > MAX_REPLY:
                    raise ValueError("RPC output exceeds 128 KiB")
            else:
                raise ValueError("RPC timed out")
            replies = [json.loads(line) for line in buffer.splitlines()]
            if len(replies) != len(requests):
                raise ValueError("RPC reply count mismatch")
            for i, reply in enumerate(replies):
                if reply.get("id") != str(i) or ("error" in reply and not allow_errors) or ("result" not in reply and "error" not in reply):
                    raise ValueError("RPC correlation or result failed")
            return [r["result"] if "result" in r else {"error": r["error"]} for r in replies]
        finally:
            selector.close()
            if process.poll() is None:
                process.kill()
            process.wait(timeout=3)
            process.stdout.close()


def check(directory):
    directory = pathlib.Path(directory).resolve()
    raw = (directory / "manifest.json").read_bytes()
    if len(raw) > 64 * 1024:
        raise ValueError("manifest exceeds 64 KiB")
    manifest = json.loads(raw)
    validate(manifest)
    source_contract(manifest)
    requests = [("manifest", {}), ("health", {})]
    if "contract_probe_v1" in manifest.get("capabilities", []):
        requests.append(("contract", {}))
    replies = rpc(directory, requests)
    if len(replies) > 2:
        source_contract(manifest, replies[2])
    if replies[0] != manifest or replies[1].get("status") != "ready":
        raise ValueError("manifest consistency or health failed")
    return manifest


def init(identifier, kind, output):
    if not ID.fullmatch(identifier):
        raise ValueError("invalid plugin id")
    directory = pathlib.Path(output)
    directory.mkdir(parents=False, exist_ok=False)
    manifest = json.loads((ASSETS / (kind + ".json")).read_text(encoding="utf-8"))
    manifest["id"] = identifier
    (directory / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    shutil.copyfile(ASSETS / (kind + ".py"), directory / "provider.py")
    shutil.copyfile(ASSETS / "sdk.py", directory / "rmusic_plugin.py")
    shutil.copyfile(ASSETS / "dev.py", directory / "plugin_dev.py")
    (directory / "README.md").write_text("# " + identifier + "\n\nRun plugin_dev.py check before packing. Never log credentials.\n", encoding="utf-8")


def pack(directory, output):
    manifest = check(directory)
    directory = pathlib.Path(directory).resolve()
    files = []
    for path in directory.rglob("*"):
        parts = path.relative_to(directory).parts
        if any(p.startswith(".") or p in ("__pycache__", "data", "node_modules") for p in parts):
            continue
        if path.is_symlink():
            raise ValueError("package must not contain symlinks")
        if path.is_file() and path.name not in ("configuration.json", "plugin_dev.py") and path.suffix.lower() in (".py", ".json", ".md", ".txt"):
            files.append(path)
    if sum(p.stat().st_size for p in files) > 64 * 1024 * 1024:
        raise ValueError("expanded package exceeds 64 MiB")
    import os
    destination = pathlib.Path(output)
    with tempfile.NamedTemporaryFile(prefix=".rmusic-package-", suffix=".partial", dir=destination.parent) as stream:
        with zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(files):
                archive.write(path, path.relative_to(directory).as_posix())
        stream.flush()
        if stream.tell() > 16 * 1024 * 1024:
            raise ValueError("compressed package exceeds 16 MiB")
        os.fsync(stream.fileno())
        os.link(stream.name, destination)  # Publish a complete package without overwriting.
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("init")
    create.add_argument("id")
    create.add_argument("--kind", choices=("source", "extension"), default="extension")
    create.add_argument("--output", required=True)
    verify = commands.add_parser("check")
    verify.add_argument("directory")
    verify.add_argument("--scenarios", help="JSON scenarios executed with an empty data directory")
    build = commands.add_parser("pack")
    build.add_argument("directory")
    build.add_argument("--output", required=True)
    args = parser.parse_args()
    try:
        if args.command == "init":
            init(args.id, args.kind, args.output)
            print(json.dumps({"created": args.output, "kind": args.kind}))
        else:
            result = pack(args.directory, args.output) if args.command == "pack" else exercise(args.directory, args.scenarios) if args.scenarios else check(args.directory)
            checks = ["manifest", "rpc_manifest", "health"]
            if "contract_probe_v1" in result.get("capabilities", []):
                checks.append("source_methods")
            if args.command == "check" and args.scenarios:
                checks.append("scenarios")
            print(json.dumps({"passed": True, "id": result["id"], "checks": checks}))
    except Exception as error:
        print(json.dumps({"passed": False, "error": str(error)}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
