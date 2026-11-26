# Servo for Haiku

The [Servo](https://servo.org) web engine on Haiku and Haiku-based systems, as a shared library with a native `BView`.

This is the Servo port of [R5 Operating System](https://github.com/r5-operating-system/r5os), published on its own so every Haiku-based system can use it. R5 runs its browser, Aomi, on it and builds its `r5servo` package from this repository. The state here matches R5 `main` at merge `0c0f50181c` (Aomi part 2, 26 November 2026).

## What is in it

| Path | What |
|---|---|
| `patches/` | The changes Servo 0.8.0 and its crates need on Haiku, against the pinned crates.io versions. `patches/WHY` has one line per patch. |
| `haiku-patches/` | The change to Haiku's `runtime_loader` and `libroot` that SpiderMonkey needs: initial-exec thread-local storage, and the fixes for the races it exposed. `haiku-patches/WHY` says what it does. |
| `engine/` | The Rust side: Servo 0.8.0 as a static library with a small C interface (`r5_servo_abi.h`). `Cargo.lock` pins every crate. |
| `BServoView.h`, `BServoView.cpp` | A `BView` that shows a Servo page, drawn through EGL/Zink. |
| `r5-servo-test.cpp` | A small test window. |
| `scripts/build-r5servo.sh` | Builds `libr5servo.so` (Rust staticlib plus the view, linked with Haiku's g++) and `r5-servo-test`. |
| `pack-r5servo.py`, `collect-servo-notices.py` | Pack the library and header into an hpkg, with every licence notice and the source offer. |
| `docs/BUILDING.md` | How to build on a Linux host with a Haiku cross toolchain. |
| `docs/R5-BUILD.md` | R5's own notes: the API history, times and disk use on R5's build machine. |
| `LICENSES/` | The licence texts of this repository and of what `libr5servo.so` contains. |

## Build

On a Linux host with a Haiku x86_64 cross toolchain, Rust 1.99.1 (rustup, with `rust-src`) and LLVM's `libclang`:

```sh
HAIKU=~/haiku LIBCLANG_PATH=~/llvm/lib scripts/build-r5servo.sh
HAIKU=~/haiku ./pack-r5servo.py
```

`HAIKU` is a Haiku build tree whose cross tools and host `package` tool are built; `HAIKU_CROSS_TOOLS` and `HAIKU_PACKAGE` name those pieces one by one instead. The crates come from crates.io and are patched by the script; cargo runs with `--locked`. Everything else, including how long the first build takes, is in [docs/BUILDING.md](docs/BUILDING.md).

## Prebuilt package

Each [release](https://github.com/r5-operating-system/servo-haiku/releases) has `r5servo-<version>-x86_64.hpkg` with `libr5servo.so`, `BServoView.h`, `THIRD_PARTY_NOTICES.txt` and `SOURCE_OFFER.txt`, and its sha256. It is for x86_64. The library loads Mesa's `libGL` at run time (Zink).

On plain Haiku the `runtime_loader` and `libroot` need `haiku-patches/runtime_loader-tls.patch` for this library. R5 has that change built in.

## Status

Experimental. Tested on R5 in QEMU on x86_64 (Zink on Lavapipe): pages load, HTTP/2 works. Not yet tested on stock Haiku or on real GPU hardware. Servo itself is version 0.8, not 1.0.

## Licences

- R5's own code here (`BServoView`, `r5_servo_abi.h`, `engine/src`, the scripts) is **MIT** (`LICENSES/MIT.txt`).
- Each file in `patches/` changes a third-party project and is under that project's licence: Servo and `mozjs_sys` are MPL-2.0 (`LICENSES/MPL-2.0.txt`); surfman, ipc-channel and gaol are MIT/Apache-2.0 (surfman also offers MPL-2.0).
- `haiku-patches/` changes Haiku's MIT code; the R5 parts are MIT and marked with SPDX snippet tags.
- A built `libr5servo.so` contains Servo, SpiderMonkey and about 640 crates under MPL-2.0 and permissive licences, no GPL or LGPL. The package lists each with its licence and copyright files; the texts are in `LICENSES/`.

## Made with AI

This port was written with AI assistance, and checked by building and booting it. That is why it is here and not in Servo or Haiku: both do not accept contributions made with large language models. Use it, fork it and improve it under the licences above.

## Updates

Servo releases monthly. R5 follows those releases here first, then pins the new package.
