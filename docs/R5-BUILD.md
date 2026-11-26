<!-- SPDX-License-Identifier: MIT -->
<!-- SPDX-FileCopyrightText: 2026 R5OS Contributors -->

# Building libr5servo

This is the instruction for the next agent. Do not ask where the trees are. Do not invent a second build.

`scripts/build-r5servo.sh` is the only recipe. It does not write the shared Haiku tree, it does not run jam, and it does not install the library into an image. Job cap: `-j4`.

## For the next agent

Work on branch `servo-proof` in the worktree `C:\Users\nickr\Projects\R5OS-servo-proof` (WSL: `/mnt/c/Users/nickr/Projects/R5OS-servo-proof`). Leave `C:\Users\nickr\Projects\R5OS` alone. That checkout is dirty and is not this branch.

From WSL, not from PowerShell (`&&` is rejected there):

```sh
cd /mnt/c/Users/nickr/Projects/R5OS-servo-proof
sed -i 's/\r$//' scripts/build-r5servo.sh scripts/run-r5servo-qemu.sh
scripts/build-r5servo.sh
```

The script downloads a missing third-party tree from the pinned crates.io tarball and applies `distro/servo/patches/`. One line per patch, saying why, is `distro/servo/patches/WHY`. An existing tree is left in place. `R5_SERVO_REFRESH_PATCHES=1` replaces it, and that rebuilds SpiderMonkey. Rust 1.99.1, LLVM, the Haiku cross compiler, and `servo.image` are still only on this machine. The script stops and names the path if one of those is missing.

After a successful `BUILD_OK`, strip before copying onto an image. An unstripped library is about 634M; stripped it is about 144M.

```sh
/home/nickr/r5os/haiku/generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-strip \
  --strip-unneeded /home/nickr/r5os/servo-build/r5servo-out/libr5servo.so
```

Boot, on a private copy of the proof image. This does not call `scripts/run-qemu-r5.sh` and it does not stop another session's QEMU. It refuses only when a guest named `r5-servo-proof` is already running.

```sh
SECONDS_UP=360 scripts/run-r5servo-qemu.sh
```

Do not set `LIBRARY_PATH` in the guest. On Haiku that replaces the default search path, and then `libbe.so` is not found. `non-packaged/lib` is already on the default path.

Do not add `libr5servo.so` to an image list, What's new, or `distro/BUILD`. Do not commit `.vendor`, `.cargo`, `target`, `cc.sh`, the patched trees, or `mozjs-sys`.

## What is in git

| Path | What it is |
|---|---|
| `distro/servo/BServoView.h`, `BServoView.cpp` | The view. Round 1 swaps the base class to Aomi's `BWebContentView` and deletes the copied message codes. |
| `distro/servo/r5_servo_abi.h` | Private C ABI. Not an Aomi header. |
| `distro/servo/r5-servo-test.cpp` | One window. `R5_SERVO_PROOF=1` loads a red `data:` document, then `https://http2.pro/api/v1`. |
| `distro/servo/engine/` | Rust staticlib `r5servo`. `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `build.rs`. |
| `scripts/build-r5servo.sh` | Rebuild. |
| `scripts/run-r5servo-qemu.sh` | Private QEMU boot. |
| `distro/rest/src/system/runtime_loader/elf_tls.cpp` | `get_tls_address()` returns the static initial-exec image. Servo needs it. The proof image was not rebuilt with this change. |

## What stays on this machine

`SERVO_SRC` defaults to `/home/nickr/r5os/servo-build`. None of this is in git.

| Path | What it is |
|---|---|
| `rust/` | Rust 1.99.1. `RUSTUP_HOME` is `rust/rustup`, `CARGO_HOME` is `rust/cargo`. Cargo is `cargo +1.99.1`. |
| `llvm/lib` | `LIBCLANG_PATH` for bindgen. |
| `mozjs-sys`, `surfman`, `ipc-channel`, `gaol`, `servo-fonts`, `servo-fonts-traits`, `servo-script` | Patched trees. Not git repos. Missing ones are recreated from crates.io plus `distro/servo/patches/`. The recipe rewrites `[patch]` to these absolute paths so cargo reuses the SpiderMonkey it already built. |
| `target/` | Cargo target dir. About 16G once SpiderMonkey is built. |
| `r5servo-engine/` | Staged copy of `distro/servo/engine` with absolute patch paths. The script deletes and rewrites it. |
| `r5servo-out/libr5servo.so`, `r5servo-out/r5-servo-test` | The products. |
| `r5servo-build.log` | Build log. |
| `bindgen-clang-args.txt` | Extra clang args for bindgen. |
| `servo.image` | The proof image (about 2G, from 26 Nov 2026 00:29 Europe/Amsterdam). Do not boot `haiku/generated.x86_64/r5-os.image` for this. |
| `r5servo-proof.image` | Private copy made by the QEMU script. |
| `r5servo-qemu.log`, `r5servo-serial.log` | Boot log and serial log. The guest log on the image is `/boot/home/r5-servo.log`. |

The Haiku tree used only for the cross compiler is `/home/nickr/r5os/haiku`. The compiler is `generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-g++` (and `gcc`, `ar`, `strip`). `bfs_shell` is `generated.x86_64/objects/linux/x86_64/release/tools/bfs_shell/bfs_shell`.

Rebuilding Rust or SpiderMonkey from nothing, only when those trees are gone: `SERVO_SRC/build-hello.sh`, then `build-mozjs.sh`, then `build-servo.sh`, then `scripts/build-r5servo.sh`. A normal change of `lib.rs` or `BServoView.cpp` does not need that.

## Pins

| Piece | Pin |
|---|---|
| Rust | 1.99.1, `cargo +1.99.1 build -Zbuild-std=std,panic_unwind,panic_abort --target x86_64-unknown-haiku` |
| Servo | crates.io `servo` 0.8.0, default features off, `js_jit` and `bundled` on |
| SpiderMonkey | `mozjs_sys` 153.6.0-1, built from source (`MOZJS_FROM_SOURCE=1`) |
| surfman | 0.14.0, patched tree |
| ipc-channel | 0.23.1, patched tree |
| gaol | 0.2.1, patched tree |
| servo-fonts / servo-fonts-traits / servo-script | 0.8.0, patched trees |
| freetype-sys | 0.23.0, feature `bundled` |

SpiderMonkey's `include_next <zlib.h>` needs the vendored header from `libz-sys-1.1.33` under `CARGO_HOME`. The script exports that include path. The Haiku sysroot has `libz.so` and no header.

## Patches, and why

Each file in `distro/servo/patches/` applies with `patch -p1` on the unpacked crates.io tarball of the pinned version. `distro/servo/patches/WHY` is one line per file. The script fetches `https://static.crates.io/crates/<crate>/<crate>-<version>.crate` when the tree is missing.

- **mozjs_sys 153.6.0-1.** SpiderMonkey's configure does not know Haiku, a few POSIX files need it, and bindgen's host libclang must be pointed at the Haiku C++ headers (`R5_BINDGEN_CLANG_ARGS_FILE`).
- **surfman 0.14.0.** Haiku EGL is Mesa's surfaceless platform. The only config is 8/8/8/8 and it has no 24-bit depth or 8-bit stencil, so the context request drops those two. `LIBGL_ALWAYS_SOFTWARE` is left set, so `GALLIUM_DRIVER=zink` reaches Lavapipe. The hardware adapter is the low-power one, so a single GPU is not asked for `DRI_PRIME=1`. There is no native window; the widget's size becomes a pbuffer.
- **ipc-channel 0.23.1.** Haiku has no `memfd`. The in-process backend is the one Android uses.
- **gaol 0.2.1.** Haiku has no sandbox. A profile cannot be built; the child is not jailed.
- **servo-fonts 0.8.0.** Haiku is not in the Linux font list. The build uses the bundled FreeType the Android target already uses, so it does not look for a host `freetype` pkg-config file.
- **servo-fonts-traits 0.8.0.** The font-file identifier is the Unix path code, and Haiku was not in that cfg.
- **servo-script 0.8.0.** `navigator.platform` has no Haiku string, so the script crate did not build for that target.

`HOST_CC` stays the Linux gcc; the cross compiler is not the host compiler.

The shared library is Haiku g++ around the Rust staticlib, with `--whole-archive` and `--allow-multiple-definition` (two copies of lz4). A cdylib hid the C++ symbols. NEEDED libraries: `libstdc++`, `libbe`, `libz`, `libbsd`, `libgcc_s`, `libroot`, `libnetwork`. Mesa `libGL` is opened at runtime.

The first document has to be the web view's initial URL. Creating the engine on `about:blank` and then calling `load()` drops the real document. `LoadURL` from the application thread locks the window before it reads the size.

Destroying the GL context aborts in `eglDestroyImageKHR`. The engine is leaked on purpose.

## Where it lands

`distro/servo/pack-r5servo.py` packs the stripped `libr5servo.so` and `BServoView.h` into `distro/packages/x86_64/r5servo-0.8.0-1-x86_64.hpkg` (~49 MiB). `scripts/pin-mesa-packages.sh` includes it in `r5-opengl.repo` and pins `r5servo-0.8.0-1`, and `distro/UserBuildConfig` adds `r5servo` to `AddHaikuImageSystemPackages`, so jam installs it into the image without running cargo.

- Header: `/boot/system/develop/headers/r5/BServoView.h`
- Library: `/boot/system/lib/libr5servo.so` (soname `libr5servo.so`, symlinked at `/boot/system/develop/lib/libr5servo.so`)
- `scripts/run-r5servo-qemu.sh` supports `R5_SERVO_FROM_PKG=1` to boot a full jam-built image without copying `libr5servo.so` into `non-packaged/`.

`r5-servo-test` links `libr5servo.so`. The guest is started with `GALLIUM_DRIVER=zink`, `LIBGL_ALWAYS_SOFTWARE=1`, `RUST_LOG=net::http_loader=debug`, and `R5_SERVO_PROOF=1`. The GL string that counts is `zink Vulkan 1.4(llvmpipe (LLVM 21.1.8, 256 bits) (MESA_LLVMPIPE))`.

## Measured

26 Nov 2026, Europe/Amsterdam, on this machine, with SpiderMonkey already built. `scripts/build-r5servo.sh`, `-j4`.

| Step | Time |
|---|---|
| `cargo` (incremental, `lib.rs` only) | 16–17 s. An earlier incremental run was 8 s. The first cargo of this crate, still reusing the existing SpiderMonkey artifact, was 577 s. |
| shared link of `libr5servo.so` | 12–15 s |
| link of `r5-servo-test` | under 1 s |
| `target/` | 16G |
| `libr5servo.so` before `strip --strip-unneeded` | 634M |
| `libr5servo.so` after strip | 144M |

A from-scratch SpiderMonkey rebuild was not run for these numbers. Do not start one if `mozjs-sys` is still at `/home/nickr/r5os/servo-build/mozjs-sys`.

## What already works

26 Nov 2026, QEMU KVM, Zink on Lavapipe. `libr5servo.so` and `r5-servo-test` link. The GL string is `zink Vulkan 1.4(llvmpipe (LLVM 21.1.8, 256 bits) (MESA_LLVMPIPE))`. With the first document as the web view's initial URL, the red `data:` page finishes: `load-finished`, body text `R5`. One read-back pixel was `240 3 0 0`, not `255 0 0 255`. The page is loaded; the alpha byte of that read-back is 0.

## What does not work

The guest never shows an HTTP/2 response.

On the boot of 10:16–10:22 Europe/Amsterdam the guest already had `inet 10.0.2.15` (QEMU user net) and `127.0.0.1`. `net-wait` was 0. It requested `https://http2.pro/api/v1`. `net::http_loader` logged the GET at 10:17 Europe/Amsterdam. At 10:18 the log was `dns error` / `failed to lookup address information: No address associated with hostname`. The page text was `Could not load the requested page: client error (Connect)`. There is no response-version line, so `Protocol()` stays `B_NOT_SUPPORTED`.

From the host, the same minute, `curl` got `HTTP/2 301` to `https://nhacaitop10.win/api/v1`. The name resolves here, and the server speaks HTTP/2. The guest's failure is its own resolver, not "the site is down" and not "Servo has no HTTP/2". That host no longer returns the old `{"http2":1}` body. A 301 page will not match the body check in `Protocol()`, even after DNS works.

Also not done, and not the thing that blocked this boot: the `elf_tls.cpp` change is source only (these boots used `servo.image`), the 5625U stick was not written, and `cargo deny check licenses` was not run.

## What was already tried

1. `LIBRARY_PATH` set to only `non-packaged/lib`. Haiku then does not open `libbe.so`. The variable stays unset. `non-packaged/lib` is already on the default path.
2. Waiting for Deskbar and for `libbe.so`. This image does not sign in by itself, and Deskbar never appeared. The log finished `about:blank` only. The `data:` document did not finish, and HTTP/2 was never requested. One pixel during that boot was already `240 3 0 0`.
3. Creating the engine on `about:blank` and then calling `load()`. The real document was dropped. The first document is now the initial URL.
4. That load from the application thread, without the window lock. The debugger stopped on `Looper must be locked`. `_Start` locks the window before it reads the size.
5. After 3 and 4, about 10:05–10:13 Europe/Amsterdam: the `data:` page finished and the GET was sent. The test had not waited for an address. The page was `client error (Connect)`, with no version line.
6. A boot that waits for a non-loopback address was refused while other sessions had their own QEMU. The guard matched every `qemu-system-x86_64`. It now refuses only a second guest named `r5-servo-proof`. Other guests were not stopped.
7. The 10:16 boot did wait. The address was already there (`10.0.2.15`). Name lookup still failed. An address is not a working resolver.

## Direction

Do not change Servo's HTTP stack for this. The next boot has to make `getaddrinfo("http2.pro")` succeed in the guest, then load the same URL again.

QEMU user net answers DNS at `10.0.2.3`. The next run should print the guest's `resolv.conf` (and the `ifconfig` that this boot only put on the serial log) before the test. If there is no nameserver, or the nameserver does not answer, point the guest at `10.0.2.3` and retry. Do not kill the other QEMU guests. Do not call `scripts/run-qemu-r5.sh`.

The evidence that counts is the `net::http_loader` line that logs the response version. That line is only written after a response. A 301 still counts as HTTP/2 when that line says so. The redirect body is not the proof.

`Protocol()` will stay `B_NOT_SUPPORTED` on that 301, because it only recognises a body that contains `"http2":1` or `HTTP/2`. Servo 0.8 does not give the negotiated version to the embedder except that log. Read the log first. Passing the version into `Protocol()` is a later change in `servo-net`, after a response has been seen.
