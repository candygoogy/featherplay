#!/usr/bin/env bash
# Builds featherplay's Linux release files from the release binary, for this machine's
# architecture (`uname -m`: x86_64, or aarch64):
#   target/linux/featherplay-<arch>.AppImage   updates itself (crates/controller/src/updates/install.rs)
#   target/linux/featherplay-<arch>.tar.gz     the same files as a plain archive; only told of updates
# Both hold the binary, the desktop entry, the icons (the hicolor PNGs build.rs renders, and
# the SVG), the AppStream metadata, LICENSE and THIRD_PARTY.md (the AppImage under
# usr/share/doc/featherplay/, the archive at its top, with README.txt). The archive bundles
# no libraries: those the binary needs are a desktop's own (README.txt lists them). The
# AppImage also carries, in usr/lib/, those of them that the AppImage excludelist doesn't
# expect every system to have (xkbcommon and some X11 libraries; see `bundled` below), so it
# starts on a bare system too.
#
# Usage: packaging/linux/build-linux.sh [--skip-build]
#   --skip-build  package the binary already at target/release/featherplay
#
# Needs: curl, sha256sum, tar, gzip, readelf, ldconfig, patchelf; cargo unless --skip-build.
# Uses desktop-file-validate and appstreamcli to check the desktop entry and the metadata
# when they're installed (release.yml installs both), and says so when they aren't.
#
# appimagetool and the AppImage runtime are downloaded into TOOLS_DIR (default
# target/linux-tools), pinned below by version and SHA256, and checked before use. The
# runtime is the static type 2 runtime: it needs no libfuse on the user's system, only
# fusermount (or --appimage-extract-and-run without FUSE).
#
# MAX_GLIBC=<x.y>: fail if the binary needs a newer glibc than that (release.yml sets the
#   build image's, so the AppImage runs wherever that glibc or a newer one is).
# MAX_BIN_BYTES=<n>: fail if the binary is larger (release.yml sets the size budget).
# Build keys, without --skip-build: LASTFM_API_KEY, LASTFM_API_SECRET and DISCORD_CLIENT_ID
#   in the environment or in the KEY=value file FEATHERPLAY_ENV names, as for
#   packaging/windows/build-installer.sh; ALLOW_NO_KEYS=1 builds without them. Never printed.
# CARGO_CMD=<command> runs the build through something else (a nice'd cargo, say).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

die() {
    echo "build-linux: $*" >&2
    exit 1
}

skip_build=
case ${1:-} in
--skip-build) skip_build=1 ;;
"") ;;
*) die "unknown argument '$1'" ;;
esac

app_id=dev.candygoogy.featherplay
arch=$(uname -m)
# Pins: appimagetool 1.9.1 and type2-runtime 20251108 (AppImage/appimagetool and
# AppImage/type2-runtime on GitHub), each file's SHA256 as its release lists it.
appimagetool_version=1.9.1
runtime_version=20251108
case $arch in
x86_64)
    appimagetool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
    runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
    elf_machine="Advanced Micro Devices X86-64"
    ld_tag=x86-64
    ;;
aarch64)
    appimagetool_sha256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
    runtime_sha256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
    elf_machine="AArch64"
    ld_tag=AArch64
    ;;
*) die "no AppImage tools pinned for $arch" ;;
esac

# The version the binary reports (the workspace's).
pkgid=$(cargo pkgid -p featherplay)
version=${pkgid##*[@#]}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' isn't x.y.z"

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

bin=target/release/featherplay
[ -f "$bin" ] || die "no $bin; build it first, or drop --skip-build"

# The binary: this architecture, within the size budget and the glibc it may need.
machine=$(readelf -h "$bin" | sed -n 's/^ *Machine: *//p')
[ "$machine" = "$elf_machine" ] || die "$bin is for '$machine', not $arch"
size=$(stat -c %s "$bin")
echo "featherplay ($arch): $size bytes${MAX_BIN_BYTES:+ (limit $MAX_BIN_BYTES)}"
if [ -n "${MAX_BIN_BYTES:-}" ] && [ "$size" -gt "$MAX_BIN_BYTES" ]; then
    die "featherplay is $size bytes, over the $MAX_BIN_BYTES byte budget"
fi
glibc=$(readelf -W --dyn-syms "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
echo "needs glibc $glibc${MAX_GLIBC:+ (limit $MAX_GLIBC)}; links:" \
    "$(readelf -d "$bin" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p' | tr '\n' ' ')"
if [ -n "${MAX_GLIBC:-}" ] && [ "$(printf '%s\n' "$glibc" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
    die "featherplay needs glibc $glibc, newer than $MAX_GLIBC"
fi

# The icons build.rs rendered. Old build dirs can linger, so take the newest.
icons=$(ls -td target/release/build/featherplay-*/out/hicolor 2>/dev/null | head -1 || true)
[ -n "$icons" ] && [ -f "$icons/256.png" ] || die "no hicolor icons under target/release/build; rebuild the release binary"

# The release date for the metadata: CHANGELOG.md's for this version, else the last
# commit's (a dry run of an unreleased version).
date=$(sed -n "s/^## \[$version\] - \([0-9-]*\).*/\1/p" CHANGELOG.md | head -1)
[ -n "$date" ] || date=$(git log -1 --format=%cs 2>/dev/null || date -u +%F)

# share/: what both files install.
out=target/linux
stage=$out/stage
rm -rf "$stage"
mkdir -p "$stage/share/applications" "$stage/share/metainfo" "$stage/share/icons/hicolor/scalable/apps"
install -m 644 packaging/linux/$app_id.desktop "$stage/share/applications/"
sed -e "s/@VERSION@/$version/" -e "s/@DATE@/$date/" packaging/linux/$app_id.metainfo.xml \
    >"$stage/share/metainfo/$app_id.metainfo.xml"
chmod 644 "$stage/share/metainfo/$app_id.metainfo.xml"
for png in "$icons"/*.png; do
    n=$(basename "$png" .png)
    mkdir -p "$stage/share/icons/hicolor/${n}x${n}/apps"
    install -m 644 "$png" "$stage/share/icons/hicolor/${n}x${n}/apps/$app_id.png"
done
install -m 644 apps/featherplay/ui/logo/tri-vane.svg "$stage/share/icons/hicolor/scalable/apps/$app_id.svg"

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$stage/share/applications/$app_id.desktop"
else
    echo "build-linux: desktop-file-validate not installed; the desktop entry is unchecked"
fi
if command -v appstreamcli >/dev/null; then
    # Errors fail the build; warnings and notes are printed. Older AppStream versions (0.15
    # on Ubuntu 22.04) exit non-zero for warnings too, and warn about tags they don't know.
    report=$(appstreamcli validate --no-net "$stage/share/metainfo/$app_id.metainfo.xml" 2>&1) || true
    echo "$report"
    if grep -q '^E:' <<<"$report"; then
        die "the AppStream metadata has errors"
    fi
else
    echo "build-linux: appstreamcli not installed; the metadata is unchecked"
fi

# The tools, downloaded once and checked every time.
tools=${TOOLS_DIR:-target/linux-tools}
mkdir -p "$tools"
fetch() {
    local url=$1 file=$2 sha256=$3
    if [ ! -f "$file" ]; then
        curl -fsSL --retry 3 -o "$file.part" "$url"
        mv "$file.part" "$file"
    fi
    echo "$sha256  $file" | sha256sum -c --quiet - || {
        rm -f "$file"
        die "$file doesn't match its pinned SHA256"
    }
}
appimagetool=$tools/appimagetool-$appimagetool_version-$arch.AppImage
runtime=$tools/runtime-$runtime_version-$arch
fetch "https://github.com/AppImage/appimagetool/releases/download/$appimagetool_version/appimagetool-$arch.AppImage" \
    "$appimagetool" "$appimagetool_sha256"
fetch "https://github.com/AppImage/type2-runtime/releases/download/$runtime_version/runtime-$arch" \
    "$runtime" "$runtime_sha256"
chmod +x "$appimagetool"

# The AppImage: AppRun is the binary itself.
appdir=$out/featherplay.AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin"
cp -r "$stage/share" "$appdir/usr/"
# The licences, where a distribution puts a package's (not in $stage/share: the archive's
# README has its users copy that folder into ~/.local/share).
install -Dm 644 -t "$appdir/usr/share/doc/featherplay" LICENSE THIRD_PARTY.md
install -m 755 "$bin" "$appdir/usr/bin/featherplay"

# The libraries the binary loads for its window (xkbcommon-dl and x11-dl dlopen them) that
# the AppImage excludelist leaves to the AppImage
# (https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist), with the
# ones they link that it leaves too, copied from this machine: the build image, so the
# oldest versions supported. The rest of what it loads (glibc, libstdc++, Fontconfig,
# FreeType, ALSA, libX11, libX11-xcb, libxcb, libwayland-client) is on the excludelist.
# The AppImage's binary finds these first by its RUNPATH, $ORIGIN/../lib (which its dlopen
# calls search too), and each of them the others by its own, $ORIGIN. The archive's binary
# is left as it is, and there's no AppRun with LD_LIBRARY_PATH: that would reach the
# programs featherplay starts (the browser, yt-dlp, Deno).
bundled=(libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1 libXcursor.so.1 libXi.so.6
    libXrender.so.1 libXfixes.so.3 libXext.so.6)
system_libs=(libc.so.6 ld-linux-x86-64.so.2 ld-linux-aarch64.so.1 libX11.so.6 libxcb.so.1)
ldconfig=$(command -v ldconfig || echo /sbin/ldconfig)
mkdir -p "$appdir/usr/lib"
for so in "${bundled[@]}"; do
    path=$("$ldconfig" -p | awk -v so="$so" -v tag="$ld_tag" '$1 == so && index($0, tag) { print $NF; exit }')
    [ -n "$path" ] || die "no $so here to bundle (its package is a dependency of the -dev packages)"
    lib=$appdir/usr/lib/$so
    install -m 644 "$(readlink -f "$path")" "$lib"
    for needed in $(readelf -d "$lib" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p'); do
        [[ " ${bundled[*]} ${system_libs[*]} " == *" $needed "* ]] ||
            die "$so links $needed, which is neither bundled nor left to the system"
    done
    # shellcheck disable=SC2016 # $ORIGIN is for the loader
    patchelf --set-rpath '$ORIGIN' "$lib"
done
# shellcheck disable=SC2016
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/featherplay"
readelf -d "$appdir/usr/bin/featherplay" | grep -q 'RUNPATH.*\[\$ORIGIN/\.\./lib\]' ||
    die "the AppImage's binary didn't get its RUNPATH"
echo "the AppImage carries ${bundled[*]}"

ln -s usr/bin/featherplay "$appdir/AppRun"
cp "$stage/share/applications/$app_id.desktop" "$appdir/"
cp "$stage/share/icons/hicolor/256x256/apps/$app_id.png" "$appdir/"
ln -s "$app_id.png" "$appdir/.DirIcon"
image=$out/featherplay-$arch.AppImage
rm -f "$image"
# No update information: the app updates itself from the signed release.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=$arch VERSION=$version \
    "$appimagetool" --no-appstream --runtime-file "$runtime" "$appdir" "$image"
chmod 755 "$image"

# The archive: featherplay/ with the binary, share/, and the notes.
tree=$out/tar/featherplay
rm -rf "$out/tar"
mkdir -p "$tree"
install -m 755 "$bin" "$tree/featherplay"
cp -r "$stage/share" "$tree/"
install -m 644 packaging/linux/README.txt LICENSE THIRD_PARTY.md "$tree/"
archive=$out/featherplay-$arch.tar.gz
tar -C "$out/tar" --owner=0 --group=0 --numeric-owner -czf "$archive" featherplay

ls -l "$image" "$archive"
