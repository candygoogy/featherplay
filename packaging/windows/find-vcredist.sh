#!/usr/bin/env bash
# Prints the folder of the newest redistributable Visual C++ runtime for an architecture
# (the build tools' VC/Redist/MSVC/<version>/<arch>/Microsoft.VC14x.CRT), over every Visual
# Studio and Build Tools install vswhere knows. The exe links that runtime (MSVCP140.dll,
# VCRUNTIME140.dll and VCRUNTIME140_1.dll, through Skia), and the installer ships it beside
# the exe: take it from the machine that built the exe, so it's no older than the toolset
# the exe was linked with (release.yml's build-arm64 does that for the ARM64 exe).
#
# Usage: packaging/windows/find-vcredist.sh x64|arm64
set -euo pipefail

arch=${1:?usage: find-vcredist.sh x64|arm64}
vswhere="C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
[ -f "$vswhere" ] || {
    echo "find-vcredist: no vswhere.exe at $vswhere" >&2
    exit 1
}
# Each candidate as "<redist version> <folder>", the newest version last.
crt=$(while IFS= read -r vs; do
    vs=$(cygpath -u "${vs%$'\r'}")
    for d in "$vs"/VC/Redist/MSVC/[0-9]*/"$arch"/Microsoft.VC14*.CRT; do
        [ -d "$d" ] || continue
        v=${d#"$vs"/VC/Redist/MSVC/}
        echo "${v%%/*} $d"
    done
done < <("$vswhere" -all -products '*' -utf8 -property installationPath) |
    sort -V | tail -1 | cut -d' ' -f2-)
[ -n "$crt" ] || {
    echo "find-vcredist: no Visual C++ runtime for $arch; install the MSVC build tools for $arch" >&2
    exit 1
}
echo "$crt"
