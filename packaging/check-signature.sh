#!/usr/bin/env bash
# Checks that a release's SHA256SUMS.txt carries a valid minisign signature
# (SHA256SUMS.txt.minisig) by one of the keys built into featherplay, made for that version
# (trusted comment `featherplay vX.Y.Z`). That's what installed copies check before they
# download an update (crates/controller/src/updates/install.rs, `verify_sums`): a release
# published without it updates nobody. RELEASING.md runs it before publishing.
#
# Usage: packaging/check-signature.sh X.Y.Z [folder]
#   With a folder: the two files in it. Without: downloaded from the vX.Y.Z release (the
#   draft) into a temporary folder.
#
# MINISIGN: the minisign exe, 0.12 or later (default: minisign on PATH).
# FEATHERPLAY_UPDATE_KEYS: the public key files to accept, separated by spaces (default:
#   apps/featherplay/keys/update.pub and update-backup.pub). Tests point it at test keys.
# FEATHERPLAY_REPO: the GitHub repository (default candygoogy/featherplay).
set -euo pipefail

version=${1:?usage: check-signature.sh X.Y.Z [folder]}
folder=${2:-}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "check-signature: $version isn't X.Y.Z" >&2
    exit 2
}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
repo=${FEATHERPLAY_REPO:-candygoogy/featherplay}
minisign=${MINISIGN:-minisign}
command -v "$minisign" >/dev/null || {
    echo "check-signature: no minisign: put it on PATH, or set MINISIGN to its exe" >&2
    exit 2
}
keys=("$root/apps/featherplay/keys/update.pub" "$root/apps/featherplay/keys/update-backup.pub")
if [ -n "${FEATHERPLAY_UPDATE_KEYS:-}" ]; then
    read -r -a keys <<<"$FEATHERPLAY_UPDATE_KEYS"
fi

refuse() {
    echo >&2
    echo "check-signature: NOT SIGNED: $1" >&2
    echo "check-signature: don't publish v$version. Sign it first: packaging/sign-release.sh $version <folder>" >&2
    exit 1
}

if [ -z "$folder" ]; then
    folder=$(mktemp -d)
    trap 'rm -rf "$folder"' EXIT
    # A missing file is told below.
    gh release download "v$version" --repo "$repo" --dir "$folder" \
        --pattern SHA256SUMS.txt --pattern SHA256SUMS.txt.minisig || true
fi
sums=$folder/SHA256SUMS.txt
sig=$folder/SHA256SUMS.txt.minisig
[ -f "$sums" ] || refuse "v$version has no SHA256SUMS.txt"
[ -f "$sig" ] || refuse "v$version has no SHA256SUMS.txt.minisig"

expected="featherplay v$version"
for key in "${keys[@]}"; do
    # -Q prints only the trusted comment, once the signature and the comment check out.
    if comment=$("$minisign" -V -H -Q -p "$key" -m "$sums" -x "$sig" 2>/dev/null); then
        comment=${comment%$'\r'}
        [ "$comment" = "$expected" ] ||
            refuse "the signature is for \"$comment\", not \"$expected\""
        echo "check-signature: v$version's SHA256SUMS.txt is signed with $(basename "$key") for \"$expected\""
        exit 0
    fi
done
refuse "SHA256SUMS.txt.minisig doesn't verify with any of: ${keys[*]}"
