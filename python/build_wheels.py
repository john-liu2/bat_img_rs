#!/usr/bin/env python3
"""
build_wheels.py — assembles one platform-specific wheel per binary artifact.

Usage:
    python build_wheels.py <binaries-dir>

Each subdirectory of <binaries-dir> should contain one binary file named
after the artifact (e.g. `bat_img-aarch64-apple-darwin`).

Outputs .whl files into dist/.
"""

import os
import re
import shutil
import stat
import sys
import zipfile
from pathlib import Path

# Map Rust target triple → Python wheel platform tag
PLATFORM_MAP = {
    "aarch64-apple-darwin":      "macosx_11_0_arm64",
    "x86_64-apple-darwin":       "macosx_10_12_x86_64",
    "x86_64-unknown-linux-gnu":  "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "x86_64-pc-windows-msvc":    "win_amd64",
}

PACKAGE_NAME  = "bat_img"
PACKAGE_DIR   = "bat_img_cli"
PYTHON_TAG    = "py3"
ABI_TAG       = "none"
VERSION       = "1.0.0"   # keep in sync with pyproject.toml


def wheel_name(platform_tag: str) -> str:
    return f"{PACKAGE_NAME}-{VERSION}-{PYTHON_TAG}-{ABI_TAG}-{platform_tag}.whl"


def build_wheel(binary_path: Path, target: str, dist_dir: Path) -> None:
    platform_tag = PLATFORM_MAP[target]
    is_windows   = "windows" in target
    bin_name     = "bat_img.exe" if is_windows else "bat_img"

    whl_path = dist_dir / wheel_name(platform_tag)
    print(f"  → {whl_path.name}")

    with zipfile.ZipFile(whl_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        # ── Bundle the binary ────────────────────────────────────────────────
        zf.write(binary_path, f"{PACKAGE_DIR}/bin/{bin_name}")

        # ── Python shim ──────────────────────────────────────────────────────
        shim = Path(__file__).parent / PACKAGE_DIR / "__init__.py"
        zf.write(shim, f"{PACKAGE_DIR}/__init__.py")

        # ── dist-info ────────────────────────────────────────────────────────
        dist_info = f"{PACKAGE_NAME}-{VERSION}.dist-info"

        # METADATA
        metadata = f"""\
Metadata-Version: 2.3
Name: {PACKAGE_NAME}
Version: {VERSION}
Summary: Fast multithreaded batch image processor (HEIC, JPEG, PNG, WebP, ...)
License: MIT
Requires-Python: >=3.8
"""
        zf.writestr(f"{dist_info}/METADATA", metadata)

        # WHEEL
        wheel_meta = f"""\
Wheel-Version: 1.0
Generator: build_wheels.py
Root-Is-Purelib: false
Tag: {PYTHON_TAG}-{ABI_TAG}-{platform_tag}
"""
        zf.writestr(f"{dist_info}/WHEEL", wheel_meta)

        # entry_points.txt
        entry_points = "[console_scripts]\nbat_img = bat_img_cli:main\n"
        zf.writestr(f"{dist_info}/entry_points.txt", entry_points)

        # RECORD (required by pip)
        record_lines = [
            f"{PACKAGE_DIR}/bin/{bin_name},,",
            f"{PACKAGE_DIR}/__init__.py,,",
            f"{dist_info}/METADATA,,",
            f"{dist_info}/WHEEL,,",
            f"{dist_info}/entry_points.txt,,",
            f"{dist_info}/RECORD,,",
        ]
        zf.writestr(f"{dist_info}/RECORD", "\n".join(record_lines))


def main() -> None:
    if len(sys.argv) < 2:
        print("Usage: build_wheels.py <binaries-dir>")
        sys.exit(1)

    binaries_dir = Path(sys.argv[1])
    dist_dir     = Path("dist")
    dist_dir.mkdir(exist_ok=True)

    print(f"Scanning {binaries_dir} for artifacts …")

    built = 0
    for subdir in sorted(binaries_dir.iterdir()):
        if not subdir.is_dir():
            continue

        # Artifact dir name: bat_img-<target>  or  bat_img-<target>.exe
        name = subdir.name
        match = re.match(r"bat_img-(.+)", name)
        if not match:
            continue

        target = match.group(1)
        if target not in PLATFORM_MAP:
            print(f"  Skipping unknown target: {target}")
            continue

        # Find the binary inside the artifact dir
        candidates = list(subdir.glob("bat_img*"))
        if not candidates:
            print(f"  No binary found in {subdir}, skipping")
            continue

        binary = candidates[0]
        # Ensure executable bit is set (lost during artifact upload on Linux/macOS)
        binary.chmod(binary.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

        build_wheel(binary, target, dist_dir)
        built += 1

    print(f"\nBuilt {built} wheel(s) in {dist_dir}/")


if __name__ == "__main__":
    main()
