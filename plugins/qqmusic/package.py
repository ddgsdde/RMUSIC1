"""Build an update package containing source and pinned requirements only."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=Path("artifacts"))
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    manifest = json.loads((source / "manifest.json").read_text("utf-8"))
    args.output.mkdir(parents=True, exist_ok=True)
    destination = args.output / f"{manifest['id']}-{manifest['version']}.zip"
    with ZipFile(destination, "w", compression=ZIP_DEFLATED) as archive:
        for name in ("manifest.json", "provider.py", "lyric_timing.py", "qq_artwork.py", "public_metadata.py", "requirements.txt", "README.md", "LICENSE"):
            archive.write(source / name, arcname=name)
    print(destination.resolve())


if __name__ == "__main__":
    main()
