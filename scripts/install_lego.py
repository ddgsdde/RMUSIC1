#!/usr/bin/env python3
"""Install the pinned ACME client in a Core container image."""
import argparse
import hashlib
import io
from pathlib import Path
import tarfile
import urllib.request

VERSION = "5.6.0"
BASE = f"https://github.com/go-acme/lego/releases/download/v{VERSION}/"


def download(name, limit):
    request = urllib.request.Request(BASE + name, headers={"User-Agent": "R-Music-build/0.1"})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"ACME asset exceeds size limit: {name}")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=["amd64", "arm64"], required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()

    name = f"lego_v{VERSION}_linux_{args.arch}.tar.gz"
    checksums = dict(reversed(line.split()) for line in download(f"lego_{VERSION}_checksums.txt", 1024 * 1024).decode().splitlines() if len(line.split()) == 2)
    archive = download(name, 160 * 1024 * 1024)
    if hashlib.sha256(archive).hexdigest() != checksums.get(name):
        raise ValueError("ACME vendor checksum mismatch")

    args.destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as source:
        for name, filename in [("lego", "lego"), ("LICENSE", "lego-LICENSE")]:
            entry = source.getmember(name)
            if not entry.isfile():
                raise ValueError(f"Invalid ACME archive member: {name}")
            path = args.destination / filename
            path.write_bytes(source.extractfile(entry).read())
            path.chmod(0o755 if name == "lego" else 0o644)


if __name__ == "__main__":
    main()
