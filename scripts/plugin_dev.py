#!/usr/bin/env python3
"""Repository entry point for the same SDK tool distributed in development templates."""
import pathlib
import runpy
import sys

IMPLEMENTATION = pathlib.Path(__file__).resolve().parents[1] / "crates/core/src/plugin_assets/dev.py"
implementation = runpy.run_path(str(IMPLEMENTATION), run_name="rmusic_plugin_dev")
globals().update({name: value for name, value in implementation.items() if not name.startswith("__")})

if __name__ == "__main__":
    sys.exit(main())
