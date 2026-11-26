#!/bin/bash
# Boot r5-servo-test on a private copy of the proof image.
# Does not stop another QEMU and does not call scripts/run-qemu-r5.sh.
set -eu
SERVO_SRC="${SERVO_SRC:-/home/nickr/r5os/servo-build}"
HAIKU="${HAIKU:-/home/nickr/r5os/haiku}"
BFS="$HAIKU/generated.x86_64/objects/linux/x86_64/release/tools/bfs_shell/bfs_shell"
SRC_IMG="${SRC_IMG:-$SERVO_SRC/servo.image}"
IMG="$SERVO_SRC/r5servo-proof.image"
OUT="$SERVO_SRC/r5servo-out"
LOG="$SERVO_SRC/r5servo-qemu.log"
SECONDS_UP="${SECONDS_UP:-300}"

exec > >(tee "$LOG") 2>&1
echo START $(date -Is)
test -x "$OUT/r5-servo-test"
test -f "$OUT/libr5servo.so"
test -f "$SRC_IMG"
# Other sessions may have their own QEMU. Refuse only a second copy of
# this proof. Do not stop those other guests.
if pgrep -f -- '-name r5-servo-proof' >/dev/null; then
	echo "r5-servo-proof is already running; not booting"
	exit 2
fi

rm -f "$IMG"
cp --sparse=always "$SRC_IMG" "$IMG"
cat > "$SERVO_SRC/r5-servo-test.job" <<'EOF'
job r5-servo-test {
	launch /bin/sh -c /boot/system/non-packaged/bin/r5-servo-run
	on initial_volumes_mounted
}
EOF
cat > "$SERVO_SRC/r5-servo-run" <<'EOF'
#!/bin/sh
echo r5-servo-run-start > /dev/dprintf
i=0
while [ "$i" -lt 90 ]; do
	if [ -e /boot/system/lib/libbe.so ]; then
		break
	fi
	i=$((i + 1))
	echo "r5-servo-wait $i" > /dev/dprintf
	sleep 2
done
sleep 15
echo "deskbar-wait $i" > /dev/dprintf
echo "deskbar-wait $i" >> /boot/home/r5-servo.log
n=0
while [ "$n" -lt 40 ]; do
	if ifconfig 2>/dev/null | grep 'inet ' | grep -v '127.0.0.1' >/dev/null; then
		break
	fi
	n=$((n + 1))
	sleep 2
done
echo "net-wait $n" > /dev/dprintf
echo "net-wait $n" >> /boot/home/r5-servo.log
ifconfig > /dev/dprintf
ifconfig >> /boot/home/r5-servo.log 2>&1
RESOLV=/boot/system/settings/network/resolv.conf
echo "--- resolv.conf before ---" >> /boot/home/r5-servo.log
cat "$RESOLV" >> /boot/home/r5-servo.log 2>&1 || echo "(missing)" >> /boot/home/r5-servo.log
if ! grep -q '10\.0\.2\.3' "$RESOLV" 2>/dev/null; then
	mkdir -p /boot/system/settings/network
	printf 'nameserver 10.0.2.3\n' > "$RESOLV"
	echo "wrote nameserver 10.0.2.3 to $RESOLV" >> /boot/home/r5-servo.log
	echo "wrote nameserver 10.0.2.3 to $RESOLV" > /dev/dprintf
fi
echo "--- resolv.conf after ---" >> /boot/home/r5-servo.log
cat "$RESOLV" >> /boot/home/r5-servo.log 2>&1 || true
echo "--- r5servo package files ---" >> /boot/home/r5-servo.log
ls -l /boot/system/lib/libr5servo.so /boot/system/develop/headers/r5/BServoView.h >> /boot/home/r5-servo.log 2>&1 || true
sync
export GALLIUM_DRIVER=zink
export LIBGL_ALWAYS_SOFTWARE=1
export RUST_LOG=net::http_loader=debug
export R5_SERVO_PROOF=1
# LIBRARY_PATH replaces the system search path on Haiku. Leave it unset
# so libbe.so is still found. non-packaged/lib is already on that path.
/boot/system/non-packaged/bin/r5-servo-test >> /boot/home/r5-servo.log 2>&1
echo "exit $?" >> /boot/home/r5-servo.log
sync
EOF
chmod 755 "$SERVO_SRC/r5-servo-run"

if [ "${R5_SERVO_FROM_PKG:-0}" = "1" ]; then
"$BFS" "$IMG" <<EOT
mkdir -p myfs/system/non-packaged/bin
mkdir -p myfs/system/settings/launch
cp :$OUT/r5-servo-test myfs/system/non-packaged/bin/r5-servo-test
cp :$SERVO_SRC/r5-servo-run myfs/system/non-packaged/bin/r5-servo-run
cp :$SERVO_SRC/r5-servo-test.job myfs/system/settings/launch/r5-servo-test
rm myfs/system/settings/launch/r5-servo-min
chmod 755 myfs/system/non-packaged/bin/r5-servo-test
chmod 755 myfs/system/non-packaged/bin/r5-servo-run
quit
EOT
else
"$BFS" "$IMG" <<EOT
mkdir -p myfs/system/non-packaged/bin
mkdir -p myfs/system/non-packaged/lib
mkdir -p myfs/system/settings/launch
cp :$OUT/r5-servo-test myfs/system/non-packaged/bin/r5-servo-test
cp :$OUT/libr5servo.so myfs/system/non-packaged/lib/libr5servo.so
cp :$OUT/libr5servo.so myfs/system/non-packaged/bin/libr5servo.so
cp :$SERVO_SRC/r5-servo-run myfs/system/non-packaged/bin/r5-servo-run
cp :$SERVO_SRC/r5-servo-test.job myfs/system/settings/launch/r5-servo-test
rm myfs/system/settings/launch/r5-servo-min
chmod 755 myfs/system/non-packaged/bin/r5-servo-test
chmod 755 myfs/system/non-packaged/bin/r5-servo-run
quit
EOT
fi

accel=tcg
cpu=Nehalem,+smap,+smep
if [ -r /dev/kvm ]; then
	accel=kvm
	cpu=host
fi
echo "booting $IMG ($accel) for $SECONDS_UP s"
timeout "$SECONDS_UP" qemu-system-x86_64 -machine pc,accel=$accel -cpu "$cpu" \
	-name r5-servo-proof -m 2048 -smp 2 -display none \
	-drive file="$IMG",format=raw,if=ide,index=0,media=disk \
	-netdev user,id=n0 -device e1000,netdev=n0 \
	-vga std -serial file:"$SERVO_SRC/r5servo-serial.log" </dev/null || true

echo "--- guest log ---"
"$BFS" "$IMG" <<'EOT'
cat myfs/home/r5-servo.log
quit
EOT
echo QEMU_DONE $(date -Is)
