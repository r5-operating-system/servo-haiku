#!/bin/bash
# SPDX-FileCopyrightText: 2026 R5OS Contributors
# SPDX-License-Identifier: MIT
#
# Build libr5servo.so (Servo 0.8.0 with BServoView) and r5-servo-test for
# x86_64 Haiku, on a Linux build host with a Haiku cross toolchain.
# docs/BUILDING.md has the requirements and every variable.
#
# Required:
#   HAIKU_CROSS_TOOLS  a Haiku x86_64 cross toolchain: the directory that has
#                      bin/x86_64-unknown-haiku-g++ and sysroot/. A Haiku build
#                      tree has it at $HAIKU/generated.x86_64/cross-tools-x86_64,
#                      so setting HAIKU instead is enough.
#   LIBCLANG_PATH      a directory with libclang.so for bindgen (LLVM 21.1.8 is
#                      the tested one). Default: $WORK/llvm/lib if it exists.
# Optional:
#   WORK               build directory (default: build/ in this repository):
#                      patched crates, cargo's target (~16G) and r5servo-out/.
#   RUST_TOOLCHAIN     rustup toolchain with rust-src (default 1.99.1).
#   RUSTUP_HOME, CARGO_HOME  default: $WORK/rust/rustup and $WORK/rust/cargo
#                      when those exist, else ~/.rustup and ~/.cargo.
#   JOBS               parallel jobs (default 4).
#   EXTRA_PATH         put in front of PATH (for example a pkg-config, or
#                      rustup's cargo and rustc when CARGO_HOME has no bin/).
#   SERVO_REFRESH_PATCHES=1  unpack and patch the crates again (this rebuilds
#                      SpiderMonkey).
set -eu

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${WORK:-$REPO/build}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.99.1}"
JOBS="${JOBS:-4}"
TARGET=x86_64-unknown-haiku
PATCHES="$REPO/patches"

if [ -z "${HAIKU_CROSS_TOOLS:-}" ] && [ -n "${HAIKU:-}" ]; then
	HAIKU_CROSS_TOOLS="$HAIKU/generated.x86_64/cross-tools-x86_64"
fi
if [ -z "${HAIKU_CROSS_TOOLS:-}" ]; then
	echo "set HAIKU_CROSS_TOOLS (or HAIKU, a Haiku build tree); see docs/BUILDING.md" >&2
	exit 1
fi
CROSS="$HAIKU_CROSS_TOOLS"
CXX="$CROSS/bin/$TARGET-g++"
GCC="$CROSS/bin/$TARGET-gcc"
AR="$CROSS/bin/$TARGET-ar"
SYSROOT="$CROSS/sysroot"

mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd)"
if [ -z "${LIBCLANG_PATH:-}" ] && [ -d "$WORK/llvm/lib" ]; then
	LIBCLANG_PATH="$WORK/llvm/lib"
fi
if [ -z "${RUSTUP_HOME:-}" ] && [ -d "$WORK/rust/rustup" ]; then
	RUSTUP_HOME="$WORK/rust/rustup"
fi
if [ -z "${CARGO_HOME:-}" ] && [ -d "$WORK/rust/cargo" ]; then
	CARGO_HOME="$WORK/rust/cargo"
fi
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"

missing=0
for need in "$CXX" "$GCC" "$AR" "$SYSROOT/boot/system/develop/headers/os" \
	"${LIBCLANG_PATH:-LIBCLANG_PATH-is-not-set}"
do
	if [ ! -e "$need" ]; then
		echo "missing $need" >&2
		missing=1
	fi
done
for tool in curl patch python3 gcc g++ pkg-config cargo rustc; do
	if ! PATH="${EXTRA_PATH:+$EXTRA_PATH:}$PATH:$CARGO_HOME/bin" command -v "$tool" >/dev/null; then
		echo "missing host tool $tool" >&2
		missing=1
	fi
done
if [ "$missing" != 0 ]; then
	echo "see docs/BUILDING.md" >&2
	exit 1
fi

OUT="$WORK/r5servo-out"
LOG="$WORK/r5servo-build.log"
mkdir -p "$OUT"
exec > >(tee -a "$LOG") 2>&1
echo START $(date -Is)

export RUSTUP_HOME CARGO_HOME LIBCLANG_PATH
LLVM_BIN="$(dirname "$LIBCLANG_PATH")/bin"
export PATH="${EXTRA_PATH:+$EXTRA_PATH:}$LLVM_BIN:$CROSS/bin:/usr/bin:/bin:$CARGO_HOME/bin"
export CARGO_TARGET_DIR="$WORK/target"
export CARGO_BUILD_JOBS="$JOBS"
export RUSTC_BOOTSTRAP=1
export MOZJS_FROM_SOURCE=1
export PKG_CONFIG_ALLOW_CROSS=1
# The host compiler builds SpiderMonkey's host tools; the cross compiler is
# not the host compiler.
export HOST_CC=/usr/bin/gcc
export HOST_CXX=/usr/bin/g++
# libbe.so needs the public zstd symbols; the bundled copy would hide them.
export ZSTD_SYS_NO_HIDE_SYMBOLS=1
export R5_BINDGEN_CLANG_ARGS_FILE="$WORK/bindgen-clang-args.txt"
export R5_SERVO_CXX="$CXX"
export R5_SERVO_VIEW="$REPO"

# bindgen runs the host libclang. These are the Haiku C++ and system headers
# it must see (patches/mozjs_sys-153.6.0-1-haiku.patch reads the file). The
# file is rewritten only when its content changes.
gccver="$("$CXX" -dumpversion)"
headers="$SYSROOT/boot/system/develop/headers"
args="$WORK/bindgen-clang-args.txt.new"
{
	echo -nostdinc++
	echo --target=$TARGET
	for dir in \
		"$CROSS/$TARGET/include/c++/$gccver" \
		"$CROSS/$TARGET/include/c++/$gccver/$TARGET" \
		"$CROSS/$TARGET/include/c++/$gccver/backward" \
		"$CROSS/$TARGET/include" \
		"$headers/os" \
		"$headers/os/app" "$headers/os/device" "$headers/os/drivers" \
		"$headers/os/game" "$headers/os/interface" "$headers/os/kernel" \
		"$headers/os/locale" "$headers/os/mail" "$headers/os/media" \
		"$headers/os/midi" "$headers/os/midi2" "$headers/os/net" \
		"$headers/os/storage" "$headers/os/support" "$headers/os/translation" \
		"$headers/os/add-ons/graphics" "$headers/os/add-ons/input_server" \
		"$headers/os/add-ons/mail_daemon" "$headers/os/add-ons/registrar" \
		"$headers/os/add-ons/screen_saver" "$headers/os/add-ons/tracker" \
		"$headers/os/be_apps/NetPositive" "$headers/os/be_apps/Tracker" \
		"$headers/bsd" "$headers/glibc" "$headers/gnu" "$headers/posix" \
		"$headers"
	do
		echo -isystem
		echo "$dir"
	done
} > "$args"
if cmp -s "$args" "$R5_BINDGEN_CLANG_ARGS_FILE"; then
	rm "$args"
else
	mv "$args" "$R5_BINDGEN_CLANG_ARGS_FILE"
fi

# A patched tree: the pinned crates.io tarball plus one file from patches/.
# An existing tree is left in place, so SpiderMonkey is not built again.
fetch_crate() {
	dir="$1"
	crate="$2"
	version="$3"
	patch="$4"
	dest="$WORK/$dir"
	if [ -f "$dest/Cargo.toml" ] \
		&& [ "${SERVO_REFRESH_PATCHES:-${R5_SERVO_REFRESH_PATCHES:-0}}" != 1 ]; then
		return 0
	fi
	if [ ! -f "$PATCHES/$patch" ]; then
		echo "missing $PATCHES/$patch" >&2
		exit 1
	fi
	tmp=$(mktemp -d)
	echo "fetch $crate $version"
	curl -fsSL -o "$tmp/crate.tar.gz" \
		"https://static.crates.io/crates/${crate}/${crate}-${version}.crate"
	tar -xzf "$tmp/crate.tar.gz" -C "$tmp"
	unpacked="$tmp/${crate}-${version}"
	patch -p1 --forward --batch -d "$unpacked" -i "$PATCHES/$patch"
	rm -rf "$dest"
	mv "$unpacked" "$dest"
	rm -rf "$tmp"
}

fetch_crate mozjs-sys mozjs_sys 153.6.0-1 mozjs_sys-153.6.0-1-haiku.patch
fetch_crate surfman surfman 0.14.0 surfman-0.14.0-haiku-egl.patch
fetch_crate ipc-channel ipc-channel 0.23.1 ipc-channel-0.23.1-haiku-inprocess.patch
fetch_crate gaol gaol 0.2.1 gaol-0.2.1-haiku-no-sandbox.patch
fetch_crate servo-fonts servo-fonts 0.8.0 servo-fonts-0.8.0-haiku-freetype.patch
fetch_crate servo-fonts-traits servo-fonts-traits 0.8.0 servo-fonts-traits-0.8.0-haiku-font-id.patch
fetch_crate servo-script servo-script 0.8.0 servo-script-0.8.0-haiku-platform.patch
fetch_crate servo-net servo-net 0.8.0 servo-net-0.8.0-no-credentials-in-uri.patch

# The engine is built from a copy whose [patch] paths point at the trees
# above (absolute paths, so cargo keeps reusing the SpiderMonkey it built).
STAGE="$WORK/r5servo-engine"
rm -rf "$STAGE"
mkdir -p "$STAGE/src" "$STAGE/.cargo"
# Cargo.lock too: the build uses the versions recorded in git (source offer).
cp "$REPO/engine/Cargo.toml" "$REPO/engine/Cargo.lock" "$REPO/engine/build.rs" "$STAGE/"
cp "$REPO/engine/src/lib.rs" "$STAGE/src/"
python3 - "$STAGE/Cargo.toml" "$WORK" <<'PY'
import pathlib, sys
path, work = sys.argv[1:]
text = pathlib.Path(path).read_text()
for name in ("mozjs-sys", "surfman", "ipc-channel", "servo-fonts", "servo-fonts-traits",
             "gaol", "servo-script", "servo-net"):
    text = text.replace(f'".vendor/{name}"', f'"{work}/{name}"')
if ".vendor/" in text:
    raise SystemExit("Cargo.toml has a .vendor path this script does not know")
pathlib.Path(path).write_text(text)
PY
cat > "$STAGE/cc.sh" <<EOF
#!/bin/sh
exec "$GCC" "\$@"
EOF
chmod +x "$STAGE/cc.sh"
cat > "$STAGE/.cargo/config.toml" <<EOF
[build]
target = "$TARGET"

[target.$TARGET]
linker = "$STAGE/cc.sh"

[env]
CC_x86_64_unknown_haiku = "$GCC"
CXX_x86_64_unknown_haiku = "$CXX"
AR_x86_64_unknown_haiku = "$AR"
CARGO_BUILD_JOBS = "$JOBS"
EOF

step() {
	name="$1"
	shift
	echo "STEP $name $(date -Is)"
	start=$(date +%s)
	set +e
	if [ -x /usr/bin/time ]; then
		/usr/bin/time -v "$@"
	else
		"$@"
	fi
	status=$?
	set -e
	end=$(date +%s)
	echo "STEP_DONE $name status=$status seconds=$((end - start))"
	if [ "$status" -ne 0 ]; then
		echo BUILD_FAIL "$name" "$status"
		exit "$status"
	fi
}

cd "$STAGE"
rustc +"$RUST_TOOLCHAIN" --version
# --locked: exactly the Cargo.lock in git; a Cargo.toml change without its
# lock stops here.
step fetch cargo +"$RUST_TOOLCHAIN" fetch --locked --target "$TARGET"

# SpiderMonkey's system wrapper does include_next <zlib.h>, and the Haiku
# sysroot has libz.so without the header. The header is the one vendored in
# libz-sys, which cargo fetch has unpacked.
ZLIB="$(ls -d "$CARGO_HOME"/registry/src/*/libz-sys-1.1.33/src/zlib 2>/dev/null | head -n 1)"
if [ -z "$ZLIB" ]; then
	echo "libz-sys 1.1.33 not found under $CARGO_HOME/registry/src" >&2
	exit 1
fi
export CPPFLAGS="-I$ZLIB"
export CFLAGS_x86_64_unknown_haiku="-I$ZLIB"

step cargo cargo +"$RUST_TOOLCHAIN" build --locked -Zbuild-std=std,panic_unwind,panic_abort \
	--release --target "$TARGET" -j "$JOBS"
LIBA="$CARGO_TARGET_DIR/$TARGET/release/libr5servo.a"
# The shared library is Haiku g++ around the Rust staticlib (a Rust cdylib
# hides the C++ symbols). Two copies of lz4 need --allow-multiple-definition.
# Mesa's libGL is opened at run time.
step shared "$CXX" -shared -fPIC -O2 -o "$OUT/libr5servo.so" \
	"$REPO/BServoView.cpp" -I"$REPO" \
	-Wl,-soname,libr5servo.so \
	-Wl,--allow-multiple-definition \
	-Wl,--whole-archive "$LIBA" -Wl,--no-whole-archive \
	-lbe -ltracker -lstdc++ -lz -lbsd -lnetwork -lroot -lgcc_s
step test "$CXX" -O2 -o "$OUT/r5-servo-test" "$REPO/r5-servo-test.cpp" \
	-I"$REPO" -L"$OUT" "$OUT/libr5servo.so" -lbe -lstdc++
echo BUILD_OK $(date -Is)
ls -lh "$OUT/libr5servo.so" "$OUT/r5-servo-test"
