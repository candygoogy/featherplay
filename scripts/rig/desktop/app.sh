#!/usr/bin/env bash
# The real app on a headless display, driven by the performance harness's tour
# (tour.txt) on the demo fixtures, with a screenshot after each step:
#
#   app.sh x11       Xvfb. Also: the tray item and its menu, MPRIS (metadata, Pause, Play,
#                    Next) and one notification per song change (checks.py); a second
#                    launch exits and asks the first to show; config, cache and state in
#                    the XDG folders, nothing else in HOME; the requests go to the proxy
#                    HTTPS_PROXY names; no script lacks a font (the app's log), and with
#                    the fonts taken away the log says which.
#   app.sh wayland   sway's headless backend (no Xwayland): a native Wayland window
#                    (server-side decorations), screenshots with grim; FEATHERPLAY_HOME
#                    in place of the XDG folders.
#   app.sh weston    weston's headless backend: no server-side decorations, so the title
#                    bar is the one winit draws itself, as on GNOME. Screenshots with
#                    weston-screenshooter.
#
# Needs `session.sh start` first, and the app built with the harness (lib.sh, APP).
# Screenshots, the harness's records, the app's log and the D-Bus checks' output go to
# RIG/out/<mode>.
set -uo pipefail
. "$(dirname "$0")/lib.sh"

mode="${1:-}"
case "$mode" in
  x11 | wayland | weston) ;;
  *)
    echo "usage: app.sh x11|wayland|weston" >&2
    exit 2
    ;;
esac
[ -x "$APP" ] || die "no app at $APP (cargo build -p featherplay --features perf-harness)"
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || die "no session: run session.sh start first"

out="$OUT/$mode"
rm -rf "$out"
mkdir -p "$out"
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
}
trap cleanup EXIT

# ----- The display ---------------------------------------------------------------------

start_x11() {
  say "Xvfb"
  xvfb 99 "$out/xvfb.log"
  [ -n "$XVFB_PID" ] && pids+=("$XVFB_PID")
  pass "$mode: X display $DISPLAY"
}

start_sway() {
  say "sway, headless"
  cat >"$out/sway.config" <<'EOF'
output HEADLESS-1 resolution 1600x1000 position 0,0
xwayland disable
EOF
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway --config "$out/sway.config" >"$out/sway.log" 2>&1 &
  pids+=($!)
  local sock
  wait_for 20 compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" || die "sway didn't start (sway.log)"
  sock=$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" | head -1)
  export SWAYSOCK="$sock"
  wait_for 10 swaymsg -t get_outputs || die "sway doesn't answer on $SWAYSOCK"
  WAYLAND_DISPLAY=$(basename "$(compgen -G "$XDG_RUNTIME_DIR/wayland-*" | grep -v '\.lock$' | head -1)")
  export WAYLAND_DISPLAY
  unset DISPLAY
  pass "$mode: sway on $WAYLAND_DISPLAY, $(swaymsg -t get_outputs -r | python3 -c 'import json,sys; o=json.load(sys.stdin)[0]; print(o["name"], o["current_mode"]["width"], "x", o["current_mode"]["height"])')"
}

start_weston() {
  say "weston, headless"
  export WAYLAND_DISPLAY=wayland-weston
  unset DISPLAY
  # --debug lets weston-screenshooter capture the screen.
  local common=(--backend=headless --width=1600 --height=1000 --socket="$WAYLAND_DISPLAY"
    --idle-time=0 --debug)
  weston "${common[@]}" --renderer=pixman >"$out/weston.log" 2>&1 &
  pids+=($!)
  if ! wait_for 15 test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"; then
    # Older westons name the software renderer otherwise.
    weston "${common[@]}" --use-pixman >"$out/weston.log" 2>&1 &
    pids+=($!)
    wait_for 15 test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" || die "weston didn't start (weston.log)"
  fi
  pass "$mode: weston on $WAYLAND_DISPLAY"
}

# shot <name>: the screen as RIG/out/<mode>/<name>.png.
shot() {
  local png="$out/$1.png"
  case "$mode" in
    x11)
      local win
      win=$(xdotool search --onlyvisible --name '^featherplay' 2>/dev/null | head -1)
      import -window "${win:-root}" "$png" 2>>"$out/shots.log"
      ;;
    wayland) grim "$png" 2>>"$out/shots.log" ;;
    weston)
      (cd "$out" && weston-screenshooter >>"$out/shots.log" 2>&1)
      local f
      f=$(ls -t "$out"/wayland-screenshot*.png 2>/dev/null | head -1)
      [ -n "$f" ] && mv "$f" "$png"
      ;;
  esac
  if [ -s "$png" ]; then
    echo "screenshot $1.png"
  else
    echo "::warning::$mode: no screenshot for $1"
  fi
}

# ----- The app -------------------------------------------------------------------------

h=$(app_home "$mode")
cp "$here/tour.txt" "$out/tour.txt"
app_env=(FEATHERPLAY_PERF_SCRIPT="$out/tour.txt" FEATHERPLAY_PERF_OUT="$out/perf.jsonl")
log_file="$h/xdg/state/featherplay/logs/featherplay.log"
if [ "$mode" = x11 ]; then
  # Every request through a local proxy that refuses them: none leaves the machine, and
  # the proxy writes down what was asked.
  proxy_port=$(free_port)
  : >"$out/proxy.jsonl"
  python3 "$here/proxy.py" "$proxy_port" "$out/proxy.jsonl" deny >"$out/proxy.log" 2>&1 &
  pids+=($!)
  wait_for 10 grep -q listening "$out/proxy.jsonl" || die "the proxy didn't start"
  app_env+=(HTTPS_PROXY="http://127.0.0.1:$proxy_port" HTTP_PROXY="http://127.0.0.1:$proxy_port")
else
  # FEATHERPLAY_HOME overrides the XDG folders.
  mkdir -p "$h/fphome/config"
  cp "$h/xdg/config/featherplay/settings.toml" "$h/fphome/config/"
  app_env+=(FEATHERPLAY_HOME="$h/fphome")
  log_file="$h/fphome/cache/logs/featherplay.log"
fi

case "$mode" in
  x11) start_x11 ;;
  wayland) start_sway ;;
  weston) start_weston ;;
esac

say "$mode: the app, on the tour"
in_home "$h" env "${app_env[@]}" "$APP" >"$out/stderr.log" 2>&1 &
app=$!
pids+=($app)
echo "app pid $app"

failed=0
check() {
  # check <description> <command...>: runs a check, keeps going on failure.
  local what="$1"
  shift
  if "$@"; then
    pass "$mode: $what"
  else
    echo "::error::$mode: $what failed"
    echo "FAIL: $mode: $what" >>"$OUT/summary.txt"
    failed=1
  fi
}

step() {
  # step <step> <arg> <shot name> [--in-time]: waits for the harness's record of it, then a
  # screenshot. --in-time: the step mustn't have timed out.
  python3 "$here/checks.py" wait-step "$out/perf.jsonl" "$1" "$2" --timeout 90 "${@:4}" \
    >>"$out/steps.log" || {
    tail -n 2 "$out/steps.log"
    return 1
  }
  sleep 0.8
  shot "$3"
}

check "startup, then Home" step startup "" 01-home
check "an album" step open-card 3 02-album
check "a playlist, its first song audible" step play-row 0 03-playlist-playing --in-time
check "search in Japanese, Korean, Chinese, Arabic and emoji" step search "米津玄師 봄날 月亮代表我的心 أنا لحبيبي 🔥🎵✨" 04-search-scripts
check "Now playing" step go now-playing 05-now-playing

if [ "$mode" = x11 ]; then
  say "x11: tray, MPRIS, notifications"
  check "the tray item and its menu" python3 "$here/checks.py" tray "$app" "$OUT/standins.jsonl"
  check "MPRIS: metadata, Pause, Play, Next; one notification per song" \
    python3 "$here/checks.py" mpris "$app" "$OUT/standins.jsonl"
  shot 06-after-next

  say "x11: single instance"
  second_ok() {
    compgen -G "$XDG_RUNTIME_DIR/featherplay-*.sock" >/dev/null || {
      echo "no featherplay socket in XDG_RUNTIME_DIR"
      return 1
    }
    in_home "$h" env FEATHERPLAY_PERF_SCRIPT= timeout 30 "$APP" >"$out/second-launch.log" 2>&1
    local code=$?
    cat "$out/second-launch.log"
    [ "$code" = 0 ] && grep -q "already running; it showed its window" "$out/second-launch.log"
  }
  check "a second launch asks the first to show its window and exits" second_ok
  kill -0 "$app" 2>/dev/null || die "x11: the first app ended after the second launch"

  say "x11: quit from the tray menu"
  check "the tray menu's Quit" python3 "$here/checks.py" tray-quit "$app" "$OUT/standins.jsonl"
else
  if [ "$mode" = wayland ]; then
    swaymsg -t get_tree >"$out/sway-tree.json"
    native() {
      python3 - "$out/sway-tree.json" <<'EOF'
import json, sys
def walk(n):
    yield n
    for c in n.get("nodes", []) + n.get("floating_nodes", []):
        yield from walk(c)
views = [n for n in walk(json.load(open(sys.argv[1]))) if n.get("pid")]
for v in views:
    print(f"window {v.get('name')!r}: app_id={v.get('app_id')!r} shell={v.get('shell')}")
sys.exit(0 if views and all(v.get("shell") == "xdg_shell" for v in views) else 1)
EOF
    }
    check "the window is a native Wayland one (xdg_shell)" native
  fi
  check "MPRIS Quit" python3 "$here/checks.py" mpris-quit "$app"
fi

say "$mode: the app ends"
ended() {
  local end=$((SECONDS + 30))
  while kill -0 "$app" 2>/dev/null; do
    if [ "$SECONDS" -ge "$end" ]; then
      tail -n 15 "$out/stderr.log"
      return 1
    fi
    sleep 0.2
  done
  wait "$app"
  local code=$?
  echo "exit code $code"
  [ "$code" = 0 ]
}
check "the app quits cleanly within 30 s" ended

cp "$log_file" "$out/featherplay.log" 2>/dev/null || echo "::warning::$mode: no log at $log_file"

say "$mode: fonts"
# The app logs once which scripts no font here has (crates/platform/src/linux/fonts.rs).
# Noto CJK lacks CJK extension B, so that one may be named; the tour's scripts may not.
fonts_found() {
  grep -E "fonts: (body text|no font)" "$out/featherplay.log"
  local line
  line=$(grep -h "no font here has" "$out/featherplay.log")
  for script in emoji Chinese Japanese Korean Arabic; do
    if [[ "$line" == *"$script"* ]]; then
      echo "no font for $script"
      return 1
    fi
  done
}
check "the log names none of Japanese, Korean, Chinese, Arabic, emoji as without a font" fonts_found

if [ "$mode" = x11 ]; then
  say "x11: XDG folders"
  find "$h" -path "$h/xdg/data" -prune -o -print | sed "s|^$h/||" | sort >"$out/files.txt"
  xdg_ok() {
    [ -f "$h/xdg/state/featherplay/logs/featherplay.log" ] || {
      echo "no log in XDG_STATE_HOME"
      return 1
    }
    [ -f "$h/xdg/config/featherplay/settings.toml" ] || {
      echo "no settings in XDG_CONFIG_HOME"
      return 1
    }
    local stray
    stray=$(find "$h/home" -ipath '*featherplay*')
    [ -z "$stray" ] || {
      echo "featherplay files in HOME, outside the XDG folders: $stray"
      return 1
    }
  }
  check "config and state in the XDG folders, nothing in HOME (files.txt)" xdg_ok

  say "x11: proxy"
  proxied() {
    grep -iE "proxy" "$out/featherplay.log" | head -5
    local asked
    asked=$(grep -cv '"method": "-"' "$out/proxy.jsonl")
    echo "the proxy saw $asked requests:"
    grep -v '"method": "-"' "$out/proxy.jsonl" | head -20
    grep -q "127.0.0.1:$proxy_port" "$out/featherplay.log"
  }
  check "the app's log names the proxy from HTTPS_PROXY" proxied
  if [ "$(grep -cv '"method": "-"' "$out/proxy.jsonl")" = 0 ]; then
    echo "::notice::x11: the fixture run made no request; the helpers' download checks the proxy (live.sh)"
  else
    pass "x11: the app's requests went to the proxy (proxy.jsonl)"
  fi

  say "x11: with the fallback fonts taken away"
  # The same start with only DejaVu (and the app's bundled Inter): the log must name the
  # scripts that now have no font, so the check above isn't vacuous.
  ctl="$RIG/homes/x11-nofonts"
  rm -rf "$ctl"
  mkdir -p "$ctl/fphome" "$ctl/fontconfig-cache"
  cat >"$ctl/fonts.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts/truetype/dejavu</dir>
  <cachedir>$ctl/fontconfig-cache</cachedir>
</fontconfig>
EOF
  printf 'startup\nquit\n' >"$ctl/tour.txt"
  nofonts() {
    in_home "$h" env FONTCONFIG_FILE="$ctl/fonts.conf" FEATHERPLAY_HOME="$ctl/fphome" \
      FEATHERPLAY_PERF_SCRIPT="$ctl/tour.txt" FEATHERPLAY_PERF_OUT="$ctl/perf.jsonl" \
      timeout 90 "$APP" >"$out/nofonts-stderr.log" 2>&1
    local line
    line=$(grep -h "no font here has" "$ctl/fphome/cache/logs/featherplay.log" 2>/dev/null)
    echo "$line"
    cp "$ctl/fphome/cache/logs/featherplay.log" "$out/nofonts-featherplay.log" 2>/dev/null
    for script in emoji Chinese Japanese Korean; do
      [[ "$line" == *"$script"* ]] || {
        echo "the line doesn't name $script"
        return 1
      }
    done
  }
  check "without the Noto fonts the log names emoji, Chinese, Japanese and Korean" nofonts
fi

exit "$failed"
