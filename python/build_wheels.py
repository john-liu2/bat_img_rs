#!/usr/bin/env python3
"""
build_wheels.py — assembles one platform-specific .whl per binary artifact
                  and one source .tar.gz sdist.

Usage (CI):
    python3 python/build_wheels.py <binaries-dir>

Usage (local wheel for current platform only):
    python3 python/build_wheels.py --local

Outputs into dist/:
    bat_img-1.0.0.tar.gz                                  <- sdist
    bat_img-1.0.0-py3-none-macosx_11_0_arm64.whl
    bat_img-1.0.0-py3-none-macosx_10_12_x86_64.whl
    bat_img-1.0.0-py3-none-manylinux_2_17_x86_64.*.whl
    bat_img-1.0.0-py3-none-win_amd64.whl
"""

import re
import stat
import sys
import tarfile
import zipfile
from io import BytesIO
from pathlib import Path

HERE         = Path(__file__).parent
PACKAGE_NAME = "bat_img"
PACKAGE_DIR  = "bat_img_cli"
PYTHON_TAG   = "py3"
ABI_TAG      = "none"
VERSION      = "1.0.3"   # keep in sync with pyproject.toml

PLATFORM_MAP = {
    "aarch64-apple-darwin":     "macosx_11_0_arm64",
    "x86_64-apple-darwin":      "macosx_10_12_x86_64",
    "x86_64-unknown-linux-gnu": "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "x86_64-pc-windows-msvc":   "win_amd64",
}

# ── Helpers ───────────────────────────────────────────────────────────────────

def wheel_filename(platform_tag):
    return f"{PACKAGE_NAME}-{VERSION}-{PYTHON_TAG}-{ABI_TAG}-{platform_tag}.whl"

def sdist_filename():
    return f"{PACKAGE_NAME}-{VERSION}.tar.gz"

def dist_info_name():
    return f"{PACKAGE_NAME}-{VERSION}.dist-info"

def metadata_text():
    readme = (HERE / "README.md").read_text(encoding="utf-8")
    return (
        f"Metadata-Version: 2.3\n"
        f"Name: {PACKAGE_NAME}\n"
        f"Version: {VERSION}\n"
        f"Summary: Fast multithreaded batch image processor (HEIC, JPEG, PNG, WebP, ...)\n"
        f"Author-email: John Liu <rim2rim@gmail.com>\n"
        f"License: MIT\n"
        f"License-File: ../LICENSE\n"
        f"Requires-Python: >=3.12\n"
        f"Keywords: strip-gps,image,batch,heic,resize,exif,cli,cli-tool,image-processing,batch-processing\n"
        f"Classifier: License :: OSI Approved :: MIT License\n"
        f"Classifier: Programming Language :: Python :: 3 :: Only\n"
        f"Classifier: Programming Language :: Rust\n"
        f"Classifier: Topic :: Multimedia :: Graphics\n"
        f"Classifier: Environment :: Console\n"
        f"Classifier: Development Status :: 5 - Production/Stable\n"
        f"Classifier: Intended Audience :: End Users/Desktop\n"
        f"Classifier: Topic :: Utilities\n"
        f"Description-Content-Type: text/markdown\n"
        f"\n"
        f"{readme}\n"
    )

# ── Wheel ─────────────────────────────────────────────────────────────────────

def build_wheel(binary_path, platform_tag, dist_dir):
    is_windows = "win" in platform_tag
    bin_name   = "bat_img.exe" if is_windows else "bat_img"
    di         = dist_info_name()
    whl_path   = dist_dir / wheel_filename(platform_tag)
    shim_src   = HERE / PACKAGE_DIR / "__init__.py"

    with zipfile.ZipFile(whl_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.write(binary_path, f"{PACKAGE_DIR}/bin/{bin_name}")
        zf.write(shim_src,    f"{PACKAGE_DIR}/__init__.py")
        zf.writestr(f"{di}/METADATA", metadata_text())
        zf.writestr(f"{di}/WHEEL", (
            f"Wheel-Version: 1.0\nGenerator: build_wheels.py\n"
            f"Root-Is-Purelib: false\n"
            f"Tag: {PYTHON_TAG}-{ABI_TAG}-{platform_tag}\n"
        ))
        zf.writestr(f"{di}/entry_points.txt",
                    "[console_scripts]\nbat_img = bat_img_cli:main\n")
        record = "\n".join([
            f"{PACKAGE_DIR}/bin/{bin_name},,",
            f"{PACKAGE_DIR}/__init__.py,,",
            f"{di}/METADATA,,", f"{di}/WHEEL,,",
            f"{di}/entry_points.txt,,", f"{di}/RECORD,,",
        ])
        zf.writestr(f"{di}/RECORD", record)

    print(f"  [wheel] {whl_path.name}")
    return whl_path

# ── Sdist (.tar.gz) ───────────────────────────────────────────────────────────

def build_sdist(dist_dir):
    """
    Source distribution — no compiled binary included.
    pip always prefers a matching wheel, so only developers building from
    source will use the sdist.
    """
    sdist_path = dist_dir / sdist_filename()
    prefix     = f"{PACKAGE_NAME}-{VERSION}"

    files = {
        f"{prefix}/pyproject.toml": (HERE / "pyproject.toml").read_bytes(),
        f"{prefix}/README.md": (HERE / "README.md").read_bytes(),
        f"{prefix}/{PACKAGE_DIR}/__init__.py": (HERE / PACKAGE_DIR / "__init__.py").read_bytes(),
        f"{prefix}/PKG-INFO": metadata_text().encode(),
    }

    with tarfile.open(sdist_path, "w:gz") as tf:
        for arcname, data in files.items():
            info      = tarfile.TarInfo(name=arcname)
            info.size = len(data)
            info.mode = 0o644
            tf.addfile(info, BytesIO(data))

    print(f"  [sdist] {sdist_path.name}")
    return sdist_path

# ── Local build (current platform) ───────────────────────────────────────────

def build_local(dist_dir):
    import platform as _p
    system  = _p.system()
    machine = _p.machine().lower()

    if system == "Darwin":
        platform_tag = "macosx_11_0_arm64" if machine == "arm64" else "macosx_10_12_x86_64"
    elif system == "Linux":
        platform_tag = "manylinux_2_17_x86_64.manylinux2014_x86_64"
    elif system == "Windows":
        platform_tag = "win_amd64"
    else:
        sys.exit(f"Unsupported platform: {system}")

    bin_name  = "bat_img.exe" if system == "Windows" else "bat_img"
    repo_root = HERE.parent

    for profile in ("release-small", "release"):
        candidate = repo_root / "target" / profile / bin_name
        if candidate.exists():
            binary = candidate
            break
    else:
        sys.exit(
            "Binary not found. Build first with:\n"
            "  cargo build --release\n"
            "  cargo build --profile release-small"
        )

    print(f"Using binary: {binary}")
    binary.chmod(binary.stat().st_mode | stat.S_IEXEC)
    whl = build_wheel(binary, platform_tag, dist_dir)
    build_sdist(dist_dir)
    return whl

# ── CI all-platform build ─────────────────────────────────────────────────────

def build_all(binaries_dir, dist_dir):
    built = 0
    for subdir in sorted(binaries_dir.iterdir()):
        if not subdir.is_dir():
            continue
        m = re.match(r"bat_img-(.+)", subdir.name)
        if not m or m.group(1) not in PLATFORM_MAP:
            continue
        candidates = list(subdir.glob("bat_img*"))
        if not candidates:
            continue
        binary = candidates[0]
        binary.chmod(binary.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        build_wheel(binary, PLATFORM_MAP[m.group(1)], dist_dir)
        built += 1
    build_sdist(dist_dir)
    print(f"\nBuilt {built} wheel(s) + 1 sdist in {dist_dir}/")

# ── Entry point ───────────────────────────────────────────────────────────────

def main():
    dist_dir = Path("dist")
    dist_dir.mkdir(exist_ok=True)

    if len(sys.argv) == 2 and sys.argv[1] == "--local":
        print("Building local wheel + sdist for current platform …\n")
        whl = build_local(dist_dir)
        print(f"\nDone. Test with:\n  pip install dist/{whl.name}")
        return

    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    print(f"Building all platform wheels + sdist from {sys.argv[1]} …\n")
    build_all(Path(sys.argv[1]), dist_dir)

if __name__ == "__main__":
    main()
