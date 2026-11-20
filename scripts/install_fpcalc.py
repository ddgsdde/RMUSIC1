#!/usr/bin/env python3
"""Install checksum-pinned upstream fpcalc for a Linux Core image, without unpacking paths."""
import argparse
import hashlib
import io
import json
import pathlib
import platform
import tarfile
import urllib.request

VERSION = '1.6.0'
RELEASES = {
    'arm64': ('linux-arm64', 'c8667f556f77d8ebbe08b75a968c0592bd2a67aaa696eff91715feb5083b1cd4'),
    'amd64': ('linux-x86_64', '946dc3eade645eb835c8d163c6bb354e092239988bff190b9c42589e8d5cf00a'),
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=pathlib.Path, default=pathlib.Path('/usr/local/bin'))
    parser.add_argument('--arch', choices=RELEASES)
    args = parser.parse_args()
    arch = args.arch or {'aarch64': 'arm64', 'arm64': 'arm64', 'x86_64': 'amd64'}.get(platform.machine())
    if arch not in RELEASES:
        raise SystemExit('fpcalc image supports Linux amd64 / arm64 only')
    target, expected = RELEASES[arch]
    name = f'chromaprint-fpcalc-{VERSION}-{target}'
    url = f'https://github.com/acoustid/chromaprint/releases/download/v{VERSION}/{name}.tar.gz'
    with urllib.request.urlopen(url, timeout=60) as response:
        archive = response.read(32 * 1024 * 1024 + 1)
    if len(archive) > 32 * 1024 * 1024 or hashlib.sha256(archive).hexdigest() != expected:
        raise SystemExit('fpcalc release checksum mismatch')
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as bundle:
        member = bundle.getmember(name + '/fpcalc')
        if not member.isfile() or not 0 < member.size <= 32 * 1024 * 1024:
            raise SystemExit('invalid fpcalc release entry')
        # Write this exact verified member; never extract archive paths or symlinks.
        binary = bundle.extractfile(member).read()
    args.directory.mkdir(parents=True, exist_ok=True)
    executable = args.directory / 'fpcalc'
    executable.write_bytes(binary)
    executable.chmod(0o755)
    (args.directory / 'fpcalc.release.json').write_text(json.dumps({
        'version': VERSION, 'source': url, 'archive_sha256': expected,
        'binary_sha256': hashlib.sha256(binary).hexdigest(),
        'license': 'https://github.com/acoustid/chromaprint/blob/v1.6.0/LICENSE.md',
    }, indent=2) + '\n')

if __name__ == '__main__':
    main()
