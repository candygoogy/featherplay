#!/usr/bin/env bash
# Makes the update test's "newer release" for run.sh to serve on 127.0.0.1, as GitHub
# would: in <server>, `latest` (GitHub's releases/latest reply for v<version>), the AppImage
# under its release name, SHA256SUMS.txt, and SHA256SUMS.txt.minisig, signed for
# "featherplay v<version>" with a key pair made here for this run only (in <keys>; never
# featherplay's own). The test build (`update-test-server` feature) trusts that key alone
# through FEATHERPLAY_UPDATE_KEY=<keys>/test.pub.
#
# Usage: MINISIGN=<minisign 0.12> release.sh <newer AppImage> <version> <server> <keys>
set -euo pipefail

image=${1:?usage: release.sh <AppImage> <version> <server dir> <keys dir>}
version=${2:?version}
server=${3:?server dir}
keys=${4:?keys dir}
minisign=${MINISIGN:-minisign}
arch=$(uname -m)
name=featherplay-$arch.AppImage

rm -rf "$server" "$keys"
mkdir -p "$server" "$keys"
install -m 755 "$image" "$server/$name"
(cd "$server" && sha256sum "$name" >SHA256SUMS.txt)
"$minisign" -G -W -p "$keys/test.pub" -s "$keys/test.key" >/dev/null
"$minisign" -S -s "$keys/test.key" -m "$server/SHA256SUMS.txt" \
    -c "featherplay rig test key" -t "featherplay v$version" >/dev/null
"$minisign" -V -p "$keys/test.pub" -m "$server/SHA256SUMS.txt" -q
# Only the public half is needed from here on.
rm -f "$keys/test.key"

dl="https://github.com/candygoogy/featherplay/releases/download/v$version"
cat >"$server/latest" <<EOF
{
  "tag_name": "v$version",
  "html_url": "https://github.com/candygoogy/featherplay/releases/tag/v$version",
  "draft": false,
  "prerelease": false,
  "assets": [
    {"name": "$name", "browser_download_url": "$dl/$name"},
    {"name": "SHA256SUMS.txt", "browser_download_url": "$dl/SHA256SUMS.txt"},
    {"name": "SHA256SUMS.txt.minisig", "browser_download_url": "$dl/SHA256SUMS.txt.minisig"}
  ]
}
EOF
chmod -R a+rX "$server" "$keys"
echo "release.sh: v$version in $server, signed with a key made for this run"
