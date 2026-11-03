#!/usr/bin/env bash
# Builds featherplay's per-user installer: the release exe, then
# target/installer/featherplay-setup-<version>.exe (packaging/windows/featherplay.iss).
#
# Usage: packaging/windows/build-installer.sh [--skip-build] [extra ISCC arguments...]
#   --skip-build  package the exe already in target/release instead of building it
#   extra arguments go to ISCC as they are (no MSYS path conversion), e.g.
#   /DCredService=<name> for the installer test, or /O<dir> for another output folder
#
# Needs:
# - Inno Setup 6.7 or later: ISCC=<path to ISCC.exe>, or ISCC on PATH, or a standard install.
# - cargo. CARGO_CMD=<command> runs the build through something else, such as a wrapper
#   that waits for a build lock (an exported `cargo` shell function is used as well).
# - The build keys LASTFM_API_KEY, LASTFM_API_SECRET and DISCORD_CLIENT_ID, compiled in with
#   option_env!: in the environment, or in a KEY=value file named by FEATHERPLAY_ENV. The
#   script refuses to build without them (ALLOW_NO_KEYS=1 builds a keyless exe anyway). It
#   never prints them.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

skip_build=
if [ "${1:-}" = "--skip-build" ]; then
    skip_build=1
    shift
fi

die() {
    echo "build-installer: $*" >&2
    exit 1
}

iscc=${ISCC:-}
if [ -z "$iscc" ]; then
    for c in "$(command -v iscc 2>/dev/null || true)" \
        "${LOCALAPPDATA:-}/Programs/Inno Setup 6/ISCC.exe" \
        "${PROGRAMFILES:-C:/Program Files}/Inno Setup 6/ISCC.exe" \
        "C:/Program Files (x86)/Inno Setup 6/ISCC.exe"; do
        if [ -n "$c" ] && [ -f "$c" ]; then
            iscc=$c
            break
        fi
    done
fi
[ -n "$iscc" ] && [ -f "$iscc" ] || die "Inno Setup's ISCC.exe not found; set ISCC=<path>"

# The version the exe reports (Cargo's workspace version, inherited by apps/featherplay).
pkgid=$(cargo pkgid -p featherplay)
version=${pkgid##*[@#]}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
    die "version '$version' isn't x.y.z, which Windows' file version needs"

if [ -z "$skip_build" ]; then
    if [ -n "${FEATHERPLAY_ENV:-}" ]; then
        [ -f "$FEATHERPLAY_ENV" ] || die "FEATHERPLAY_ENV: no file $FEATHERPLAY_ENV"
        set -a
        # shellcheck disable=SC1090
        source "$FEATHERPLAY_ENV"
        set +a
    fi
    for name in LASTFM_API_KEY LASTFM_API_SECRET DISCORD_CLIENT_ID; do
        if [ -z "${!name:-}" ] && [ -z "${ALLOW_NO_KEYS:-}" ]; then
            die "$name is not set (see the header of this script)"
        fi
    done
    ${CARGO_CMD:-cargo} build --release --locked -p featherplay
fi

exe=target/release/featherplay.exe
[ -f "$exe" ] || die "no $exe; build it first, or drop --skip-build"

# build.rs renders the icon and the wizard images into its OUT_DIR. Old build dirs can
# linger, so take the newest.
icon=$(ls -t target/release/build/featherplay-*/out/featherplay.ico 2>/dev/null | head -1)
[ -n "$icon" ] || die "no featherplay.ico under target/release/build; build the release exe"
art=$(dirname "$icon")
for f in installer-small installer-panel installer-small-light installer-panel-light; do
    [ -f "$art/$f.png" ] || die "no $art/$f.png; rebuild the release exe"
done

mkdir -p target/installer
win() { cygpath -w "$1"; }
# ISCC's switches start with "/", which MSYS would otherwise rewrite as paths.
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$iscc" /Qp \
    "/DAppVersion=$version" \
    "/DExeFile=$(win "$root/$exe")" \
    "/DArtDir=$(win "$root/$art")" \
    "/O$(win "$root/target/installer")" \
    "$@" \
    "$(win "$root/packaging/windows/featherplay.iss")"

echo "featherplay.exe: $(stat -c %s "$exe") bytes"
ls -l target/installer/featherplay-setup-"$version".exe 2>/dev/null || true
