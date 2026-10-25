#!/usr/bin/env python3
"""Apply tracked Android settings to Tauri's generated project."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ANDROID = ROOT / "src-tauri" / "gen" / "android"
MANIFEST = ANDROID / "app" / "src" / "main" / "AndroidManifest.xml"
PROPERTIES = ANDROID / "app" / "tauri.properties"
BUILD_TASK = ANDROID / "buildSrc" / "src" / "main" / "java" / "com" / "gmail" / "solvaus" / "hybridi" / "kotlin" / "BuildTask.kt"
RUST_PLUGIN = ANDROID / "buildSrc" / "src" / "main" / "java" / "com" / "gmail" / "solvaus" / "hybridi" / "kotlin" / "RustPlugin.kt"
CONFIG = ROOT / "src-tauri" / "tauri.conf.json"
ACTIVITY = '            android:name=".MainActivity"'
ORIENTATION = '            android:screenOrientation="portrait"'
BUILD_TASK_PACKAGE = "package com.gmail.solvaus.hybridi.kotlin"


def fix_build_task_package() -> None:
    if not BUILD_TASK.is_file():
        raise SystemExit(f"Generated Android Rust build task not found: {BUILD_TASK}")

    source = BUILD_TASK.read_text(encoding="utf-8")
    if not source.startswith(BUILD_TASK_PACKAGE + "\n") and not source.startswith(BUILD_TASK_PACKAGE + "\r\n"):
        if source.startswith("package "):
            raise SystemExit(f"Unexpected package declaration in {BUILD_TASK}")
        if not re.search(r"(?:abstract\s+)?class BuildTask\s*:\s*DefaultTask\(\)", source):
            raise SystemExit(f"Unrecognized generated Rust build task: {BUILD_TASK}")
        source = BUILD_TASK_PACKAGE + "\n\n" + source

    cargo_resolver = '''        val cargoHome = System.getenv("CARGO_HOME")
            ?: File(System.getProperty("user.home"), ".cargo").absolutePath
        val cargoBin = File(cargoHome, "bin")
        val executable = listOf("cargo.exe", "cargo.cmd", "cargo.bat", "cargo")
            .map { File(cargoBin, it) }
            .firstOrNull { it.isFile }
            ?.absolutePath
            ?: "cargo"'''
    executable_line = re.compile(r'(?m)^[ \t]*val executable = """.*?""";\s*$')
    if executable_line.search(source):
        source = executable_line.sub(cargo_resolver, source, count=1)
    elif 'val cargoHome = System.getenv("CARGO_HOME")' not in source:
        raise SystemExit(f"Unrecognized Cargo executable setup in {BUILD_TASK}")

    if "import java.util.Properties" not in source:
        source = source.replace("import java.io.File\n", "import java.io.File\nimport java.util.Properties\n", 1)

    host_and_ndk_bin = '''        val ndkHost = when {
            System.getProperty("os.name").startsWith("Windows", ignoreCase = true) -> "windows-x86_64"
            System.getProperty("os.name").startsWith("Mac", ignoreCase = true) -> "darwin-x86_64"
            else -> "linux-x86_64"
        }
        val androidNdkBin = androidNdk?.let {
            File(it, "toolchains/llvm/prebuilt/$ndkHost/bin")
        }'''
    ndk_declarations = '''        val androidSdk = System.getenv("ANDROID_HOME")
            ?: System.getenv("ANDROID_SDK_ROOT")
            ?: Properties().let { properties ->
                val localProperties = File(project.rootDir, "local.properties")
                if (localProperties.isFile) {
                    localProperties.inputStream().use { properties.load(it) }
                }
                properties.getProperty("sdk.dir")
            }
        val androidNdk = sequenceOf(
            System.getenv("ANDROID_NDK_HOME"),
            System.getenv("NDK_HOME"),
            System.getenv("ANDROID_NDK_ROOT")
        ).firstOrNull { !it.isNullOrBlank() } ?: androidSdk?.let { sdk ->
            File(sdk, "ndk").listFiles()
                ?.filter { it.isDirectory }
                ?.maxByOrNull { it.name }
                ?.absolutePath
        }
        val ndkHost = when {
            System.getProperty("os.name").startsWith("Windows", ignoreCase = true) -> "windows-x86_64"
            System.getProperty("os.name").startsWith("Mac", ignoreCase = true) -> "darwin-x86_64"
            else -> "linux-x86_64"
        }
        val androidNdkBin = androidNdk?.let {
            File(it, "toolchains/llvm/prebuilt/$ndkHost/bin")
        }'''
    old_ndk_bin = '''        val androidNdkBin = androidNdk?.let {
            File(it, "toolchains/llvm/prebuilt/windows-x86_64/bin")
        }'''
    if old_ndk_bin in source:
        source = source.replace(old_ndk_bin, host_and_ndk_bin, 1)
    elif 'val androidNdkBin = androidNdk?.let {' not in source:
        run_function = source.find("    fun runTauriCli(executable: String) {")
        args_line = re.search(r'(?m)^([ \t]*)val args = .*$', source[run_function:]) if run_function >= 0 else None
        if not args_line:
            raise SystemExit(f"Unrecognized Tauri CLI args in {BUILD_TASK}")
        insert_at = run_function + args_line.end()
        source = source[:insert_at] + "\n\n" + ndk_declarations + source[insert_at:]

    if 'environment("ANDROID_NDK_HOME", androidNdk)' not in source:
        exec_block = re.search(r'(?m)^([ \t]*)(?:project\.exec|execOperations\.exec)\s*\{\s*$', source)
        workdir = (
            re.search(
                r'(?m)^([ \t]*)workingDir\(File\((?:project\.projectDir|projectDir), rootDirRel\)\)',
                source[exec_block.end():],
            )
            if exec_block
            else None
        )
        if not exec_block or not workdir:
            raise SystemExit(f"Unrecognized Rust build process setup in {BUILD_TASK}")
        indent = workdir.group(1)
        env_setup = f'''{indent}if (!androidSdk.isNullOrBlank()) environment("ANDROID_HOME", androidSdk)
{indent}if (!androidNdk.isNullOrBlank()) {{
{indent}    environment("ANDROID_NDK_HOME", androidNdk)
{indent}    environment("NDK_HOME", androidNdk)
{indent}}}
{indent}if (androidNdkBin?.isDirectory == true) {{
{indent}    environment(
{indent}        "PATH",
{indent}        androidNdkBin.absolutePath + File.pathSeparator + System.getenv("PATH").orEmpty()
{indent}    )
{indent}}}
'''
        workdir_at = exec_block.end() + workdir.start()
        source = source[:workdir_at] + env_setup + source[workdir_at:]

    BUILD_TASK.write_text(source, encoding="utf-8", newline="\n")

    # Some Tauri CLI templates omit the package declaration on BuildTask.kt,
    # while RustPlugin.kt references BuildTask as if both files shared a
    # package. Add an explicit import so this remains valid if the generated
    # plugin's package declaration changes between CLI versions.
    if not RUST_PLUGIN.is_file():
        raise SystemExit(f"Generated Rust plugin not found: {RUST_PLUGIN}")
    plugin = RUST_PLUGIN.read_text(encoding="utf-8")
    build_task_import = f"import {BUILD_TASK_PACKAGE.removeprefix('package ')}.BuildTask"
    if build_task_import not in plugin:
        package_match = re.search(r"(?m)^package\s+[^\r\n]+\s*$", plugin)
        if not package_match:
            raise SystemExit(f"Could not locate package declaration in {RUST_PLUGIN}")
        plugin = plugin[:package_match.end()] + "\n\n" + build_task_import + plugin[package_match.end():]
        RUST_PLUGIN.write_text(plugin, encoding="utf-8", newline="\n")

    print("Configured generated Rust build task package, Cargo, and Android NDK environment")


def sync_version() -> None:
    config = json.loads(CONFIG.read_text(encoding="utf-8"))
    version_name = config.get("version")
    version_code = config.get("bundle", {}).get("android", {}).get("versionCode")
    if not isinstance(version_name, str) or not version_name:
        raise SystemExit(f"App version is missing from {CONFIG}")
    if isinstance(version_code, bool) or not isinstance(version_code, int) or not 1 <= version_code <= 2_100_000_000:
        raise SystemExit(f"A valid bundle.android.versionCode is required in {CONFIG}")

    lines = PROPERTIES.read_text(encoding="utf-8").splitlines() if PROPERTIES.exists() else []
    lines = [
        line for line in lines
        if not line.startswith(("tauri.android.versionName=", "tauri.android.versionCode="))
    ]
    if not any(line.startswith("// THIS IS AN AUTOGENERATED FILE") for line in lines):
        lines.insert(0, "// THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.")
    lines.extend((
        f"tauri.android.versionName={version_name}",
        f"tauri.android.versionCode={version_code}",
    ))
    PROPERTIES.parent.mkdir(parents=True, exist_ok=True)
    PROPERTIES.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"Synced Android versionName={version_name}, versionCode={version_code}")


def main() -> int:
    if not MANIFEST.is_file():
        raise SystemExit(f"Generated Android manifest not found: {MANIFEST}")

    fix_build_task_package()
    sync_version()
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
