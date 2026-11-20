"""Build the local-install archive and a standalone HACS repository bundle."""

import argparse
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED


def package(destination: Path) -> None:
    root = Path(__file__).resolve().parent
    destination.mkdir(parents=True, exist_ok=True)
    with ZipFile(destination / "rmusic-home-assistant.zip", "w", ZIP_DEFLATED) as archive:
        archive.write(root / "LICENSE", "custom_components/rmusic/LICENSE")
        for path in sorted((root / "custom_components").rglob("*")):
            if path.is_file() and "__pycache__" not in path.parts and path.suffix != ".pyc":
                archive.write(path, path.relative_to(root).as_posix())
    with ZipFile(destination / "rmusic-hacs-repository.zip", "w", ZIP_DEFLATED) as archive:
        for path in sorted(root.rglob("*")):
            if path.is_file() and ("custom_components" in path.parts or path.name in {"README.md", "hacs.json", "LICENSE"}):
                if "__pycache__" not in path.parts and path.suffix != ".pyc":
                    archive.write(path, path.relative_to(root).as_posix())


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=Path("artifacts"))
    package(parser.parse_args().output)
