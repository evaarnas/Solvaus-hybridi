#!/usr/bin/env python3
"""Apply tracked Android manifest settings to Tauri's generated project."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "src-tauri" / "gen" / "android" / "app" / "src" / "main" / "AndroidManifest.xml"
ACTIVITY = '            android:name=".MainActivity"'
ORIENTATION = '            android:screenOrientation="portrait"'


def main() -> int:
    if not MANIFEST.is_file():
        raise SystemExit(f"Generated Android manifest not found: {MANIFEST}")

    source = MANIFEST.read_text(encoding="utf-8")
    if ORIENTATION in source:
        print("Android MainActivity is already locked to portrait")
        return 0

    if source.count(ACTIVITY) != 1:
        raise SystemExit("Could not identify exactly one generated MainActivity declaration")

    MANIFEST.write_text(source.replace(ACTIVITY, ORIENTATION + "\n" + ACTIVITY, 1), encoding="utf-8", newline="\n")
    print("Locked Android MainActivity to portrait")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
