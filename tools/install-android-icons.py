#!/usr/bin/env python3
"""Copy generated Tauri launcher icons into the generated Android app resources."""

from __future__ import annotations

import shutil
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ANDROID_RES = ROOT / "src-tauri" / "gen" / "android" / "app" / "src" / "main" / "res"


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: install-android-icons.py <generated-icon-output>/android")

    source = Path(sys.argv[1]).resolve()
    if not source.is_dir():
        raise SystemExit(f"Generated Android icon resources not found: {source}")
    if not (source / "mipmap-anydpi-v26" / "ic_launcher.xml").is_file():
        raise SystemExit(f"Tauri Android launcher icon resources are incomplete: {source}")

    ANDROID_RES.mkdir(parents=True, exist_ok=True)
    copied = 0
    for item in source.iterdir():
        destination = ANDROID_RES / item.name
        if item.is_dir():
            shutil.copytree(item, destination, dirs_exist_ok=True)
            copied += sum(1 for path in item.rglob("*") if path.is_file())
        elif item.is_file():
            shutil.copy2(item, destination)
            copied += 1

    print(f"Installed {copied} Android launcher icon resources into {ANDROID_RES}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
