#!/usr/bin/env bash
# Builds featherplay's per-user installer: the release exe, then
# target/installer/featherplay-setup-<version>.exe (packaging/windows/featherplay.iss), or
# featherplay-setup-<version>-arm64.exe with --arch arm64.
#
# Usage: packaging/windows/build-installer.sh [--arch x64|arm64] [--skip-build] [extra ISCC arguments...]
#   --arch        the exe's architecture, x64 by default. arm64 packages
#                 target/aarch64-pc-windows-msvc/release/featherplay.exe, where
#                 `cargo build --target aarch64-pc-windows-msvc` puts it (release.yml puts
#                 the exe it built on an ARM64 runner there too)
#   --skip-build  package the exe already there instead of building it
#   extra arguments go to ISCC as they are (no MSYS path conversion), e.g.
#   /DCredService=<name> for the installer test, or /O<dir> for another output folder
#
# Needs:
# - Inno Setup 6.7 or later: ISCC=<path to ISCC.exe>, or ISCC on PATH, or a standard install.
# - The Visual C++ runtime the exe was linked against, which the installer ships beside the
#   exe: VCREDIST_DIR=<its Microsoft.VC14x.CRT folder>, or this machine's MSVC build tools
#   for the architecture (find-vcredist.sh).
# - cargo. CARGO_CMD=<command> runs the build through something else, such as a wrapper
#   that waits for a build lock (an exported `cargo` shell function is used as well).
#   Building the arm64 exe on an x64 PC also needs the ARM64 MSVC build tools and
#   `rustup target add aarch64-pc-windows-msvc`.
# - The build keys LASTFM_API_KEY, LASTFM_API_SECRET and DISCORD_CLIENT_ID, compiled in with
#   option_env!: in the environment, or in a KEY=value file named by FEATHERPLAY_ENV. The
#   script refuses to build without them (ALLOW_NO_KEYS=1 builds a keyless exe anyway). It
#   never prints them.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

die() {
    echo "build-installer: $*" >&2
    exit 1
}

arch=x64
skip_build=
while [ $# -gt 0 ]; do
    case $1 in
    --arch)
        [ $# -ge 2 ] || die "--arch needs x64 or arm64"
        arch=$2
        shift 2
        ;;
    --skip-build)
        skip_build=1
        shift
        ;;
    *) break ;;
    esac
done

case $arch in
x64)
    target_args=()
    exe=target/release/featherplay.exe
    setup_suffix=
    ;;
arm64)
    target_args=(--target aarch64-pc-windows-msvc)
    exe=target/aarch64-pc-windows-msvc/release/featherplay.exe
    setup_suffix=-arm64
    ;;
*) die "--arch: '$arch' isn't x64 or arm64" ;;
esac

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
    ${CARGO_CMD:-cargo} build --release --locked -p featherplay "${target_args[@]}"
fi

[ -f "$exe" ] || die "no $exe; build it first, or drop --skip-build"

# build.rs renders the icon and the wizard images into its OUT_DIR. They don't depend on the
# architecture, so an x64 build's serve the arm64 setup too. Old build dirs can linger, so
# take the newest.
icon=$(ls -t target/release/build/featherplay-*/out/featherplay.ico \
    target/aarch64-pc-windows-msvc/release/build/featherplay-*/out/featherplay.ico 2>/dev/null |
    head -1 || true)
[ -n "$icon" ] || die "no featherplay.ico under target/release/build; build the release exe"
art=$(dirname "$icon")
for f in installer-small installer-panel installer-small-light installer-panel-light; do
    [ -f "$art/$f.png" ] || die "no $art/$f.png; rebuild the release exe"
done

# The Visual C++ runtime the exe links (Skia is C++): MSVCP140.dll, VCRUNTIME140.dll and
# VCRUNTIME140_1.dll. A clean Windows doesn't have it, and without it the exe doesn't start
# (STATUS_DLL_NOT_FOUND), so the installer puts it beside the exe (app-local). It must be no
# older than the toolset that linked the exe: VCREDIST_DIR=<its Microsoft.VC14x.CRT folder>
# names the one from the machine that built it (release.yml does for the ARM64 exe), else
# find-vcredist.sh takes this machine's newest.
crt=${VCREDIST_DIR:-}
if [ -z "$crt" ]; then
    crt=$(packaging/windows/find-vcredist.sh "$arch") || die "set VCREDIST_DIR, or install the MSVC build tools for $arch"
fi
# Every runtime DLL the exe imports is shipped: the .iss lists them by name, so a new one (a
# toolset or Skia update) would install fine and fail on a PC without the redistributable.
shipped="msvcp140.dll vcruntime140.dll"
[ -f "$crt/vcruntime140_1.dll" ] && shipped="$shipped vcruntime140_1.dll"
for f in $shipped; do
    [ -f "$crt/$f" ] || die "no $f for $arch in '$crt'"
done
for dll in $(grep -aoiE '(msvcp|vcruntime|concrt|vccorlib)[0-9a-z_]*\.dll' "$exe" | tr 'A-Z' 'a-z' | sort -u); do
    case " $shipped " in
    *" $dll "*) ;;
    *) die "$exe imports $dll, which the installer doesn't ship; add it to featherplay.iss and here" ;;
    esac
done

mkdir -p target/installer
win() { cygpath -w "$1"; }
# ISCC's switches start with "/", which MSYS would otherwise rewrite as paths.
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$iscc" /Qp \
    "/DAppVersion=$version" \
    "/DArch=$arch" \
    "/DExeFile=$(win "$root/$exe")" \
    "/DArtDir=$(win "$root/$art")" \
    "/DCrtDir=$(win "$crt")" \
    "/O$(win "$root/target/installer")" \
    "$@" \
    "$(win "$root/packaging/windows/featherplay.iss")"

echo "featherplay.exe ($arch): $(stat -c %s "$exe") bytes"
ls -l "target/installer/featherplay-setup-$version$setup_suffix.exe" 2>/dev/null || true
