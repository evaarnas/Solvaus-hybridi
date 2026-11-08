#!/usr/bin/env python3
"""Update all release version copies from one version argument."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def parse_version(value: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", value)
    if not match:
        raise argparse.ArgumentTypeError("version must be a semantic version like 0.2.16")
    return tuple(map(int, match.groups()))


def replace_once(path: Path, pattern: str, replacement: str, already_applied: str | None = None) -> None:
    source = path.read_text(encoding="utf-8")
    if already_applied and already_applied in source:
        return
    updated, count = re.subn(pattern, replacement, source, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"expected one version field in {path.relative_to(ROOT)}")
    path.write_text(updated, encoding="utf-8", newline="")


def automatic_notes(target: tuple[int, int, int]) -> list[str]:
    previous = []
    for path in (ROOT / "docs").glob("release-notes-v*.md"):
        match = re.fullmatch(r"release-notes-v(\d+)\.(\d+)\.(\d+)\.md", path.name)
        if match:
            version = tuple(map(int, match.groups()))
            if version < target:
                previous.append((version, path))
    if not previous:
        raise SystemExit("cannot find previous release notes; supply --note bullets")
    previous_version, previous_notes = max(previous)
    previous_tag = "v" + ".".join(map(str, previous_version))
    tag_result = subprocess.run(
        ["git", "rev-parse", "--verify", f"refs/tags/{previous_tag}^{{commit}}"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if tag_result.returncode == 0:
        anchor = tag_result.stdout.strip()
    else:
        # The notes file can be edited after its release-prep commit (for example,
        # when adding an asset to the pending draft). Anchor at its creation so
        # those later commits are still included in the next draft's notes.
        anchor = subprocess.run(
            ["git", "log", "-1", "--diff-filter=A", "--format=%H", "--", str(previous_notes.relative_to(ROOT))],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        if not anchor:
            anchor = subprocess.run(
                ["git", "log", "-1", "--format=%H", "--", str(previous_notes.relative_to(ROOT))],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
    if not anchor:
        raise SystemExit("previous release notes are not committed; supply --note bullets")
    history = subprocess.run(
        ["git", "log", "--no-merges", "--format=%s", f"{anchor}..HEAD"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    ignored = re.compile(r"^(?:release|prepare release|bump version)\b", re.I)
    notes = [subject.rstrip(".") + "." for subject in history if subject.strip() and not ignored.match(subject)]
    if not notes:
        raise SystemExit("no commits since the previous release notes; supply --note bullets")
    return notes


def release_is_current(version: str, version_code: int, tag: str, notes: Path) -> bool:
    try:
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        desktop = tomllib.loads((ROOT / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8"))
        config = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
        releases = ET.parse(ROOT / "packaging" / "flatpak" / "io.github.adamnolle.cubesolver.metainfo.xml").getroot().find("./releases")
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        if workspace["workspace"]["package"]["version"] != version or desktop["package"]["version"] != version:
            return False
        if config["version"] != version or config["bundle"]["android"]["versionCode"] != version_code:
            return False
        if releases is None or len(releases) == 0 or releases[0].get("version") != version:
            return False
        if f"        default: {tag}" not in workflow or f"tag must look like {tag}" not in workflow:
            return False
        if not notes.is_file():
            return False
        local_names = {"cube_core", "cube_solver", "cube_wasm", "solver_lab_app", "solver_store", "cube-solver"}
        for lock_path in (ROOT / "Cargo.lock", ROOT / "src-tauri" / "Cargo.lock"):
            lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
            if any(package["version"] != version for package in lock["package"] if package["name"] in local_names):
                return False
        return True
    except (OSError, KeyError, IndexError, ET.ParseError, tomllib.TOMLDecodeError, json.JSONDecodeError):
        return False


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", type=parse_version, help="new X.Y.Z release version")
    parser.add_argument("--date", default=dt.date.today().isoformat(), help="release date in YYYY-MM-DD format")
    parser.add_argument("--note", action="append", default=[], help="release note bullet; repeat to replace automatic commit notes")
    args = parser.parse_args()
    major, minor, patch = args.version
    version = f"{major}.{minor}.{patch}"
    tag = f"v{version}"
    version_code = major * 1_000_000 + minor * 1_000 + patch
    if max(major, minor, patch) > 999 or version_code > 2_100_000_000:
        raise SystemExit("version components exceed the Android version-code scheme")
    try:
        dt.date.fromisoformat(args.date)
    except ValueError as error:
        raise SystemExit("--date must be a valid YYYY-MM-DD date") from error

    workspace = ROOT / "Cargo.toml"
    workspace_text = workspace.read_text(encoding="utf-8")
    current_match = re.search(r"^version\s*=\s*\"([^\"]+)\"", workspace_text.split("[workspace.package]", 1)[1], re.M)
    if not current_match:
        raise SystemExit("could not find [workspace.package] version in Cargo.toml")
    current = parse_version(current_match.group(1))
    if args.version < current:
        raise SystemExit(f"new version {version} must be greater than current {current_match.group(1)}")
    notes = ROOT / "docs" / f"release-notes-{tag}.md"
    if current == args.version and release_is_current(version, version_code, tag, notes):
        print(f"{tag} is already prepared; no files changed.")
        return
    if notes.exists() and args.version > current:
        raise SystemExit(f"release notes already exist: {notes.relative_to(ROOT)}")
    notes_content = args.note or (automatic_notes(args.version) if not notes.exists() else [])
    existing_tag = subprocess.run(
        ["git", "tag", "--list", tag], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout.strip()
    if existing_tag:
        raise SystemExit(f"tag already exists locally: {tag}")

    replace_once(workspace, r"^version\s*=\s*\"[^\"]+\"(?=\s*$)", f'version = "{version}"')
    replace_once(ROOT / "src-tauri" / "Cargo.toml", r"^version\s*=\s*\"[^\"]+\"(?=\s*$)", f'version = "{version}"')

    config_path = ROOT / "src-tauri" / "tauri.conf.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    config["version"] = version
    config["bundle"]["android"]["versionCode"] = version_code
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8", newline="")

    metainfo = ROOT / "packaging" / "flatpak" / "io.github.adamnolle.cubesolver.metainfo.xml"
    replace_once(
        metainfo,
        r"^(\s*<releases>\s*)$",
        f'\\1\n    <release version="{version}" date="{args.date}"/>',
        f'<release version="{version}"',
    )
    workflow = ROOT / ".github" / "workflows" / "release.yml"
    replace_once(workflow, r"^        default: v\d+\.\d+\.\d+$", f"        default: {tag}")
    replace_once(workflow, r"tag must look like v\d+\.\d+\.\d+", f"tag must look like {tag}")
    if not notes.exists():
        notes.write_text(
            f"# Solvaus {tag}\n\n" + "".join(f"- {note.strip()}\n" for note in notes_content),
            encoding="utf-8",
            newline="",
        )
    subprocess.run(["cargo", "update", "--workspace"], cwd=ROOT, check=True)
    subprocess.run(
        ["cargo", "update", "--workspace", "--manifest-path", "src-tauri/Cargo.toml"],
        cwd=ROOT,
        check=True,
    )
    note_source = "provided" if args.note else "commit history"
    print(f"Prepared {tag} (Android version code {version_code}); release notes came from {note_source}. Review before tagging.")


if __name__ == "__main__":
    main()
