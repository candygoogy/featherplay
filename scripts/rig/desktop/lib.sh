# Shared by the desktop rig's scripts: sourced, not run.
#
# RIG is the rig's working folder (default: rig/ under RUNNER_TEMP, else under the repo's
# target/). Everything the rig makes goes there: the session's sockets, the homes the app
# runs in, the logs, the screenshots (RIG/out, the artifact).

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
RIG="${RIG:-${RUNNER_TEMP:-$repo/target}/rig}"
OUT="$RIG/out"
mkdir -p "$RIG" "$OUT"

# The session's bus and runtime folder, once `session.sh start` has made them.
if [ -f "$RIG/session.env" ]; then
  # shellcheck disable=SC1091
  . "$RIG/session.env"
fi

# The app the rig runs: a dev build with the performance harness (`cargo build -p featherplay
# --features perf-harness`), unless FEATHERPLAY_BIN names another.
APP="${FEATHERPLAY_BIN:-$repo/target/debug/featherplay}"

say() { printf '\n== %s\n' "$*"; }

# A failed check: a GitHub error annotation and a note in the summary, then exit 1.
die() {
  echo "::error::$*"
  echo "FAIL: $*" >>"$OUT/summary.txt"
  exit 1
}

pass() {
  echo "ok: $*"
  echo "ok: $*" >>"$OUT/summary.txt"
}

# wait_for <seconds> <command...>: runs the command until it succeeds.
wait_for() {
  local secs="$1"
  shift
  local end=$((SECONDS + secs))
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.2
  done
}

# xvfb <n> <log>: an Xvfb on :n, DISPLAY set to it; its pid in XVFB_PID. With RIG_X_DISPLAY
# set (a local run in WSLg, which has no Xvfb), that display instead.
xvfb() {
  unset WAYLAND_DISPLAY
  XVFB_PID=""
  if [ -n "${RIG_X_DISPLAY:-}" ]; then
    export DISPLAY="$RIG_X_DISPLAY"
    return
  fi
  Xvfb ":$1" -screen 0 1600x1000x24 -nolisten tcp >"$2" 2>&1 &
  XVFB_PID=$!
  export DISPLAY=":$1"
  wait_for 15 xdpyinfo || die "Xvfb didn't start ($2)"
}

has_name() {
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner "$1" 2>/dev/null | grep -q true
}

# A free TCP port on 127.0.0.1.
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

# The environment of one app run, in a home of its own under RIG/homes/<name>: HOME and
# the XDG folders apart from each other (so a file in the wrong one shows), no proxy, the
# demo fixtures through the performance harness, the tour script, notifications on, and
# ALSA's default device on the session's PulseAudio null sink. Prints the home.
app_home() {
  local name="$1" h="$RIG/homes/$1"
  rm -rf "$h"
  mkdir -p "$h/home" "$h/xdg/config/featherplay" "$h/xdg/cache" "$h/xdg/state" "$h/xdg/data"
  printf 'schema = 1\nnotifications = true\n' >"$h/xdg/config/featherplay/settings.toml"
  printf 'pcm.!default pulse\nctl.!default pulse\n' >"$h/home/.asoundrc"
  echo "$h"
}

# Runs "$@" in the app environment of home $1 (from `app_home`).
in_home() {
  local h="$1"
  shift
  env -u FEATHERPLAY_HOME -u HTTPS_PROXY -u https_proxy -u HTTP_PROXY -u http_proxy \
    -u ALL_PROXY -u all_proxy -u NO_PROXY -u no_proxy \
    HOME="$h/home" \
    XDG_CONFIG_HOME="$h/xdg/config" XDG_CACHE_HOME="$h/xdg/cache" \
    XDG_STATE_HOME="$h/xdg/state" XDG_DATA_HOME="$h/xdg/data" \
    FEATHERPLAY_PERF_FIXTURES=1 FEATHERPLAY_FIXTURES=demo \
    FEATHERPLAY_PERF_AUDIO="$repo/site/public/music/california-lullabye.m4a" \
    "$@"
}
