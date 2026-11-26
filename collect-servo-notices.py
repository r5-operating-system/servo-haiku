#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 R5OS Contributors
# SPDX-License-Identifier: MIT
"""Collect the licence and copyright files of everything inside libr5servo.so.

MIT, BSD, Apache-2.0, ISC, Zlib and the OpenSSL licence all ask that the
copyright notice and licence text travel with the binary. This writes one
text file with, for each crate in engine/Cargo.lock, its SPDX
expression and every LICENSE/COPYING/NOTICE file it ships, followed by the
licence files of the C and C++ code bundled in SpiderMonkey (mozjs_sys) and
aws-lc. pack-r5servo.py puts the file in the r5servo package.

Usage: collect-servo-notices.py <Cargo.lock> <out.txt>
Environment: WORK, the build directory of scripts/build-r5servo.sh with the
patched trees (SERVO_SRC is the older name), and CARGO_HOME, whose registry
holds the crates (default $WORK/rust/cargo if it exists, else ~/.cargo).
"""
import os
import re
import sys
import tomllib
from collections import Counter
from pathlib import Path

SERVO_SRC = Path(os.environ.get("WORK") or os.environ.get("SERVO_SRC")
                 or str(Path(__file__).resolve().parent / "build"))
if os.environ.get("CARGO_HOME"):
    CARGO_HOME = Path(os.environ["CARGO_HOME"])
elif (SERVO_SRC / "rust/cargo").is_dir():
    CARGO_HOME = SERVO_SRC / "rust/cargo"
else:
    CARGO_HOME = Path.home() / ".cargo"
REGISTRY = CARGO_HOME / "registry/src"
# Patched trees that replace a crates.io crate (patches/).
PATCHED = {
    "mozjs_sys": "mozjs-sys",
    "surfman": "surfman",
    "ipc-channel": "ipc-channel",
    "servo-fonts": "servo-fonts",
    "servo-fonts-traits": "servo-fonts-traits",
    "gaol": "gaol",
    "servo-script": "servo-script",
    "servo-net": "servo-net",
}
NOTICE_NAME = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|AUTHORS|UNLICENSE)", re.I)
# Build-time tools (Python packages, PGO data, devtools) do not end up in the
# library.
SKIP_DIRS = {"test", "tests", "testing", "testsuite", "examples", "benches", "docs", "fuzz",
             "python", "pgo", "devtools", "pkg"}


def registry_dirs() -> list[Path]:
    if not REGISTRY.is_dir():
        raise SystemExit(f"missing cargo registry {REGISTRY}")
    return [d for d in REGISTRY.iterdir() if d.is_dir()]


def crate_dir(name: str, version: str, roots: list[Path]) -> Path | None:
    if name in PATCHED:
        path = SERVO_SRC / PATCHED[name]
        if path.is_dir():
            return path
    for root in roots:
        path = root / f"{name}-{version}"
        if path.is_dir():
            return path
    return None


def top_notices(path: Path) -> list[Path]:
    return sorted(p for p in path.iterdir() if p.is_file() and NOTICE_NAME.match(p.name))


def deep_notices(path: Path) -> list[Path]:
    found = []
    for base, dirs, files in os.walk(path):
        dirs[:] = sorted(d for d in dirs if d.lower() not in SKIP_DIRS and not d.startswith("."))
        for name in sorted(files):
            if NOTICE_NAME.match(name):
                found.append(Path(base) / name)
    return found


def read(path: Path) -> str:
    return path.read_bytes().decode("utf-8", errors="replace").rstrip() + "\n"


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    lock = tomllib.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    out = Path(sys.argv[2])
    roots = registry_dirs()
    licences: Counter[str] = Counter()
    missing = []
    parts = [
        "Third-party notices for libr5servo.so (the r5servo package)\n",
        "Servo, SpiderMonkey and their dependencies, built for Haiku.\n",
        "Source offer: the pinned versions (engine/Cargo.lock), every change\n",
        "(patches/) and the build script are in github.com/r5-operating-system/servo-haiku;\n",
        "scripts/build-r5servo.sh fetches the unmodified sources from crates.io.\n",
    ]
    packages = sorted(lock.get("package", []), key=lambda p: (p["name"], p["version"]))
    bundled = {}
    for pkg in packages:
        name, version = pkg["name"], pkg["version"]
        if name == "r5servo":
            continue
        path = crate_dir(name, version, roots)
        if path is None:
            missing.append(f"{name} {version}")
            continue
        spdx = "unknown"
        authors = []
        manifest = path / "Cargo.toml"
        if manifest.is_file():
            meta = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
            spdx = meta.get("license") or ("see licence file" if meta.get("license-file") else "unknown")
            authors = [a for a in meta.get("authors", []) if isinstance(a, str)]
        licences[spdx] += 1
        parts.append("\n" + "=" * 78 + f"\n{name} {version}\nLicense: {spdx}\n")
        if authors:
            # The Cargo authors field is not a copyright statement.
            parts.append("Authors (Cargo.toml): " + ", ".join(authors) + "\n")
        notices = top_notices(path)
        for notice in notices:
            parts.append(f"\n--- {notice.name} ---\n{read(notice)}")
        if not notices:
            # The crate ships only the SPDX line in its Cargo.toml; the
            # licence text itself is in this repository's LICENSES/.
            parts.append(
                "\n(This crate ships no licence file. The text of the licence "
                "above is in LICENSES/ of the servo-haiku repository and in "
                "/boot/system/data/licenses.)\n")


        if name in ("mozjs_sys", "aws-lc-sys"):
            bundled[name] = path
    for name, path in bundled.items():
        parts.append("\n" + "=" * 78 + f"\nC and C++ code bundled in {name}\n")
        for notice in deep_notices(path):
            rel = notice.relative_to(path)
            if len(rel.parts) == 1:
                continue
            parts.append(f"\n--- {name}/{rel} ---\n{read(notice)}")
    out.write_text("".join(parts), encoding="utf-8")
    print(f"wrote {out} ({out.stat().st_size} bytes, {sum(licences.values())} crates)")
    for spdx, count in licences.most_common():
        print(f"  {count:4d}  {spdx}")
    # Cargo.lock lists the crates of every platform. Cargo never downloads
    # the ones that are not built for Haiku (Windows, macOS, Android, wasm).
    if missing:
        print(f"  {len(missing)} crates not built for Haiku (not downloaded)")


if __name__ == "__main__":
    main()
