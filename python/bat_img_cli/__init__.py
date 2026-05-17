"""
bat_img_cli — thin Python shim that forwards all arguments to the
bundled bat_img Rust binary.

The binary is stored next to this file in the `bin/` subdirectory,
named `bat_img` (macOS/Linux) or `bat_img.exe` (Windows).
"""

import os
import sys
import subprocess
from pathlib import Path


def _binary_path() -> Path:
    here = Path(__file__).parent
    name = "bat_img.exe" if sys.platform == "win32" else "bat_img"
    candidate = here / "bin" / name
    if not candidate.exists():
        raise FileNotFoundError(
            f"bat_img binary not found at {candidate}.\n"
            "The package may have been installed for the wrong platform."
        )
    return candidate


def main() -> None:
    binary = _binary_path()
    # Replace the current process with the binary so signals, exit codes,
    # and stdin/stdout all behave as if the user ran bat_img directly.
    if sys.platform != "win32":
        os.execv(binary, [str(binary)] + sys.argv[1:])
    else:
        result = subprocess.run([str(binary)] + sys.argv[1:])
        sys.exit(result.returncode)
