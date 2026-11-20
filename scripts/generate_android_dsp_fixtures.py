"""Generate deterministic Android DSP fixtures and independent FFmpeg f64 references."""
import argparse
import math
from pathlib import Path
import struct
import subprocess
import wave

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ffmpeg', default=str(root / 'artifacts/windows/tools/ffmpeg.exe'))
parser.add_argument('--work-dir', type=Path, default=root / 'target/android-dsp-fixtures')
args = parser.parse_args()
args.work_dir.mkdir(parents=True, exist_ok=True)
assets = root / 'apps/desktop/src-tauri/gen/android/app/src/androidTest/assets'
for rate, bits, extension in [(96000, 24, 'flac'), (768000, 32, 'wav')]:
    name = f'dsp-{rate}'
    samples = []
    for frame in range(4096):
        for channel in range(2):
            value = round((math.sin(frame * .173 + channel) * .21 + math.cos(frame * .037) * .03) * 2147483648)
            samples.append(value & ~255 if bits == 24 else value)
    raw = struct.pack('<' + 'i' * len(samples), *samples)
    source = args.work_dir / (name + '.wav')
    with wave.open(str(source), 'wb') as output:
        output.setnchannels(2)
        output.setsampwidth(4)
        output.setframerate(rate)
        output.writeframes(raw)
    fixture = assets / (name + '.' + extension)
    if extension == 'flac':
        subprocess.run([args.ffmpeg, '-v', 'error', '-y', '-i', str(source), '-c:a', 'flac', '-sample_fmt', 's32', '-bits_per_raw_sample', '24', str(fixture)], check=True)
    else:
        fixture.write_bytes(source.read_bytes())
    # Preamp -6 dB plus measured -14 -> target -18 LUFS gives -10 dB total.
    filters = 'aformat=sample_fmts=dbl,volume=-10dB:precision=double,pan=stereo|c0=0.75*c0|c1=c1,equalizer=f=1200:t=q:w=1.2:g=4:r=f64'
    subprocess.run([args.ffmpeg, '-v', 'error', '-y', '-i', str(fixture), '-af', filters, '-c:a', 'pcm_s32le', '-f', 's32le', str(assets / (name + '-processed.s32'))], check=True)
    (assets / (name + '-bypass.s32')).write_bytes(raw[7 * 8:])
    fade_ms = 10 if rate == 96000 else 1
    fade_frames = rate * fade_ms // 1000
    envelope = f'afade=t=in:ns={fade_frames}:curve=qsin,afade=t=out:ss={4096-fade_frames}:ns={fade_frames}:curve=qsin'
    for label, processing in [('dsp', filters), ('bypass', 'aformat=sample_fmts=dbl')]:
        reference = assets / (name + '-faded-' + label + '.s32')
        subprocess.run([args.ffmpeg, '-v', 'error', '-y', '-i', str(fixture), '-af', processing + ',' + envelope, '-c:a', 'pcm_s32le', '-f', 's32le', str(reference)], check=True)
        if label == 'bypass':
            reference.write_bytes(reference.read_bytes()[7 * 8:])
