#!/bin/bash
# Rebuild libr5servo.so and r5-servo-test for x86_64 Haiku.
# The instruction for the next agent is distro/servo/BUILD.md.
# Pins, patches, paths and the measured times are there.
# Job cap is 4. A running jam is left alone; this script does not write
# the shared Haiku tree.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SERVO_SRC="${SERVO_SRC:-/home/nickr/r5os/servo-build}"
HAIKU="${HAIKU:-/home/nickr/r5os/haiku}"
ENGINE="$ROOT/distro/servo/engine"
CXX="$HAIKU/generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-g++"
GCC="$HAIKU/generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-gcc"
AR="$HAIKU/generated.x86_64/cross-tools-x86_64/bin/x86_64-unknown-haiku-ar"
OUT="$SERVO_SRC/r5servo-out"
LOG="$SERVO_SRC/r5servo-build.log"
PATCHES="$ROOT/distro/servo/patches"

# Recreate a patched tree from the pinned crates.io tarball plus one
# patch. An existing tree is left in place so SpiderMonkey is not built
# again. R5_SERVO_REFRESH_PATCHES=1 replaces it.
fetch_crate() {
	dir="$1"
	crate="$2"
	version="$3"
	patch="$4"
	dest="$SERVO_SRC/$dir"
	if [ -f "$dest/Cargo.toml" ] && [ "${R5_SERVO_REFRESH_PATCHES:-0}" != 1 ]; then
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

for need in \
	"$CXX" \
	"$SERVO_SRC/rust/cargo/bin/cargo"
do
	if [ ! -e "$need" ]; then
		echo "missing $need" >&2
		echo "Rust 1.99.1 and the Haiku cross compiler are not in git. See distro/servo/BUILD.md." >&2
		exit 1
	fi
done

exec > >(tee -a "$LOG") 2>&1
echo START $(date -Is)

export RUSTUP_HOME="$SERVO_SRC/rust/rustup"
export CARGO_HOME="$SERVO_SRC/rust/cargo"
export PATH="$SERVO_SRC/host-tools/bin:$SERVO_SRC/llvm/bin:$HAIKU/generated.x86_64/cross-tools-x86_64/bin:/usr/bin:/bin:${CARGO_HOME}/bin"
export LIBCLANG_PATH="$SERVO_SRC/llvm/lib"
export CARGO_TARGET_DIR="$SERVO_SRC/target"
export CARGO_BUILD_JOBS=4
export RUSTC_BOOTSTRAP=1
export MOZJS_FROM_SOURCE=1
export PKG_CONFIG_ALLOW_CROSS=1
export HOST_CC=/usr/bin/gcc
export HOST_CXX=/usr/bin/g++
export ZSTD_SYS_NO_HIDE_SYMBOLS=1
export R5_BINDGEN_CLANG_ARGS_FILE="$SERVO_SRC/bindgen-clang-args.txt"
export R5_SERVO_CXX="$CXX"
export R5_SERVO_VIEW="$ROOT/distro/servo"
ZLIB="$CARGO_HOME/registry/src/index.crates.io-1949cf8c6b5b557f/libz-sys-1.1.33/src/zlib"
if [ -d "$ZLIB" ]; then
	# SpiderMonkey's system wrapper does include_next <zlib.h>. The Haiku
	# sysroot has libz.so and no header. This is the vendored header.
	export CPPFLAGS="-I$ZLIB"
	export CFLAGS_x86_64_unknown_haiku="-I$ZLIB"
fi

rustc +1.99.1 --version
# Build from a copy whose patch paths are the same absolute trees servo-min
# used, so cargo can reuse the SpiderMonkey it already compiled.
STAGE="$SERVO_SRC/r5servo-engine"
rm -rf "$STAGE"
mkdir -p "$STAGE/src" "$STAGE/.cargo" "$OUT"
cp "$ENGINE/Cargo.toml" "$ENGINE/build.rs" "$STAGE/"
cp "$ENGINE/src/lib.rs" "$STAGE/src/"
# The C++ view is included by path from the repository, not the stage.
python3 - "$STAGE/Cargo.toml" "$SERVO_SRC" <<'PY'
import pathlib, sys
path, servo = sys.argv[1:]
text = pathlib.Path(path).read_text()
repls = {
    ".vendor/mozjs-sys": f"{servo}/mozjs-sys",
    ".vendor/surfman": f"{servo}/surfman",
    ".vendor/ipc-channel": f"{servo}/ipc-channel",
    ".vendor/servo-fonts": f"{servo}/servo-fonts",
    ".vendor/servo-fonts-traits": f"{servo}/servo-fonts-traits",
    ".vendor/gaol": f"{servo}/gaol",
    ".vendor/servo-script": f"{servo}/servo-script",
}
for old, new in repls.items():
    text = text.replace(old, new)
pathlib.Path(path).write_text(text)
PY
cat > "$STAGE/cc.sh" <<EOF
#!/bin/sh
exec "$GCC" "\$@"
EOF
chmod +x "$STAGE/cc.sh"
cat > "$STAGE/.cargo/config.toml" <<EOF
[build]
target = "x86_64-unknown-haiku"

[target.x86_64-unknown-haiku]
linker = "$STAGE/cc.sh"

[env]
CC_x86_64_unknown_haiku = "$GCC"
CXX_x86_64_unknown_haiku = "$CXX"
AR_x86_64_unknown_haiku = "$AR"
CARGO_BUILD_JOBS = "4"
EOF
ENGINE="$STAGE"

step() {
	name="$1"
	shift
	echo "STEP $name $(date -Is)"
	start=$(date +%s)
	set +e
	/usr/bin/time -v "$@"
	status=$?
	set -e
	end=$(date +%s)
	echo "STEP_DONE $name status=$status seconds=$((end - start))"
	du -sh "$CARGO_TARGET_DIR" "$OUT" 2>/dev/null || true
	if [ "$status" -ne 0 ]; then
		echo BUILD_FAIL "$name" "$status"
		exit "$status"
	fi
}

cd "$ENGINE"
step cargo cargo +1.99.1 build -Zbuild-std=std,panic_unwind,panic_abort --release --target x86_64-unknown-haiku -j 4
LIBA="$CARGO_TARGET_DIR/x86_64-unknown-haiku/release/libr5servo.a"
# Same NEEDED set as the proof binary servo-min. Mesa's libGL is loaded
# by the engine at runtime, the way that binary does it.
step shared "$CXX" -shared -fPIC -O2 -o "$OUT/libr5servo.so" \
	"$ROOT/distro/servo/BServoView.cpp" -I"$ROOT/distro/servo" \
	-Wl,-soname,libr5servo.so \
	-Wl,--allow-multiple-definition \
	-Wl,--whole-archive "$LIBA" -Wl,--no-whole-archive \
	-lbe -lstdc++ -lz -lbsd -lnetwork -lroot -lgcc_s
step test "$CXX" -O2 -o "$OUT/r5-servo-test" "$ROOT/distro/servo/r5-servo-test.cpp" \
	-I"$ROOT/distro/servo" \
	-L"$OUT" \
	"$OUT/libr5servo.so" -lbe -lstdc++
echo BUILD_OK $(date -Is)
ls -lh "$OUT/libr5servo.so" "$OUT/r5-servo-test"
file "$OUT/libr5servo.so" "$OUT/r5-servo-test"
