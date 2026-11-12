#!/usr/bin/env bash
# The AppImage rig's tests, run as an unprivileged user inside a distribution's container
# (container.sh), or directly on a Linux machine. Each run gets a fresh $HOME, so the app
# uses its default XDG folders. Every AppImage starts with --appimage-extract-and-run (no
# FUSE) and --timing (`first_frame_ms=<n>` on stderr once the window has drawn).
#
#   start    the release AppImage, signed out and offline: the first frame, then SIGTERM;
#            exit code 0 and the close line in its log.
#   play     the test AppImage (`perf-harness` and `update-test-server` features) on the
#            recorded fixtures: plays a song (every song plays the bundled 10 s tone) to a
#            null sink of a PulseAudio started here (or, where a sound server already runs,
#            a null sink added to it); the harness must report it audible and the sink an
#            input; then SIGTERM, exit code 0, "session saved" in the log. Skipped, and said
#            so, when no sound server can run.
#   update   the older test AppImage in ~/Applications, pointed at release.sh's server on
#            127.0.0.1: it downloads and checks the newer one, "Restart to update" is pressed
#            (update.txt), and it must quit with 0, the file must be the newer AppImage (its
#            sha256, its mode, executable), and the newer version must start from it; then
#            SIGTERM.
#
# Results: one PASS/FAIL/SKIP line per test in $OUT/results.txt, and every log in $OUT. The
# exit code is 1 if anything failed.
#
# Layout under $RIG (default /rig): release/featherplay-<arch>.AppImage,
# test/{old,new}/featherplay-<arch>.AppImage, test/versions.txt ("<old> <new>"),
# server/ and keys/ (release.sh), scripts/ (this folder), tone-10s.m4a.
# Set RIG_TESTS to run only some ("start play update"); container.sh sets RIG_OFFLINE=1.
set -uo pipefail

rig=${RIG:-/rig}
out=${OUT:-$rig/out}
tests=${RIG_TESTS:-start play update}
arch=$(uname -m)
name=featherplay-$arch.AppImage
port=${RIG_PORT:-8765}
mkdir -p "$out"
work=$(mktemp -d "${TMPDIR:-/tmp}/fp-rig.XXXXXX")
results=$out/results.txt
: >"$results"
failed=0

say() { echo "run.sh: $*"; }
result() {
    echo "$1 $2: $3" | tee -a "$results"
    if [ "$1" = FAIL ]; then failed=1; fi
}

# ----- The display and the sound server --------------------------------------------------

# X11 only, as Xvfb gives (on a desktop, the WAYLAND_DISPLAY it has is left out too).
unset WAYLAND_DISPLAY
xvfb=
if [ -z "${DISPLAY:-}" ]; then
    Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >"$out/xvfb.log" 2>&1 &
    xvfb=$!
    export DISPLAY=:99
    for _ in $(seq 100); do [ -S /tmp/.X11-unix/X99 ] && break; sleep 0.1; done
    [ -S /tmp/.X11-unix/X99 ] || say "Xvfb didn't start (xvfb.log)"
fi

export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$work/runtime}
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
pulse=
null_module=
sound=
# pactl, given up on after 10 s (a busy server can leave it waiting).
pa() { timeout 10 pactl "$@"; }
if command -v pactl >/dev/null && pa info >/dev/null 2>&1; then
    # A server runs already (a desktop, WSLg): a null sink of our own on it, for this run.
    null_module=$(pa load-module module-null-sink sink_name=fp_rig_appimage 2>/dev/null) &&
        export PULSE_SINK=fp_rig_appimage && sound="a null sink on the running sound server"
elif command -v pulseaudio >/dev/null; then
    pulseaudio -n --daemonize=no --exit-idle-time=-1 --log-target=stderr \
        --load="module-native-protocol-unix auth-anonymous=1" \
        --load="module-null-sink sink_name=fp_rig_appimage" >"$out/pulseaudio.log" 2>&1 &
    pulse=$!
    for _ in $(seq 50); do pa info >/dev/null 2>&1 && break; sleep 0.2; done
    if pa info >/dev/null 2>&1; then
        export PULSE_SINK=fp_rig_appimage
        sound="PulseAudio $(pulseaudio --version | sed 's/[^0-9.]//g') with a null sink"
    else
        say "PulseAudio didn't start (pulseaudio.log)"
    fi
fi
say "display $DISPLAY; sound: ${sound:-none}"

cleanup() {
    pkill -TERM -u "$(id -u)" -f '/appimage_extracted_[0-9a-f]*/AppRun' 2>/dev/null
    [ -n "${server_pid:-}" ] && kill "$server_pid" 2>/dev/null
    [ -n "$null_module" ] && pa unload-module "$null_module" 2>/dev/null
    [ -n "$pulse" ] && kill "$pulse" 2>/dev/null
    [ -n "$xvfb" ] && kill "$xvfb" 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT

# ----- Running the AppImage -----------------------------------------------------------

# A fresh $HOME for test $1, with ALSA's default set to the sound server (a desktop
# distribution configures this system-wide; the rig's own HOME has nothing).
fresh_home() {
    local h=$work/home-$1
    rm -rf "$h"
    mkdir -p "$h"
    printf 'pcm.!default { type pulse }\nctl.!default { type pulse }\n' >"$h/.asoundrc"
    echo "$h"
}

# The log of the run whose home is $1.
log_of() { find "$1" -name featherplay.log -print -quit 2>/dev/null; }

# start <test> <home> <AppImage> [VAR=value...]: starts it in the background; sets RT (the
# AppImage runtime's pid, whose exit code is the app's) and APP (the program's pid).
start() {
    local t=$1 h=$2 img=$3
    shift 3
    # Nothing of an earlier run's.
    rm -f "$out/$t".* "$out/$t"-*
    env HOME="$h" "$@" "$img" --appimage-extract-and-run --timing \
        >"$out/$t.stdout" 2>"$out/$t.stderr" &
    RT=$!
    APP=
    for _ in $(seq 100); do
        APP=$(pgrep -P "$RT" | head -1)
        [ -n "$APP" ] && break
        kill -0 "$RT" 2>/dev/null || break
        sleep 0.1
    done
    # A runtime that runs the program in its own process (no child): the same pid.
    [ -n "$APP" ] || APP=$RT
}

# first_frame <test>: waits up to 60 s for `first_frame_ms=`; prints it.
first_frame() {
    local f=$out/$1.stderr
    for _ in $(seq 600); do
        if grep -q '^first_frame_ms=' "$f"; then
            sed -n 's/^first_frame_ms=//p' "$f" | head -1
            return 0
        fi
        kill -0 "$RT" 2>/dev/null || return 1
        sleep 0.1
    done
    return 1
}

# stop: SIGTERM to the program, then the runtime's exit code (124 if it took over 30 s).
stop() {
    kill -TERM "$APP" 2>/dev/null
    for _ in $(seq 300); do
        kill -0 "$RT" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$RT" 2>/dev/null; then
        kill -KILL "$APP" "$RT" 2>/dev/null
        wait "$RT" 2>/dev/null
        return 124
    fi
    wait "$RT"
}

# keep <test> <home>: copies the run's log into $OUT.
keep() {
    local log
    log=$(log_of "$2")
    [ -n "$log" ] && cp "$log" "$out/$1.log"
}

closed='the system asked featherplay to close'

# ----- start ---------------------------------------------------------------------------

test_start() {
    local img=$rig/release/$name h ms code
    [ -f "$img" ] || { result FAIL start "no $img"; return; }
    h=$(fresh_home start)
    start start "$h" "$img"
    if ! ms=$(first_frame start); then
        stop
        keep start "$h"
        result FAIL start "no first frame within 60 s (start.stderr, start.log)"
        return
    fi
    sleep 2
    stop
    code=$?
    keep start "$h"
    if [ "$code" != 0 ]; then
        result FAIL start "first frame in $ms ms, but exit code $code after SIGTERM"
    elif ! grep -q "$closed" "$out/start.log" 2>/dev/null; then
        result FAIL start "exit code 0, but no close line in the log"
    else
        result PASS start "first frame in $ms ms, signed out${RIG_OFFLINE:+ with no network}; SIGTERM: exit 0"
    fi
}

# ----- play ----------------------------------------------------------------------------

test_play() {
    local img=$rig/test/old/$name h ms code line inputs
    [ -n "$sound" ] || { result SKIP play "no sound server could run here"; return; }
    [ -f "$img" ] || { result FAIL play "no $img"; return; }
    h=$(fresh_home play)
    start play "$h" "$img" FEATHERPLAY_PERF_SCRIPT="$rig/scripts/play.txt" \
        FEATHERPLAY_PERF_FIXTURES=1 FEATHERPLAY_PERF_AUDIO="$rig/tone-10s.m4a" \
        FEATHERPLAY_PERF_OUT="$out/play-perf.jsonl"
    if ! ms=$(first_frame play); then
        stop
        keep play "$h"
        result FAIL play "no first frame within 60 s"
        return
    fi
    # The harness's play-row line: written once the song is audible, or after its timeout.
    line=
    for _ in $(seq 900); do
        line=$(grep '"step":"play-row"' "$out/play-perf.jsonl" 2>/dev/null | head -1)
        [ -n "$line" ] && break
        kill -0 "$RT" 2>/dev/null || break
        sleep 0.1
    done
    sleep 3
    inputs=$(pa list short sink-inputs 2>/dev/null | wc -l)
    pa list sink-inputs >"$out/play-sink-inputs.txt" 2>&1
    stop
    code=$?
    keep play "$h"
    if [ -z "$line" ]; then
        result FAIL play "first frame in $ms ms, but the harness never played (play-perf.jsonl)"
    elif grep -q '"timeout":true\|"error"' <<<"$line" || ! grep -q '"audible":[0-9]' <<<"$line"; then
        result FAIL play "the song never became audible: $line"
    elif [ "$inputs" -lt 1 ]; then
        result FAIL play "audible to the harness, but the sink had no input"
    elif [ "$code" != 0 ]; then
        result FAIL play "played, but exit code $code after SIGTERM"
    elif ! grep -q 'session saved' "$out/play.log"; then
        result FAIL play "exit code 0, but no \"session saved\" in the log"
    else
        result PASS play "audible after $(sed 's/.*"audible":\([0-9.]*\).*/\1/' <<<"$line") ms on $sound; SIGTERM: exit 0, session saved"
    fi
}

# ----- update --------------------------------------------------------------------------

test_update() {
    local old=$rig/test/old/$name new=$rig/test/new/$name h target mode_before code
    local versions new_version log want got mode_after newpid
    for f in "$old" "$new" "$rig/server/latest" "$rig/keys/test.pub"; do
        [ -f "$f" ] || { result FAIL update "no $f"; return; }
    done
    versions=$(cat "$rig/test/versions.txt")
    new_version=${versions#* }
    h=$(fresh_home update)
    mkdir -p "$h/Applications"
    target=$h/Applications/$name
    install -m 755 "$old" "$target"
    mode_before=$(stat -c %a "$target")
    python3 -m http.server "$port" --bind 127.0.0.1 --directory "$rig/server" \
        >"$out/http-server.log" 2>&1 &
    server_pid=$!
    for _ in $(seq 50); do
        python3 -c "import urllib.request as u; u.urlopen('http://127.0.0.1:$port/latest')" \
            2>/dev/null && break
        sleep 0.2
    done
    start update "$h" "$target" FEATHERPLAY_UPDATE_SERVER="http://127.0.0.1:$port" \
        FEATHERPLAY_UPDATE_KEY="$rig/keys/test.pub" \
        FEATHERPLAY_PERF_SCRIPT="$rig/scripts/update.txt" FEATHERPLAY_PERF_FIXTURES=1 \
        FEATHERPLAY_PERF_OUT="$out/update-perf.jsonl"
    local oldapp=$APP
    if ! first_frame update >/dev/null; then
        stop
        keep update "$h"
        result FAIL update "no first frame within 60 s"
        return
    fi
    # It quits by itself once "Restart to update" is pressed.
    for _ in $(seq 1800); do
        kill -0 "$RT" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$RT" 2>/dev/null; then
        stop
        keep update "$h"
        result FAIL update "still running after 3 minutes: no update (update.log, http-server.log)"
        return
    fi
    wait "$RT"
    code=$?
    log=$(log_of "$h")
    # The newer version, started from the replaced file by the restart.
    for _ in $(seq 600); do
        grep -q "featherplay $new_version on" "$log" 2>/dev/null && break
        sleep 0.1
    done
    sleep 2
    newpid=$(pgrep -u "$(id -u)" -f '/appimage_extracted_[0-9a-f]*/AppRun' | grep -vx "$oldapp" | head -1)
    want=$(sha256sum <"$new" | cut -d' ' -f1)
    got=$(sha256sum <"$target" | cut -d' ' -f1)
    mode_after=$(stat -c %a "$target")
    if [ -n "$newpid" ]; then
        readlink "/proc/$newpid/exe" >"$out/update-restarted-exe.txt" 2>&1
        kill -TERM "$newpid"
        for _ in $(seq 300); do kill -0 "$newpid" 2>/dev/null || break; sleep 0.1; done
    fi
    sleep 0.5
    keep update "$h"
    local why=
    [ "$code" = 0 ] || why="$why; the old copy exited with $code"
    grep -q "update: $new_version downloaded and checked" "$out/update.log" ||
        why="$why; no \"downloaded and checked\""
    grep -q "update: the setup is running; quitting" "$out/update.log" ||
        why="$why; no \"the setup is running; quitting\""
    [ "$want" = "$got" ] || why="$why; the file isn't the newer AppImage"
    [ -x "$target" ] || why="$why; the file isn't executable"
    [ "$mode_after" = "$mode_before" ] || why="$why; mode $mode_before became $mode_after"
    grep -q "featherplay $new_version on" "$out/update.log" ||
        why="$why; $new_version never started"
    grep -q "\"version\":\"$new_version\"" "$out/update-perf.jsonl" 2>/dev/null ||
        why="$why; the harness never ran in $new_version"
    [ -n "$newpid" ] || why="$why; no restarted process"
    [ "$(grep -c "$closed" "$out/update.log")" -ge 1 ] || why="$why; the new copy didn't close on SIGTERM"
    if [ -n "$why" ]; then
        result FAIL update "${versions% *} -> $new_version${why}"
    else
        result PASS update "${versions% *} -> $new_version: downloaded, verified (test key), replaced in place (mode $mode_after), restarted into $new_version; SIGTERM closed it"
    fi
}

for t in $tests; do
    say "test $t"
    "test_$t"
done
cat "$results"
exit $failed
