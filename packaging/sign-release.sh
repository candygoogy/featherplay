#!/usr/bin/env bash
# Signs a draft release's SHA256SUMS.txt with the update key, on the maintainer's PC
# (RELEASING.md, step 4). Installed copies download an update only if SHA256SUMS.txt
# carries a valid signature by one of the keys built into featherplay, made for that
# version (apps/featherplay/src/updates/install.rs, `verify_sums`).
#
# Usage: packaging/sign-release.sh [--dry-run] X.Y.Z <folder>
#   1. checks that vX.Y.Z is still a draft, and downloads its files into <folder> (new, or
#      empty);
#   2. checks them against its SHA256SUMS.txt (`sha256sum -c`);
#   3. signs SHA256SUMS.txt with trusted comment `featherplay vX.Y.Z`: minisign asks for the
#      key's password itself; this script never sees the password or reads the key;
#   4. checks the signature, and its comment, with the main public key;
#   5. uploads SHA256SUMS.txt.minisig to the draft.
#   --dry-run: no GitHub at all. Steps 2-4 on the files already in <folder>; nothing uploaded.
#
# MINISIGN: the minisign exe, 0.12 or later (default: minisign on PATH).
# FEATHERPLAY_SIGN_KEY: the secret key file (required; the main key's, or the backup's).
# FEATHERPLAY_UPDATE_KEYS: the public key files, the main one first (default:
#   apps/featherplay/keys/update.pub and update-backup.pub). Only the first is checked here.
# FEATHERPLAY_REPO: the GitHub repository (default candygoogy/featherplay).
set -euo pipefail

dry=0
if [ "${1:-}" = --dry-run ]; then
    dry=1
    shift
fi
usage="usage: sign-release.sh [--dry-run] X.Y.Z <folder>"
version=${1:?$usage}
folder=${2:?$usage}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "sign-release: $version isn't X.Y.Z" >&2
    exit 2
}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=${FEATHERPLAY_REPO:-candygoogy/featherplay}
root=$(cd "$here/.." && pwd)
# Paths stay right after the `cd` into the folder below.
abs() {
    case $1 in
    /* | [A-Za-z]:*) echo "$1" ;;
    *) echo "$PWD/$1" ;;
    esac
}
minisign=${MINISIGN:-minisign}
[ -n "${FEATHERPLAY_SIGN_KEY:-}" ] || {
    echo "sign-release: set FEATHERPLAY_SIGN_KEY to the secret key file" >&2
    exit 2
}
key=$(abs "$FEATHERPLAY_SIGN_KEY")
main_pub=$root/apps/featherplay/keys/update.pub
if [ -n "${FEATHERPLAY_UPDATE_KEYS:-}" ]; then
    read -r -a pubs <<<"$FEATHERPLAY_UPDATE_KEYS"
    main_pub=$(abs "${pubs[0]}")
fi
tag=v$version

fail() {
    echo "sign-release: $1" >&2
    exit 1
}

[ -x "$minisign" ] || command -v "$minisign" >/dev/null || fail "no minisign at $minisign"

# 1. The draft's files.
if [ $dry = 0 ]; then
    draft=$(gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft) || fail "no release $tag"
    [ "$draft" = true ] || fail "$tag is published already; only a draft is signed"
    mkdir -p "$folder"
    [ -z "$(ls -A "$folder")" ] || fail "$folder isn't empty"
    gh release download "$tag" --repo "$repo" --dir "$folder"
fi
cd "$folder"
[ -f SHA256SUMS.txt ] || fail "$tag has no SHA256SUMS.txt"

# 2. SHA256SUMS.txt is the one for these files.
sha256sum -c SHA256SUMS.txt || fail "the files don't match SHA256SUMS.txt"

# 3. The signature. minisign reads the key and asks for its password.
comment="featherplay $tag"
rm -f SHA256SUMS.txt.minisig
"$minisign" -S -s "$key" -t "$comment" -m SHA256SUMS.txt -x SHA256SUMS.txt.minisig ||
    fail "signing failed"

# 4. It verifies with the main public key, for this version. -Q prints only the trusted
# comment, once the signature and the comment check out.
signed_for=$("$minisign" -V -H -Q -p "$main_pub" -m SHA256SUMS.txt -x SHA256SUMS.txt.minisig) ||
    fail "the new signature doesn't verify with $main_pub"
[ "${signed_for%$'\r'}" = "$comment" ] || fail "the new signature is for \"$signed_for\", not \"$comment\""
echo "sign-release: SHA256SUMS.txt is signed for \"$comment\" and verifies with $(basename "$main_pub")"

# 5. Onto the draft.
if [ $dry = 1 ]; then
    echo "sign-release: dry run: would upload $folder/SHA256SUMS.txt.minisig to $tag"
    exit 0
fi
gh release upload "$tag" --repo "$repo" SHA256SUMS.txt.minisig --clobber
echo "sign-release: $tag is signed. Check it from GitHub before publishing: packaging/check-signature.sh $version"
