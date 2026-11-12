#!/usr/bin/env bash
# Checks, after `packages.sh runtime` and before anything else is installed, that the
# AppImage's program finds every library it needs: those it links (the loader's list, none
# "not found") and those it loads at run time for its window (each soname in the loader's
# cache). The run-time list is what the release AppImage loaded under X11 and Wayland
# (LD_DEBUG=files, 2026-11-12); the sound server's libraries come from ALSA's plugin, not
# from featherplay.
#
# Usage: libs.sh <AppImage>
set -uo pipefail

image=${1:?usage: libs.sh <AppImage>}
out=${OUT:-/rig/out}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$image" "$work/app.AppImage"
chmod +x "$work/app.AppImage"
(cd "$work" && ./app.AppImage --appimage-extract >/dev/null) || {
    echo "libs.sh: the AppImage didn't extract" >&2
    exit 1
}
bin=$work/squashfs-root/usr/bin/featherplay

dlopened="libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXi.so.6 libxcb.so.1 libxkbcommon.so.0
          libxkbcommon-x11.so.0 libwayland-client.so.0"

{
    echo "# linked (the loader's list)"
    if command -v ldd >/dev/null; then
        list=$(ldd "$bin" 2>&1)
    else
        loader=$(ls /lib64/ld-linux-x86-64.so.2 /lib/ld-linux-aarch64.so.1 2>/dev/null | head -1)
        list=$("$loader" --list "$bin" 2>&1)
    fi
    echo "$list"
    echo "# loaded at run time"
    cache=$(ldconfig -p 2>/dev/null)
    for so in $dlopened; do
        if grep -q "^[[:space:]]*$so " <<<"$cache"; then
            echo "$so: found"
        else
            echo "$so: MISSING"
        fi
    done
} | tee "$out/libs.txt"
! grep -q 'not found\|MISSING' "$out/libs.txt"
