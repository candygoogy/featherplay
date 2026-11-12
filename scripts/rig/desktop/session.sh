#!/usr/bin/env bash
# The desktop session the rig's runs share, without a desktop:
#
#   session.sh start   a session D-Bus (its own runtime folder), the Secret Service
#                      (gnome-keyring, with an always-unlocked login keyring of an empty
#                      password), the stand-in notification server and tray watcher
#                      (standins.py), and a PulseAudio null sink for ALSA's default device;
#                      writes RIG/session.env for the other scripts.
#   session.sh stop    ends them.
#
# Needs dbus, gnome-keyring, libsecret-tools, pulseaudio, python3-dbus and python3-gi.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

start() {
  local run="$RIG/runtime" logs="$OUT/logs" outer_pulse="${PULSE_SERVER:-}"
  rm -rf "$run" "$RIG/keyring"
  mkdir -p "$run" "$logs" "$RIG/keyring/keyrings"
  chmod 700 "$run"
  export XDG_RUNTIME_DIR="$run"
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$run/bus"
  export PULSE_SERVER="unix:$run/pulse/native"
  # Without PulseAudio installed (a local run in WSL), the sound server there is used.
  if ! command -v pulseaudio >/dev/null && [ -n "$outer_pulse" ]; then
    PULSE_SERVER="$outer_pulse"
  fi
  cat >"$RIG/session.env" <<EOF
export XDG_RUNTIME_DIR='$run'
export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'
export PULSE_SERVER='$PULSE_SERVER'
EOF

  say "session bus"
  dbus-daemon --session --fork --address="$DBUS_SESSION_BUS_ADDRESS" \
    --print-pid=1 >"$RIG/dbus.pid"
  wait_for 10 has_name org.freedesktop.DBus || die "the session bus didn't start"
  pass "session bus at $DBUS_SESSION_BUS_ADDRESS"

  say "Secret Service"
  # The login keyring with an empty password: gnome-keyring keeps it as a plain file that is
  # always unlocked, and `default` makes it the default collection. Only the rig's dummy
  # items go in it.
  printf '[keyring]\ndisplay-name=Login\nctime=0\nmtime=0\nlock-on-idle=false\nlock-after=false\n' \
    >"$RIG/keyring/keyrings/login.keyring"
  printf 'login' >"$RIG/keyring/keyrings/default"
  chmod 600 "$RIG/keyring/keyrings/"*
  XDG_DATA_HOME="$RIG/keyring" setsid gnome-keyring-daemon --foreground --components=secrets \
    >"$logs/gnome-keyring.log" 2>&1 &
  echo $! >"$RIG/keyring.pid"
  wait_for 15 has_name org.freedesktop.secrets || die "gnome-keyring didn't take org.freedesktop.secrets"
  printf 'rig-secret' | secret-tool store --label='featherplay rig probe' rig probe ||
    die "secret-tool couldn't store an item (the keyring is locked?)"
  [ "$(secret-tool lookup rig probe)" = rig-secret ] || die "secret-tool read back something else"
  secret-tool clear rig probe
  [ -z "$(secret-tool lookup rig probe || true)" ] || die "secret-tool clear left the item"
  pass "Secret Service: gnome-keyring, unlocked, stores, reads and clears an item"

  say "stand-in notification server and tray watcher"
  : >"$OUT/standins.jsonl"
  setsid python3 "$here/standins.py" "$OUT/standins.jsonl" >"$logs/standins.log" 2>&1 &
  echo $! >"$RIG/standins.pid"
  wait_for 15 has_name org.freedesktop.Notifications || die "no stand-in notification server"
  wait_for 15 has_name org.kde.StatusNotifierWatcher || die "no stand-in tray watcher"
  pass "stand-ins own org.freedesktop.Notifications and org.kde.StatusNotifierWatcher"

  say "PulseAudio null sink"
  if ! command -v pulseaudio >/dev/null; then
    wait_for 5 pactl info || die "no PulseAudio here and none at PULSE_SERVER"
    pass "PulseAudio: this system's, at $PULSE_SERVER (none installed for the rig)"
    return
  fi
  mkdir -p "$run/pulse"
  pulseaudio -n --daemonize=yes --exit-idle-time=-1 --use-pid-file=yes \
    --log-target="file:$logs/pulseaudio.log" \
    --load="module-native-protocol-unix auth-anonymous=1 socket=$run/pulse/native" \
    --load="module-null-sink sink_name=rig sink_properties=device.description=rig" \
    --load="module-always-sink" ||
    die "PulseAudio didn't start"
  wait_for 15 pactl info || die "PulseAudio doesn't answer"
  pactl set-default-sink rig
  pass "PulseAudio: default sink $(pactl get-default-sink)"
}

stop() {
  for f in standins keyring dbus; do
    if [ -f "$RIG/$f.pid" ]; then
      kill "$(cat "$RIG/$f.pid")" 2>/dev/null || true
      rm -f "$RIG/$f.pid"
    fi
  done
  pulseaudio --kill 2>/dev/null || true
  rm -f "$RIG/session.env"
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *)
    echo "usage: session.sh start|stop" >&2
    exit 2
    ;;
esac
