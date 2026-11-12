#!/usr/bin/env bash
# Runs the AppImage rig in a distribution's container with Docker: installs the runtime
# packages (packages.sh runtime), checks the AppImage finds its libraries with only those
# (libs.sh), adds the rig's own and a sound server (packages.sh rig, sound), takes the
# container off the network, and
# runs the tests (run.sh) as an unprivileged user. Everything run.sh writes ends up in
# <rig>/out. The container is removed afterwards.
#
# Usage: container.sh <image> <rig dir>
#   <image>    ubuntu:22.04, ubuntu:24.04, debian:stable, fedora:latest, archlinux:latest
#   <rig dir>  the layout run.sh describes, mounted at /rig
set -uo pipefail

image=${1:?usage: container.sh <image> <rig dir>}
rig=$(cd "${2:?rig dir}" && pwd)
name=fp-rig-appimage-$$
arch=$(uname -m)
mkdir -p "$rig/out"
# The test user writes here; its uid inside isn't ours outside.
chmod 777 "$rig/out"

trap 'docker rm -f "$name" >/dev/null 2>&1' EXIT
docker pull -q "$image" >/dev/null
docker run -d --name "$name" -v "$rig:/rig" -e OUT=/rig/out "$image" sleep infinity >/dev/null
in_box() { docker exec "$name" "$@"; }

status=0
if ! in_box bash /rig/scripts/packages.sh runtime; then
    echo "::error::$image: the runtime packages didn't install"
    exit 1
fi
if ! in_box bash /rig/scripts/libs.sh "/rig/release/featherplay-$arch.AppImage"; then
    echo "::error::$image: the AppImage misses libraries with the runtime packages alone (libs.txt)"
    status=1
fi
if ! in_box bash /rig/scripts/packages.sh rig; then
    echo "::error::$image: the rig's packages didn't install"
    exit 1
fi
# Without a sound server run.sh skips the playback test, and says so.
if ! in_box bash /rig/scripts/packages.sh sound; then
    echo "::warning::$image: no sound server installed; the playback test is skipped"
fi
in_box sh -c 'mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix'
in_box useradd -m tester
# Offline from here: the app starts signed out with no network, and the update server is
# on 127.0.0.1 inside the container.
docker network disconnect bridge "$name"
if ! docker exec -u tester -w /home/tester -e OUT=/rig/out -e RIG_OFFLINE=1 "$name" \
    bash /rig/scripts/run.sh; then
    status=1
fi
exit $status
