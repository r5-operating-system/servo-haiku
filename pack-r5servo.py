#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 R5OS Contributors
# SPDX-License-Identifier: MIT
"""Pack the built libr5servo.so and BServoView.h into an x86_64 .hpkg.

Usage: pack-r5servo.py [out-dir]   (default: $WORK/r5servo-out)

Environment (docs/BUILDING.md):
  WORK               the build directory of scripts/build-r5servo.sh
                     (SERVO_SRC is the older name; default build/ here).
  HAIKU_PACKAGE      Haiku's host `package` tool. A Haiku build tree has it at
                     $HAIKU/generated.x86_64/objects/linux/x86_64/release/tools/package/package,
                     so setting HAIKU instead is enough.
  HAIKU_CROSS_TOOLS  the cross toolchain, for x86_64-unknown-haiku-strip
                     (default $HAIKU/generated.x86_64/cross-tools-x86_64).
  CARGO_HOME         as for collect-servo-notices.py.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent
WORK = Path(os.environ.get("WORK") or os.environ.get("SERVO_SRC") or str(REPO / "build"))
HAIKU = os.environ.get("HAIKU")
CROSS = os.environ.get("HAIKU_CROSS_TOOLS") or (
    f"{HAIKU}/generated.x86_64/cross-tools-x86_64" if HAIKU else None)
PKG = os.environ.get("HAIKU_PACKAGE") or (
    f"{HAIKU}/generated.x86_64/objects/linux/x86_64/release/tools/package/package"
    if HAIKU else None)
STRIP = Path(CROSS) / "bin/x86_64-unknown-haiku-strip" if CROSS else None
SO_SRC = WORK / "r5servo-out" / "libr5servo.so"

VERSION = "0.8.0-6"
PKG_NAME = f"r5servo-{VERSION}-x86_64.hpkg"
SOURCE = "https://github.com/r5-operating-system/servo-haiku"

PACKAGE_INFO = f"""name\t\t\tr5servo
version\t\t\t{VERSION}
architecture\t\tx86_64
summary\t\t\t"Servo web engine shared library for Haiku"
description\t\t"Servo 0.8.0 and SpiderMonkey 153 built as libr5servo.so, with BServoView, from {SOURCE}."
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

SOURCE_OFFER = f"""Source code of libr5servo.so (r5servo {VERSION})

libr5servo.so contains Servo 0.8.0, SpiderMonkey 153 (mozjs_sys 153.6.0-1) and
other files under the Mozilla Public License 2.0, built with changes for Haiku.
The source code of all of it, in the form this library was built from, is
available at no charge:

  {SOURCE}
  (release r5servo-{VERSION}, and the git tag of the same name)

That repository has the exact crate versions (engine/Cargo.lock), every
change to them (patches/, with patches/WHY), the C++ view and the build
script (scripts/build-r5servo.sh), which downloads the unmodified sources from
crates.io and applies the changes. The MPL-2.0 text is LICENSES/MPL-2.0.txt
there. The licence and copyright notices of every part of this library are in
THIRD_PARTY_NOTICES.txt next to this file.
"""


def main() -> None:
    hdr_src = REPO / "BServoView.h"
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else (WORK / "r5servo-out")
    out_dir.mkdir(parents=True, exist_ok=True)
    dest = out_dir / PKG_NAME

    if not SO_SRC.is_file():
        raise SystemExit(f"missing {SO_SRC}; run scripts/build-r5servo.sh first")
    if not PKG or not Path(PKG).is_file():
        raise SystemExit("set HAIKU_PACKAGE (or HAIKU) to Haiku's host package tool")
    if not STRIP or not STRIP.is_file():
        raise SystemExit("set HAIKU_CROSS_TOOLS (or HAIKU); x86_64-unknown-haiku-strip is needed")

    with tempfile.TemporaryDirectory(prefix="r5servo-pkg-") as tmp:
        stage = Path(tmp)
        (stage / "lib").mkdir(parents=True)
        (stage / "develop" / "headers" / "r5").mkdir(parents=True)
        (stage / "develop" / "lib").mkdir(parents=True)

        so_dst = stage / "lib" / "libr5servo.so"
        shutil.copy2(SO_SRC, so_dst)
        subprocess.check_call([str(STRIP), "--strip-unneeded", str(so_dst)])
        os.chmod(so_dst, 0o755)
        os.symlink("../../lib/libr5servo.so", stage / "develop" / "lib" / "libr5servo.so")

        hdr_dst = stage / "develop" / "headers" / "r5" / "BServoView.h"
        hdr_dst.write_bytes(hdr_src.read_bytes().replace(b"\r\n", b"\n"))

        # Every licence, copyright and NOTICE file of what is inside the
        # library travels with it (MIT, BSD, Apache-2.0, ISC, Zlib and
        # OpenSSL ask that), and so does where its MPL-2.0 source is.
        docs = stage / "documentation" / "packages" / "r5servo"
        docs.mkdir(parents=True)
        subprocess.check_call([
            sys.executable,
            str(REPO / "collect-servo-notices.py"),
            str(REPO / "engine" / "Cargo.lock"),
            str(docs / "THIRD_PARTY_NOTICES.txt"),
        ])
        (docs / "SOURCE_OFFER.txt").write_text(SOURCE_OFFER, encoding="utf-8", newline="\n")

        (stage / ".PackageInfo").write_text(PACKAGE_INFO, encoding="utf-8", newline="\n")

        if dest.exists():
            dest.unlink()
        subprocess.check_call([str(PKG), "create", "-C", str(stage), str(dest)])
    digest = hashlib.sha256(dest.read_bytes()).hexdigest()
    size = dest.stat().st_size
    print(f"created {dest} ({size} bytes, {size / (1024 * 1024):.2f} MiB)")
    print(f"sha256 {digest}  {PKG_NAME}")


if __name__ == "__main__":
    main()
