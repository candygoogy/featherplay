#!/usr/bin/env bash
# The Linux sound rig: plays a track through featherplay's audio engine to a null sink and
# checks what the sink received. Headless, no sound hardware.
#
#   scripts/rig/sound/run.sh <pipewire|pulseaudio> <player> <track> <out-dir> <track-seconds>
#
# <player> is the `play_file` example of the audio crate (crates/audio/examples/play_file.rs),
# which drives `AudioEngine` as the app does. <track-seconds> is the track's length.
#
# 1. A private sound server: its own XDG_RUNTIME_DIR, HOME and PULSE_SERVER, so a desktop's
#    own server (or WSLg's) is never touched. PipeWire runs with WirePlumber and
#    pipewire-pulse; PulseAudio runs with no default script. Either way a null sink,
#    `rig_sink`, is loaded through the PulseAudio protocol and made the default.
# 2. ALSA's default device goes to that server (~/.asoundrc in the private HOME): through
#    pipewire-alsa for PipeWire, as Ubuntu's desktop does, or through the pulse plugin from
#    libasound2-plugins for PulseAudio. cpal, under the engine, opens ALSA's default.
# 3. `parec` records the sink's monitor from a second before the track to a second and a half
#    after it; PipeWire's `pw-top` runs alongside for the xrun counts.
# 4. analyze.py checks the recording and the logs (thresholds there) and writes report.txt,
#    report.json and excerpt.wav to <out-dir>, with the servers' and player's logs.
#
# Run it inside `dbus-run-session` (WirePlumber wants a session bus). Optional environment:
#   RIG_SECONDS=N         play N seconds and pause, instead of the whole track
#   RIG_TMP=DIR           where the private runtime dir goes (default $RUNNER_TEMP or /tmp)
#   RIG_PULSEAUDIO=CMD    the PulseAudio daemon to start (default `pulseaudio`)
set -euo pipefail

if [ $# -ne 5 ]; then
    sed -n '5p' "$0" | sed 's/^# *//' >&2
    exit 2
fi
server=$1
player=$(realpath "$2")
track=$(realpath "$3")
out=$4
track_seconds=$5
here=$(cd "$(dirname "$0")" && pwd)
case "$server" in pipewire | pulseaudio) ;; *) echo "server: pipewire or pulseaudio" >&2; exit 2 ;; esac

mkdir -p "$out"
out=$(realpath "$out")
rig=$(mktemp -d "${RIG_TMP:-${RUNNER_TEMP:-/tmp}}/rig-sound.XXXXXX")
mkdir -p -m 700 "$rig/run"
mkdir -p "$rig/home" "$rig/tmp"
export XDG_RUNTIME_DIR="$rig/run"
export HOME="$rig/home"
export PULSE_SERVER="unix:$XDG_RUNTIME_DIR/pulse/native"
export PULSE_RUNTIME_PATH="$XDG_RUNTIME_DIR/pulse"
unset PIPEWIRE_REMOTE PULSE_SINK PULSE_SOURCE ALSA_CONFIG_PATH
# PipeWire and WirePlumber keep their state under these.
export XDG_CONFIG_HOME="$HOME/.config" XDG_STATE_HOME="$HOME/.local/state"

pids=()
cleanup() {
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
    wait 2>/dev/null || true
    rm -rf "$rig"
}
trap cleanup EXIT

say() { printf '[rig %s] %s\n' "$(date +%T)" "$*"; }
trap 'say "failed at line $LINENO: $BASH_COMMAND"' ERR

# Waits up to 10 s for `$@` to succeed.
wait_for() {
    for _ in $(seq 100); do
        if "$@" >/dev/null 2>&1; then return 0; fi
        sleep 0.1
    done
    say "timed out waiting for: $*"
    return 1
}

say "server: $server; runtime dir $XDG_RUNTIME_DIR"
if [ "$server" = pipewire ]; then
    pipewire >"$out/pipewire.log" 2>&1 &
    pids+=($!)
    wait_for test -S "$XDG_RUNTIME_DIR/pipewire-0"
    wireplumber >"$out/wireplumber.log" 2>&1 &
    pids+=($!)
    pipewire-pulse >"$out/pipewire-pulse.log" 2>&1 &
    pids+=($!)
    cat >"$HOME/.asoundrc" <<'ALSA'
pcm.!default pipewire
ctl.!default pipewire
ALSA
else
    # No default script (-n): only the protocol socket, then the null sink below. Debug
    # logging, for the stream underrun lines it prints (counted in the report, not checked).
    ${RIG_PULSEAUDIO:-pulseaudio} -n --daemonize=no --exit-idle-time=-1 --use-pid-file=no \
        --realtime=no --high-priority=no --log-target=stderr --log-level=debug --log-time=yes \
        -L "module-native-protocol-unix auth-anonymous=1 socket=$XDG_RUNTIME_DIR/pulse/native" \
        2>"$out/pulseaudio.log" &
    pids+=($!)
    cat >"$HOME/.asoundrc" <<'ALSA'
pcm.!default pulse
ctl.!default pulse
ALSA
fi
wait_for pactl info
pactl load-module module-null-sink sink_name=rig_sink rate=48000 channels=2 \
    sink_properties=device.description=rig-null-sink >/dev/null
# Retried: under PipeWire it says "Not supported" until WirePlumber has made the default
# metadata, a moment after pipewire-pulse answers.
wait_for pactl set-default-sink rig_sink
wait_for sh -c 'pactl get-default-sink | grep -qx rig_sink'
{
    pactl info
    echo
    pactl list short sinks
    echo
    aplay -L 2>/dev/null | head -20 || true
} >"$out/server-info.txt"
say "default sink: $(pactl get-default-sink)"

# The recording: raw, so a killed recorder can't leave a WAV with a wrong header.
parec -d rig_sink.monitor --raw --format=s16le --rate=48000 --channels=2 --latency-msec=20 \
    >"$rig/rec.raw" 2>"$out/parec.log" &
rec=$!
pids+=("$rec")
if [ "$server" = pipewire ] && command -v pw-top >/dev/null; then
    pw-top --batch-mode >"$out/pw-top.txt" 2>&1 &
    top=$!
    pids+=("$top")
fi
sleep 1

say "playing $(basename "$track")${RIG_SECONDS:+ for ${RIG_SECONDS} s}"
args=("$track" --volume 1 --tmp "$rig/tmp")
if [ -n "${RIG_SECONDS:-}" ]; then args+=(--seconds "$RIG_SECONDS"); fi
set +e
timeout 300 "$player" "${args[@]}" >"$out/player.log" 2>&1
code=$?
set -e
say "player exited with $code"
sleep 1.5
if [ -n "${top:-}" ]; then kill -INT "$top" 2>/dev/null || true; fi
kill -INT "$rec" 2>/dev/null || true
wait "$rec" 2>/dev/null || true
if [ -n "${top:-}" ]; then wait "$top" 2>/dev/null || true; fi
pactl list sink-inputs >"$out/sink-inputs-after.txt" 2>&1 || true

python3 -I "$here/analyze.py" --server "$server" --raw "$rig/rec.raw" --rate 48000 \
    --player-log "$out/player.log" --player-exit "$code" --track-seconds "$track_seconds" \
    ${RIG_SECONDS:+--seconds "$RIG_SECONDS"} --out "$out"
