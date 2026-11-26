#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 R5OS Contributors
# SPDX-License-Identifier: MIT
"""Package prebuilt libr5servo.so and BServoView.h into an x86_64 .hpkg."""
import os
import shutil
import subprocess
import sys
from pathlib import Path

HAIKU = Path(os.environ.get("HAIKU", "/home/nickr/r5os/haiku"))
SERVO_SRC = Path(os.environ.get("SERVO_SRC", "/home/nickr/r5os/servo-build"))
PKG = HAIKU / "generated.x86_64/objects/linux/x86_64/release/tools/package/package"
STRIP = HAIKU / "generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-strip"
SO_SRC = SERVO_SRC / "r5servo-out" / "libr5servo.so"

VERSION = "0.8.0-1"
PKG_NAME = f"r5servo-{VERSION}-x86_64.hpkg"

PACKAGE_INFO = f"""name\t\t\tr5servo
version\t\t\t{VERSION}
architecture\t\tx86_64
summary\t\t\t"Servo web engine shared library for R5"
description\t\t"Servo 0.8.0 and SpiderMonkey 153 built as libr5servo.so for R5 Operating System."
packager\t\t"R5OS Contributors"
vendor\t\t\t"R5OS"
licenses {{
\t"MPL v2.0"
\t"MIT"
\t"Apache v2"
}}
copyrights {{
\t"2026 R5OS Contributors"
\t"Servo Project Developers"
\t"Mozilla Foundation"
}}
provides {{
\tr5servo = 0.8.0
\tlib:libr5servo = 0.8.0
\tdevel:libr5servo = 0.8.0
}}
requires {{
\thaiku
\tlib:libstdc++
\tlib:libz
\tlib:libgcc_s
}}
"""


def main() -> None:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    hdr_src = root / "distro" / "servo" / "BServoView.h"
    out_dir = root / "distro" / "packages" / "x86_64"
    out_dir.mkdir(parents=True, exist_ok=True)
    dest = out_dir / PKG_NAME

    if not SO_SRC.is_file():
        raise SystemExit(f"missing {SO_SRC}")
    if not hdr_src.is_file():
        raise SystemExit(f"missing {hdr_src}")
    if not PKG.is_file():
        raise SystemExit(f"missing {PKG}")

    stage = Path("/tmp/r5-servo-pkg")
    shutil.rmtree(stage, ignore_errors=True)
    (stage / "lib").mkdir(parents=True)
    (stage / "develop" / "headers" / "r5").mkdir(parents=True)
    (stage / "develop" / "lib").mkdir(parents=True)

    so_dst = stage / "lib" / "libr5servo.so"
    shutil.copy2(SO_SRC, so_dst)
    if STRIP.is_file():
        subprocess.check_call([str(STRIP), "--strip-unneeded", str(so_dst)])
    os.chmod(so_dst, 0o755)
    os.symlink("../../lib/libr5servo.so", stage / "develop" / "lib" / "libr5servo.so")

    hdr_dst = stage / "develop" / "headers" / "r5" / "BServoView.h"
    hdr_dst.write_bytes(hdr_src.read_bytes().replace(b"\r\n", b"\n"))

    (stage / ".PackageInfo").write_text(PACKAGE_INFO, encoding="utf-8", newline="\n")

    if dest.exists():
        dest.unlink()
    subprocess.check_call([str(PKG), "create", "-C", str(stage), str(dest)])
    size_mb = dest.stat().st_size / (1024 * 1024)
    print(f"created {dest} ({dest.stat().st_size} bytes, {size_mb:.2f} MiB)")


if __name__ == "__main__":
    main()
