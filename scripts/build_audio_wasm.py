"""Build the native pure DSP browser module; requires rustup target add wasm32-unknown-unknown."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
environment = os.environ.copy()
environment['CARGO_INCREMENTAL'] = '0'
environment['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join([
    '-C', 'target-feature=+simd128',
    f'--remap-path-prefix={Path.home()}=/build-user',
    f'--remap-path-prefix={root}=/src',
])
subprocess.run(['cargo', 'rustc', '-p', 'rmusic-audio-wasm', '--lib', '--target', 'wasm32-unknown-unknown', '--release', '--', '-C', 'link-arg=--max-memory=268435456'], cwd=root, env=environment, check=True)
source = root / 'target/wasm32-unknown-unknown/release/rmusic_audio_wasm.wasm'
assert source.read_bytes()[:8] == b'\0asm\1\0\0\0'
destination = root / 'apps/web/src/audio/rmusic-audio.wasm'
shutil.copy2(source, destination)
sources = ['Cargo.toml', 'Cargo.lock', 'crates/audio-wasm/Cargo.toml', 'crates/audio-wasm/src/lib.rs', 'crates/audio/src/dsp.rs', 'crates/audio/src/impulse.rs', 'crates/audio/src/convolution.rs', 'scripts/build_audio_wasm.py']
manifest = {'sha256': hashlib.sha256(destination.read_bytes()).hexdigest(), 'bytes': destination.stat().st_size, 'memory_limit_bytes': 268435456, 'target_features': ['simd128'], 'source_hash_encoding': 'UTF-8 with LF newlines', 'sources': {relative: hashlib.sha256((root / relative).read_text(encoding='utf-8').encode('utf-8')).hexdigest() for relative in sources}}
destination.with_suffix('.wasm.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'wasm': str(destination), 'bytes': manifest['bytes'], 'sha256': manifest['sha256']}))
