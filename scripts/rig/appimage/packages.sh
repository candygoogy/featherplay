#!/usr/bin/env bash
# Installs, as root inside a distribution's container, one of three package sets, and lists
# what it installed in $OUT/packages-<set>.txt:
#   runtime  what featherplay's AppImage needs and a desktop install already has: the
#            libraries the binary links (fontconfig, FreeType, ALSA, OpenSSL 3) and those it
#            loads at run time for an X11 or Wayland window (libX11, libX11-xcb, libXcursor,
#            libXi, libxcb, libxkbcommon, libxkbcommon-x11, libwayland-client), and one font.
#            libs.sh then checks that these are enough, before anything else is installed.
#   rig      what the rig itself needs: an X server (Xvfb), and for the tests python3 (the
#            update server), procps (pgrep), findutils and useradd.
#   sound    a sound server for the null sink, as a desktop has one: PulseAudio, pactl, and
#            ALSA's pulse plugin (the route ALSA programs such as featherplay take to it).
# The distribution is /etc/os-release's ID: ubuntu, debian, fedora or arch.
#
# Usage: packages.sh runtime|rig|sound
set -euo pipefail

set_name=${1:?usage: packages.sh runtime|rig|sound}
out=${OUT:-/rig/out}
mkdir -p "$out"
# shellcheck disable=SC1091
. /etc/os-release

case "$ID:$set_name" in
ubuntu:runtime | debian:runtime)
    # 24.04 and Debian 13 renamed ALSA's and OpenSSL's libraries for the 64-bit time_t move.
    apt-get update -qq
    t64=
    if apt-cache show libasound2t64 >/dev/null 2>&1; then t64=t64; fi
    pkgs="libfontconfig1 libfreetype6 libasound2$t64 libssl3$t64 libx11-6 libx11-xcb1
          libxcursor1 libxi6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0
          fonts-dejavu-core"
    ;;
ubuntu:rig | debian:rig)
    pkgs="xvfb python3 procps findutils passwd"
    ;;
ubuntu:sound | debian:sound)
    pkgs="pulseaudio pulseaudio-utils libasound2-plugins"
    ;;
fedora:runtime)
    pkgs="fontconfig freetype alsa-lib openssl-libs libX11 libX11-xcb libXcursor libXi libxcb
          libxkbcommon libxkbcommon-x11 libwayland-client dejavu-sans-fonts"
    ;;
fedora:rig)
    pkgs="xorg-x11-server-Xvfb python3 procps-ng findutils shadow-utils"
    ;;
fedora:sound)
    pkgs="pulseaudio pulseaudio-utils alsa-plugins-pulseaudio"
    ;;
arch:runtime)
    # libX11-xcb is part of libx11 here.
    pkgs="fontconfig freetype2 alsa-lib openssl libx11 libxcursor libxi libxcb libxkbcommon
          libxkbcommon-x11 wayland ttf-dejavu"
    ;;
arch:rig)
    pkgs="xorg-server-xvfb python procps-ng findutils shadow"
    ;;
arch:sound)
    pkgs="pulseaudio pulseaudio-alsa alsa-plugins"
    ;;
*)
    echo "packages.sh: no '$set_name' packages for $ID" >&2
    exit 1
    ;;
esac
# shellcheck disable=SC2086
set -- $pkgs

echo "packages.sh: $PRETTY_NAME, $set_name: $*"
case $ID in
ubuntu | debian)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    before=$(dpkg-query -W -f '${Package}\n' | sort)
    apt-get install -y -qq --no-install-recommends "$@" >/dev/null
    after=$(dpkg-query -W -f '${Package}\n' | sort)
    ;;
fedora)
    before=$(rpm -qa --qf '%{NAME}\n' | sort)
    dnf -y -q install --setopt=install_weak_deps=False "$@"
    after=$(rpm -qa --qf '%{NAME}\n' | sort)
    ;;
arch)
    before=$(pacman -Qq | sort)
    # A full upgrade with the install: Arch doesn't support partial upgrades.
    pacman -Syu --noconfirm --needed --noprogressbar "$@" >/dev/null
    after=$(pacman -Qq | sort)
    ;;
esac

# What was asked for, then everything that came with it.
{
    echo "# $PRETTY_NAME ($(uname -m)), $set_name set"
    echo "asked: $*"
    echo "installed with them: $(comm -13 <(echo "$before") <(echo "$after") | tr '\n' ' ')"
} | tee "$out/packages-$set_name.txt"
