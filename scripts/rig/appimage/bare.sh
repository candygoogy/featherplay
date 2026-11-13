#!/usr/bin/env bash
# The release AppImage's first start on a bare system, as the AppImage catalog
# (appimage.github.io) tests an AppImage: in a container of <image> with only the libraries
# the AppImage excludelist expects every system to have, and the keyboard data (packages.sh
# bare), no network, no D-Bus and no keyring, as a plain user with a fresh home. It passes
# when a window shows on this machine's X server (Xvfb, started here unless DISPLAY is set)
# within 30 s, as `xdotool search --onlyvisible --name .` finds it, and the program is still
# running. The container shares only the X socket with this machine.
#
# In <out>: results.txt (one PASS or FAIL line), bare.stdout and bare.stderr, bare.log (the
# app's log), bare-libs.txt (the libraries the running program has mapped, and from where),
# bare-windows.txt, packages-bare.txt.
#
# Usage: bare.sh <image> <AppImage> <out dir>
#   Needs docker, Xvfb, xdotool, pgrep, and sudo (to read the program's maps).
set -uo pipefail

image=${1:?usage: bare.sh <image> <AppImage> <out dir>}
appimage=${2:?AppImage}
mkdir -p "${3:?out dir}"
out=$(cd "$3" && pwd)
scripts=$(cd "$(dirname "$0")" && pwd)
name=fp-rig-bare-$$
results=$out/results.txt
: >"$results"
chmod 777 "$out"

result() { echo "$1 bare: $2" | tee -a "$results"; }

xvfb=
if [ -z "${DISPLAY:-}" ]; then
    Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >"$out/xvfb.log" 2>&1 &
    xvfb=$!
    export DISPLAY=:99
    for _ in $(seq 100); do [ -S /tmp/.X11-unix/X99 ] && break; sleep 0.1; done
fi
trap 'docker rm -f "$name" >/dev/null 2>&1; [ -n "$xvfb" ] && kill "$xvfb"' EXIT

docker pull -q "$image" >/dev/null
docker run -d --name "$name" -v "$out:/out" -v "$scripts:/scripts:ro" \
    -v /tmp/.X11-unix:/tmp/.X11-unix -e OUT=/out "$image" sleep infinity >/dev/null
docker cp "$appimage" "$name:/opt/app.AppImage" >/dev/null
in_box() { docker exec "$name" "$@"; }
in_box chmod 755 /opt/app.AppImage
if ! in_box bash /scripts/packages.sh bare; then
    result FAIL "the bare packages didn't install"
    exit 1
fi
in_box useradd -m tester
docker network disconnect bridge "$name"

# The first start. FUSE isn't in the container: the runtime extracts the AppImage instead.
docker exec -d -u tester -w /home/tester -e HOME=/home/tester -e DISPLAY="$DISPLAY" "$name" \
    sh -c '/opt/app.AppImage --appimage-extract-and-run >/out/bare.stdout 2>/out/bare.stderr
           echo $? >/out/bare.exit'
window=
for s in $(seq 30); do
    sleep 1
    [ -f "$out/bare.exit" ] && break
    if timeout 5 xdotool search --onlyvisible --name . >/dev/null 2>&1; then
        window=$s
        break
    fi
done
timeout 5 xdotool search --onlyvisible --name . getwindowname %@ >"$out/bare-windows.txt" 2>&1
pid=$(pgrep -f '/appimage_extracted_[0-9a-f]*/AppRun' | head -1)
if [ -n "$pid" ]; then
    sudo cat "/proc/$pid/maps" | awk '$6 ~ /\.so/ { print $6 }' | sort -u >"$out/bare-libs.txt"
    sudo kill -TERM "$pid"
    for _ in $(seq 100); do [ -f "$out/bare.exit" ] && break; sleep 0.1; done
fi
log=$(in_box find /home/tester -name featherplay.log -print -quit 2>/dev/null)
[ -n "$log" ] && docker cp "$name:$log" "$out/bare.log" >/dev/null

code=$(cat "$out/bare.exit" 2>/dev/null || echo "none within 10 s")
if [ -n "$window" ] && [ -n "$pid" ]; then
    result PASS "a window within $window s ($(head -1 "$out/bare-windows.txt")), offline, no D-Bus; SIGTERM: exit $code"
elif [ -n "$window" ]; then
    result FAIL "a window, but the program exited right after (exit $code; bare.stderr)"
elif [ -z "$pid" ]; then
    result FAIL "exited with $code instead of showing a window (bare.stderr)"
else
    result FAIL "no window within 30 s (bare.stderr, bare.log)"
fi
grep -q '^PASS' "$results"
