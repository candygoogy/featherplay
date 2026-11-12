#!/usr/bin/env bash
# The Linux live tests that need a desktop session, run against the rig's (session.sh):
#
#   - the Secret Service: a dummy item stored, read, replaced and deleted, and the same
#     code with no session bus says so (credentials.rs);
#   - a notification to the stand-in server, the second replacing the first (notify.rs);
#   - MPRIS read and driven over the bus (media_controls/tests.rs);
#   - the browser sign-in's path with a real Chrome or Chromium on Xvfb: the launch with a
#     temporary profile, the pipe on fds 3 and 4, the two commands (Storage.getCookies,
#     Browser.close) against a local page's test cookies, navigator.webdriver false, the
#     profile removed (browsers/tests.rs; the app's own client, pages/sign_in/browser);
#   - the helpers: a download that's cut off is tried again, a wrong SHA-256 is refused
#     and deleted, an installed helper is executable (local server); and the pinned yt-dlp
#     and Deno downloaded from GitHub through a local proxy named by HTTPS_PROXY, checked
#     against their pins and run (sidecars/tests.rs);
#   - titles in other scripts draw no .notdef box with the Noto fonts, and do without them
#     (tests/app/glyph_fallback.rs), so the count isn't vacuous.
#
# No Google page and no account anywhere. Needs `session.sh start` first, Xvfb, a
# Chromium-based browser and cargo-nextest; builds the test programs if needed.
set -uo pipefail
. "$(dirname "$0")/lib.sh"

[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || die "no session: run session.sh start first"
out="$OUT/live"
rm -rf "$out"
mkdir -p "$out"
export FEATHERPLAY_HOME="$RIG/homes/live"
rm -rf "$FEATHERPLAY_HOME"
mkdir -p "$FEATHERPLAY_HOME"
pids=()
trap 'for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done' EXIT

failed=0
# run <name> <cargo nextest args...>: one test run, its output in RIG/out/live/<name>.log.
run() {
  local name="$1"
  shift
  say "$name"
  if (cd "$repo" && cargo nextest run --no-fail-fast --no-capture "$@") >"$out/$name.log" 2>&1; then
    tail -n 3 "$out/$name.log"
    pass "live: $name"
  else
    tail -n 40 "$out/$name.log"
    echo "::error::live: $name failed ($name.log)"
    echo "FAIL: live: $name" >>"$OUT/summary.txt"
    failed=1
    return 1
  fi
}

platform=(-p featherplay-platform --lib --run-ignored only)

run secret-service "${platform[@]}" -E 'test(live_secret_service)'

before=$(grep -c '"call": "Notify"' "$OUT/standins.jsonl")
run notification "${platform[@]}" -E 'test(live_notification)'
sent=$(($(grep -c '"call": "Notify"' "$OUT/standins.jsonl") - before))
if [ "$sent" = 2 ]; then
  pass "live: the stand-in server got the test's 2 notifications"
else
  echo "::error::live: the stand-in server got $sent notifications from the test, not 2"
  echo "FAIL: live: notifications at the stand-in" >>"$OUT/summary.txt"
  failed=1
fi

run mpris "${platform[@]}" -E 'test(live_mpris)'

# The browser needs a display.
xvfb 98 "$out/xvfb.log"
[ -n "$XVFB_PID" ] && pids+=("$XVFB_PID")
browser=""
for b in google-chrome google-chrome-stable chromium chromium-browser; do
  # A snap's wrapper can't take the pipe (the sign-in refuses it too).
  if command -v "$b" >/dev/null && [[ "$(readlink -f "$(command -v "$b")")" != /snap/* ]]; then
    browser=$(command -v "$b")
    echo "browser: $browser: $("$b" --version 2>/dev/null)"
    break
  fi
done
if [ -z "$browser" ]; then
  # aarch64: Google ships no Linux build of Chrome for it, and Ubuntu's Chromium is a snap.
  echo "::notice::live: no usable Chromium-based browser on this $(uname -m) runner; browser sign-in not checked"
  echo "skipped: live: browser sign-in (no browser on $(uname -m))" >>"$OUT/summary.txt"
else
  run browser-pipe "${platform[@]}" -E 'test(live_browser)'
  if ! run browser-app-client -p featherplay --lib --run-ignored only -E 'test(live_browsers)'; then
    # The test stops at the first browser that fails, and starts each with no output: that
    # one again, its log kept, given longer (pipe_probe.py), and the policies it may read.
    exe=$(grep -o 'starting /[^ ]*' "$out/browser-app-client.log" | tail -n 1 | cut -d' ' -f2)
    if [ -n "$exe" ]; then
      say "probe: $exe ($("$exe" --version 2>/dev/null))"
      probe="$out/probe-$(basename "$exe")"
      mkdir -p "$probe"
      python3 "$here/pipe_probe.py" "$exe" "$probe" 60
      grep -v '^$' "$probe/stderr.log" | tail -n 25
      for d in /etc/opt/edge/policies /etc/opt/chrome/policies /etc/chromium/policies; do
        find "$d" -type f 2>/dev/null | while read -r f; do echo "policy $f:"; cat "$f"; done
      done
    fi
  fi
  left=$(find "$FEATHERPLAY_HOME" -maxdepth 3 -iname '*signin*' 2>/dev/null)
  if [ -z "$left" ]; then
    pass "live: no sign-in profile left in the test home"
  else
    echo "::error::live: sign-in profiles left behind: $left"
    echo "FAIL: live: sign-in profiles left" >>"$OUT/summary.txt"
    failed=1
  fi
fi
[ "${#pids[@]}" -gt 0 ] && kill "${pids[@]}" 2>/dev/null
pids=()
unset DISPLAY

run helpers-local -p featherplay-streams --lib \
  -E 'test(a_download_cut_off_mid_file_is_tried_again) | test(sha256_mismatch_rejected) | test(an_installed_helper_is_executable)'

port=$(free_port)
: >"$out/proxy.jsonl"
python3 "$here/proxy.py" "$port" "$out/proxy.jsonl" forward >"$out/proxy.log" 2>&1 &
pids+=($!)
wait_for 10 grep -q listening "$out/proxy.jsonl" || die "the proxy didn't start"
HTTPS_PROXY="http://127.0.0.1:$port" HTTP_PROXY="http://127.0.0.1:$port" \
  run helpers-download-through-proxy -p featherplay-streams --lib --run-ignored only \
  -E 'test(live_the_pinned_releases_install)'
echo "the proxy saw:"
grep -v '"method": "-"' "$out/proxy.jsonl" | head -20
if grep -q '"target": "github.com:443"' "$out/proxy.jsonl"; then
  pass "live: the helpers' download went through the proxy HTTPS_PROXY names"
else
  echo "::error::live: the proxy saw no CONNECT to github.com"
  echo "FAIL: live: helpers' proxy" >>"$OUT/summary.txt"
  failed=1
fi

# Titles in other scripts: with the Noto fonts, no .notdef box; with only DejaVu, boxes.
export FEATHERPLAY_PIXELS=unchecked
mkdir -p "$out/glyphs-noto"
FEATHERPLAY_TEST_SHOTS="$out/glyphs-noto" run glyphs-noto -p featherplay --test app \
  -E 'test(glyph_fallback)'
mkdir -p "$out/glyphs-dejavu-only" "$RIG/fontconfig-cache"
cat >"$RIG/fonts-dejavu-only.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts/truetype/dejavu</dir>
  <cachedir>$RIG/fontconfig-cache</cachedir>
</fontconfig>
EOF
say "glyphs without the Noto fonts"
if (cd "$repo" && FONTCONFIG_FILE="$RIG/fonts-dejavu-only.conf" \
  FEATHERPLAY_TEST_SHOTS="$out/glyphs-dejavu-only" \
  cargo nextest run --no-fail-fast --no-capture -p featherplay --test app -E 'test(glyph_fallback)') \
  >"$out/glyphs-dejavu-only.log" 2>&1; then
  echo "::error::live: with only DejaVu the glyph test still passed: it can't see boxes"
  echo "FAIL: live: glyph test control" >>"$OUT/summary.txt"
  failed=1
elif grep -q "drew as .notdef boxes" "$out/glyphs-dejavu-only.log"; then
  grep -h "drew as .notdef boxes" "$out/glyphs-dejavu-only.log" | head -1
  pass "live: without the Noto fonts the glyph test counts .notdef boxes"
else
  tail -n 30 "$out/glyphs-dejavu-only.log"
  echo "::error::live: the glyph test without Noto failed for another reason"
  echo "FAIL: live: glyph test control" >>"$OUT/summary.txt"
  failed=1
fi

exit "$failed"
