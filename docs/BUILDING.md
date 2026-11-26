<!-- SPDX-License-Identifier: MIT -->
<!-- SPDX-FileCopyrightText: 2026 R5OS Contributors -->

# Building libr5servo

`scripts/build-r5servo.sh` builds `libr5servo.so` and `r5-servo-test` for x86_64 Haiku on a Linux build host. `pack-r5servo.py` packs the library into an hpkg. Neither needs R5's tree.

## What the host needs

| Piece | Version used | How to get it |
|---|---|---|
| Haiku x86_64 cross toolchain (`x86_64-unknown-haiku-gcc`, `g++`, `ar`, `strip`, and its `sysroot/` with Haiku's headers and libraries) | GCC 13.3.0 | A Haiku build tree: `configure --cross-tools-source <buildtools> --build-cross-tools x86_64`, then a jam build of an image (`jam -q @nightly-anyboot`). The image build links Haiku's libraries (`libbe.so`, `libroot.so`, …) into `sysroot/boot/system/develop/lib`, which the link of `libr5servo.so` needs. The toolchain is then `<tree>/generated.x86_64/cross-tools-x86_64`. |
| Haiku's host `package` tool (only for `pack-r5servo.py`) | from the same tree | Built by that image build, at `generated.x86_64/objects/linux/x86_64/release/tools/package/package`. |
| Rust through rustup, with `rust-src` | 1.99.1 | `rustup toolchain install --profile minimal -c rust-src 1.99.1`. The Haiku standard library is built from source (`-Zbuild-std`), so no Haiku target component is needed. |
| LLVM's `libclang` (bindgen) | 21.1.8 | The release tarball `LLVM-21.1.8-Linux-X64.tar.xz` from github.com/llvm/llvm-project; `LIBCLANG_PATH` is its `lib/`. |
| Host tools | | `gcc`, `g++` (SpiderMonkey's host tools), `python3` (3.11 or newer), `curl`, `patch`, `pkg-config`, `make`. `/usr/bin/time` is used for the timings if it is there. |
| Disk and memory | | About 20 GB for `$WORK`. R5 builds with 4 jobs in a WSL machine with 15 GB of memory. |

## Variables

| Variable | Meaning | Default |
|---|---|---|
| `HAIKU_CROSS_TOOLS` | The cross toolchain directory (has `bin/` and `sysroot/`). | `$HAIKU/generated.x86_64/cross-tools-x86_64` |
| `HAIKU` | A Haiku build tree. Enough instead of `HAIKU_CROSS_TOOLS` and `HAIKU_PACKAGE`. | |
| `HAIKU_PACKAGE` | Haiku's host `package` tool, for `pack-r5servo.py`. | from `HAIKU` |
| `LIBCLANG_PATH` | Directory with `libclang.so`. Its `../bin` goes on `PATH`. | `$WORK/llvm/lib` |
| `WORK` | Build directory: the patched crates, cargo's `target/` (10 GB and more), `r5servo-out/`, the log. | `build/` in this repository |
| `RUST_TOOLCHAIN` | rustup toolchain name. | `1.99.1` |
| `RUSTUP_HOME`, `CARGO_HOME` | rustup and cargo homes. The crates are downloaded into `CARGO_HOME`. | `$WORK/rust/rustup` and `$WORK/rust/cargo` if they exist, else `~/.rustup` and `~/.cargo` |
| `JOBS` | Parallel jobs for cargo. | `4` |
| `EXTRA_PATH` | Put in front of `PATH`, for example a `pkg-config` or rustup's `cargo` when `CARGO_HOME/bin` has none. | |
| `SERVO_REFRESH_PATCHES=1` | Unpack and patch the crates again. This rebuilds SpiderMonkey. | |

In a Haiku build tree the sysroot's `develop/headers` and `develop/lib` are links into jam's packaging directory. Do not run a jam on that tree while this build runs: when jam repacks `haiku_devel`, headers such as `stdint.h` and `unistd.h` are briefly missing and SpiderMonkey fails with `fatal error: stdint.h: No such file or directory`. A copy of the toolchain with a real sysroot does not have this problem; the toolchain is relocatable:

```sh
X=~/haiku-cross-x86_64
cp -a ~/haiku/generated.x86_64/cross-tools-x86_64/. $X/
rm -rf $X/sysroot && cp -rL ~/haiku/generated.x86_64/cross-tools-x86_64/sysroot $X/sysroot
HAIKU_CROSS_TOOLS=$X LIBCLANG_PATH=~/llvm/lib scripts/build-r5servo.sh
```

The script sets `PATH` itself to `EXTRA_PATH`, LLVM, the cross tools, `/usr/bin`, `/bin` and `CARGO_HOME/bin`, so other compilers on the host do not leak in.

## Steps

```sh
git clone https://github.com/r5-operating-system/servo-haiku.git
cd servo-haiku
HAIKU=~/haiku LIBCLANG_PATH=~/llvm/lib scripts/build-r5servo.sh
HAIKU=~/haiku ./pack-r5servo.py
```

The build script:

1. writes `$WORK/bindgen-clang-args.txt`: the Haiku C++ and system header directories of the cross toolchain, for bindgen's host libclang (`patches/mozjs_sys-153.6.0-1-haiku.patch` reads it through `R5_BINDGEN_CLANG_ARGS_FILE`);
2. downloads the eight patched crates from `static.crates.io` at their pinned versions and applies `patches/` (`patch -p1`); a tree that is already there is kept, so SpiderMonkey is not rebuilt;
3. copies `engine/` to `$WORK/r5servo-engine` with the `[patch]` paths pointing at those trees, and writes its `.cargo/config.toml` (target `x86_64-unknown-haiku`, the cross compiler as linker);
4. runs `cargo fetch --locked`, then `cargo build --locked -Zbuild-std=std,panic_unwind,panic_abort --release --target x86_64-unknown-haiku`: exactly the versions in `engine/Cargo.lock`;
5. links `libr5servo.so` with the cross g++ from the staticlib and `BServoView.cpp` (`--whole-archive`, `--allow-multiple-definition` for two copies of lz4; NEEDED: `libbe`, `libtracker`, `libstdc++`, `libz`, `libbsd`, `libnetwork`, `libroot`, `libgcc_s`), and `r5-servo-test` against it.

The products are in `$WORK/r5servo-out`. `libr5servo.so` is about 630 MB before `strip --strip-unneeded` and about 140 MB after; `pack-r5servo.py` strips it.

`pack-r5servo.py` writes `r5servo-<version>-x86_64.hpkg` into `$WORK/r5servo-out` with:

- `lib/libr5servo.so` (stripped) and `develop/lib/libr5servo.so` (link),
- `develop/headers/r5/BServoView.h`,
- `documentation/packages/r5servo/THIRD_PARTY_NOTICES.txt`: the SPDX expression and every licence, copyright and NOTICE file of each crate and of the C and C++ code bundled in SpiderMonkey and aws-lc (`collect-servo-notices.py`, from `CARGO_HOME`'s registry and the patched trees),
- `documentation/packages/r5servo/SOURCE_OFFER.txt`: where the MPL-2.0 source is.

It prints the size and sha256.

## Environment the build sets

`RUSTC_BOOTSTRAP=1` (for `-Zbuild-std`), `MOZJS_FROM_SOURCE=1`, `PKG_CONFIG_ALLOW_CROSS=1`, `HOST_CC=/usr/bin/gcc` and `HOST_CXX=/usr/bin/g++` (the cross compiler is not the host compiler), `ZSTD_SYS_NO_HIDE_SYMBOLS=1` (`libbe.so` needs the public zstd symbols), and `CPPFLAGS`/`CFLAGS_x86_64_unknown_haiku` with the zlib header of `libz-sys` 1.1.33: SpiderMonkey's system wrapper does `include_next <zlib.h>`, and the Haiku sysroot has `libz.so` without the header.

## Same library as R5

R5 builds its `r5servo` package with this script, `WORK` set to its build directory; that build is byte-equal to the library in the release. The library depends on the paths of `WORK` and `CARGO_HOME`: Rust keeps source paths in panic messages and derives crate hashes, and so symbol names and code layout, from the paths of the patched crates. A build in another directory is the same source with a different layout and another sha256.

## Measured

26 Nov 2026, on R5's build machine (WSL, 12 threads, 15 GB), `JOBS=4`, from an empty `WORK` and `CARGO_HOME`: `cargo fetch` 10 s, `cargo build` 1326 s (22 minutes, SpiderMonkey included), link of `libr5servo.so` 14 s. `WORK` was 9.8 GB afterwards, `CARGO_HOME` 0.8 GB. An incremental build after a change in `engine/src/lib.rs` or `BServoView.cpp` takes under a minute.

## Using it on Haiku

`libr5servo.so` uses initial-exec thread-local storage. Haiku's `runtime_loader` needs `haiku-patches/runtime_loader-tls.patch` (against Haiku `master` of November 2026; it applies with `patch -p1` in the Haiku tree) before the library loads. R5 has that change.

Run `r5-servo-test` with `GALLIUM_DRIVER=zink`; with `LIBGL_ALWAYS_SOFTWARE=1` that is Zink on Lavapipe. `R5_SERVO_PROOF=1` loads a red `data:` page, then an HTTP/2 test page.
