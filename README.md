# Servo for Haiku

The [Servo](https://servo.org) web engine on Haiku and Haiku-based systems, as a shared library with a native `BView`.

This is the Servo port of [R5 Operating System](https://github.com/r5-operating-system/r5os), published on its own so every Haiku-based system can use it. R5 runs its browser, Aomi, on it. The state here is exported from R5 commit `0be5c00cc6` (26 November 2026).

## What is in it

| Path | What |
|---|---|
| `patches/` | The changes Servo 0.8.0 and its crates need on Haiku, against the pinned crates.io versions. `patches/WHY` has one line per patch. |
| `haiku-patches/` | A change to Haiku's `runtime_loader` (initial-exec TLS) that SpiderMonkey needs. It applies to Haiku's `src/system/runtime_loader/elf_tls.{h,cpp}`. |
| `engine/` | The Rust side: Servo 0.8.0 as a static library with a small C interface (`r5_servo_abi.h`). `Cargo.lock` pins every crate. |
| `BServoView.h`, `BServoView.cpp` | A `BView` that shows a Servo page, drawn through EGL/Zink. |
| `r5-servo-test.cpp` | A small test window. |
| `scripts/build-r5servo.sh` | Builds `libr5servo.so`: Rust staticlib plus the view, linked with Haiku's g++. |
| `pack-r5servo.py` | Packs the library and header into an hpkg. |
| `docs/R5-BUILD.md` | R5's own build notes: versions, times, disk use. |

## Status

Experimental. Tested in QEMU on x86_64 (Zink on Lavapipe): pages load, HTTP/2 works. Not yet tested on real GPU hardware. Servo itself is version 0.8, not 1.0.

The build script still expects R5's build host: a Haiku cross toolchain, Rust 1.99.1 with the Haiku target, and LLVM in fixed places (see `docs/R5-BUILD.md`). Making it run on any Haiku build host is the next step.

## Licences

- R5's own code here (`BServoView`, `r5_servo_abi.h`, `engine/src`, the scripts) is **MIT** (`LICENSES/MIT.txt`).
- Each file in `patches/` changes a third-party project and is under that project's licence: Servo and `mozjs_sys` are MPL-2.0 (`LICENSES/MPL-2.0.txt`); surfman, ipc-channel and gaol are MIT/Apache-2.0 (surfman also offers MPL-2.0).
- `haiku-patches/` changes Haiku's MIT code; the R5 parts are MIT.
- A built `libr5servo.so` contains Servo, SpiderMonkey and about 640 crates. R5's licence report lists them: MPL-2.0 and permissive licences, no GPL or LGPL.

## Made with AI

This port was written with AI assistance, and checked by building and booting it. That is why it is here and not in Servo or Haiku: both do not accept contributions made with large language models. Use it, fork it and improve it under the licences above.

## Updates

Servo releases monthly. R5 follows those releases here first.
