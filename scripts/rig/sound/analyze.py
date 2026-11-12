"""Checks the sound rig's recording and logs (run.sh calls it), and writes the report.

The recording is the null sink's monitor, raw s16le stereo, from a second before the player
started to a second and a half after it stopped (PulseAudio's monitor delivers nothing while
its sink is idle, so there the recording starts with the stream). Checks, each with its
threshold here:

- sound: the RMS level from the first to the last sounding 10 ms block is at least
  MIN_RMS_DBFS. The track plays at full volume, so a good run is far above it (music sits
  around -20 dBFS); a broken route records digital zeros.
- length: the sounding span (first to last block above SOUND_DBFS) is the track's length
  within LENGTH_TOLERANCE_S. Too short: audio was lost or cut; too long: it stalled or
  repeated. The time from the engine's `Started` to its `Ended` is the track's length within
  PLAYED_TOLERANCE_S, which is wider: with PulseAudio's null sink the output stalls for
  ~0.9 s right after it starts (0.84-0.89 s in WSL), sometimes after `Started`, and no sound
  is lost to it. With `--seconds` (play that long, then pause) the span is checked against
  the position the engine reached, and that position against the seconds asked for.
- dropouts: no run of near-zero samples (every sample within DROPOUT_LEVEL of zero) of
  DROPOUT_MIN_MS or more inside the sounding span. That is what an underrun puts on the sink:
  the server fills a period the client didn't deliver with silence. The music itself never
  goes digitally silent for that long between its first and last notes.
- engine: the engine's own glitch report (crates/audio/src/glitches.rs) says its ring never
  ran dry, and it sent no `Buffering(true)`. Its other counter, "late output callback(s)" with
  the longest gap between two callbacks, is reported but not checked. On ALSA a callback
  fills one period of a two-period buffer and the engine calls it late once the gap passes
  one period, so scheduling jitter counts. And the longest gap includes the stream's start:
  through the pulse plugin it was 864 ms in a run whose recording had no gap at all (the
  server was still prebuffering). The dropout check above is what hears a gap. The engine's
  first-sound line must be in the log, so a logger that prints nothing can't pass.
- PipeWire: the xrun counts (pw-top's ERR column) of the nodes in the graph while the track
  played are at most MAX_PW_XRUNS in total. PulseAudio's stream underrun lines are counted
  but not checked (it logs them at a stream's start and drain too).

Exit status 0 when every check passes, 1 otherwise.
"""

import argparse
import json
import math
import re
import sys
import wave
from array import array
from pathlib import Path

BLOCK_MS = 10
MIN_RMS_DBFS = -40.0
SOUND_DBFS = -60.0
LENGTH_TOLERANCE_S = 0.5
PLAYED_TOLERANCE_S = 2.0
DROPOUT_LEVEL = 2  # in s16 steps (of 32768)
DROPOUT_MIN_MS = 10
MAX_PW_XRUNS = 0
EXCERPT_S = 10


def dbfs(x):
    return 20 * math.log10(x) if x > 0 else -math.inf


def load(raw, channels):
    samples = array("h")
    data = raw.read_bytes()
    data = data[: len(data) - len(data) % (2 * channels)]
    samples.frombytes(data)
    if sys.byteorder != "little":
        samples.byteswap()
    return samples


def blocks(samples, rate, channels):
    """RMS of each BLOCK_MS block, over all channels, as a fraction of full scale."""
    n = rate * BLOCK_MS // 1000 * channels
    out = []
    for i in range(0, len(samples) - n + 1, n):
        chunk = samples[i : i + n]
        out.append(math.sqrt(sum(s * s for s in chunk) / n) / 32768)
    return out


def dropouts(samples, rate, channels, first, last):
    """Runs of near-zero frames of DROPOUT_MIN_MS or more between frames first and last."""
    need = rate * DROPOUT_MIN_MS // 1000
    found = []
    run_start = None
    for f in range(first, last):
        base = f * channels
        quiet = all(abs(samples[base + c]) <= DROPOUT_LEVEL for c in range(channels))
        if quiet:
            if run_start is None:
                run_start = f
        elif run_start is not None:
            if f - run_start >= need:
                found.append((run_start, f))
            run_start = None
    if run_start is not None and last - run_start >= need:
        found.append((run_start, last))
    return found


def player_facts(text):
    facts = dict(re.findall(r"^rig: (\w+)=(\S+)$", text, re.M))
    late = starved = 0
    gaps = []
    for m in re.finditer(
        r"audio: (\d+) late output callback\(s\), longest gap (\d+) ms; ring ran dry (\d+) time",
        text,
    ):
        late += int(m[1])
        gaps.append(int(m[2]))
        starved += int(m[3])
    return {
        "facts": facts,
        "late_callbacks": late,
        "longest_callback_gap_ms": max(gaps, default=0),
        "ring_ran_dry": starved,
        "glitch_reports": len(gaps),
        "first_sound_logged": bool(re.search(r"^log: .*audio: .*", text, re.M)),
    }


def pw_xruns(path):
    """ERR per node in the last pw-top iteration that still showed the player's stream."""
    if not path.exists():
        return None
    frames, cur = [], None
    for line in path.read_text(errors="replace").splitlines():
        cols = line.split()
        if not cols:
            continue
        if cols[0] == "S" and "ERR" in cols:
            cur = []
            frames.append(cur)
            err_at = cols.index("ERR")
            continue
        if cur is None or len(cols) <= err_at:
            continue
        try:
            err = int(cols[err_at])
        except ValueError:
            continue
        cur.append({"id": cols[1], "err": err, "name": cols[-1], "line": line.strip()})
    ours = [f for f in frames if any("play_file" in n["name"] or "alsa_playback" in n["name"] for n in f)]
    if not ours:
        return {"parsed": False, "iterations": len(frames)}
    last = ours[-1]
    return {"parsed": True, "iterations": len(frames), "total": sum(n["err"] for n in last), "nodes": last}


def pulse_underruns(path):
    if not path.exists():
        return None
    return sum(1 for line in path.read_text(errors="replace").splitlines() if "nderrun" in line)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", required=True)
    ap.add_argument("--raw", type=Path, required=True)
    ap.add_argument("--rate", type=int, default=48000)
    ap.add_argument("--channels", type=int, default=2)
    ap.add_argument("--player-log", type=Path, required=True)
    ap.add_argument("--player-exit", type=int, required=True)
    ap.add_argument("--track-seconds", type=float, required=True, help="the track's length")
    ap.add_argument("--seconds", type=float, default=None, help="played this long, then paused")
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    rate, ch = a.rate, a.channels

    samples = load(a.raw, ch)
    frames = len(samples) // ch
    levels = blocks(samples, rate, ch)
    block_frames = rate * BLOCK_MS // 1000
    threshold = 10 ** (SOUND_DBFS / 20)
    sounding = [i for i, v in enumerate(levels) if v > threshold]
    checks = []

    def check(name, ok, detail):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})

    report = {
        "server": a.server,
        "recorded_s": round(frames / rate, 3),
        "track_s": a.track_seconds,
        "seconds": a.seconds,
        "thresholds": {
            "min_rms_dbfs": MIN_RMS_DBFS,
            "sound_dbfs": SOUND_DBFS,
            "length_tolerance_s": LENGTH_TOLERANCE_S,
            "played_tolerance_s": PLAYED_TOLERANCE_S,
            "dropout_level": DROPOUT_LEVEL,
            "dropout_min_ms": DROPOUT_MIN_MS,
            "max_pw_xruns": MAX_PW_XRUNS,
        },
    }

    player_text = a.player_log.read_text(errors="replace") if a.player_log.exists() else ""
    p = player_facts(player_text)
    report["player"] = p
    check(
        "player",
        a.player_exit == 0 and p["facts"].get("result") == "ok",
        f"exit {a.player_exit}, result {p['facts'].get('result')}",
    )

    facts = p["facts"]
    whole = a.seconds is None
    position = int(facts["position_ms"]) / 1000 if "position_ms" in facts else None
    # What the sink should have received: the whole track, or as far as the engine got.
    expect = a.track_seconds if whole else position
    if sounding:
        first = sounding[0] * block_frames
        last = (sounding[-1] + 1) * block_frames
        span = (last - first) / rate
        body = samples[first * ch : last * ch]
        rms = math.sqrt(sum(s * s for s in body) / len(body)) / 32768
        peak = max(abs(s) for s in body) / 32768
        report.update(
            first_sound_s=round(first / rate, 3),
            last_sound_s=round(last / rate, 3),
            sounding_span_s=round(span, 3),
            rms_dbfs=round(dbfs(rms), 1),
            peak_dbfs=round(dbfs(peak), 1),
        )
        check("sound", dbfs(rms) >= MIN_RMS_DBFS, f"RMS {dbfs(rms):.1f} dBFS (min {MIN_RMS_DBFS})")
        check(
            "length",
            expect is not None and abs(span - expect) <= LENGTH_TOLERANCE_S,
            f"sounding span {span:.2f} s, expected {expect} +/- {LENGTH_TOLERANCE_S}"
            + (" (the track)" if whole else " (the engine's position at the pause)"),
        )
        gaps = dropouts(samples, rate, ch, first, last)
        report["dropouts"] = [
            {"at_s": round(s / rate, 3), "ms": round((e - s) * 1000 / rate, 1)} for s, e in gaps
        ]
        check(
            "dropouts",
            not gaps,
            f"{len(gaps)} run(s) of silence >= {DROPOUT_MIN_MS} ms inside the sound"
            + (f", first at {gaps[0][0] / rate:.2f} s" if gaps else ""),
        )
        # 10 s from the first sound, for listening to.
        ex = samples[first * ch : (first + EXCERPT_S * rate) * ch]
        if sys.byteorder != "little":
            ex = array("h", ex)
            ex.byteswap()
        with wave.open(str(a.out / "excerpt.wav"), "wb") as w:
            w.setnchannels(ch)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(ex.tobytes())
    else:
        check("sound", False, f"nothing above {SOUND_DBFS} dBFS in {frames / rate:.1f} s recorded")

    if whole:
        check("ended", facts.get("ended") == "true", f"Ended event: {facts.get('ended')}")
        played = int(facts["played_ms"]) / 1000 if "played_ms" in facts else None
        check(
            "played",
            played is not None and abs(played - a.track_seconds) <= PLAYED_TOLERANCE_S,
            f"Started to Ended {played} s, expected {a.track_seconds} +/- {PLAYED_TOLERANCE_S}",
        )
    else:
        check(
            "played",
            position is not None and abs(position - a.seconds) <= PLAYED_TOLERANCE_S,
            f"position at the pause {position} s, expected {a.seconds} +/- {PLAYED_TOLERANCE_S}",
        )
    check("logger", p["first_sound_logged"], "the engine's first-sound line is in the player's log")
    check("ring", p["ring_ran_dry"] == 0, f"ring ran dry {p['ring_ran_dry']} time(s)")
    report["late_callbacks_info"] = (
        f"{p['late_callbacks']} callback(s) past one period, longest gap "
        f"{p['longest_callback_gap_ms']} ms, in {p['glitch_reports']} report(s); not checked"
    )
    buffering = p["facts"].get("buffering")
    check("buffering", buffering == "0", f"Buffering(true) events: {buffering}")

    if a.server == "pipewire":
        x = pw_xruns(a.out / "pw-top.txt")
        report["pipewire_xruns"] = x
        if x is None or not x["parsed"]:
            check("xruns", True, f"pw-top output not usable ({x}); not checked")
        else:
            check("xruns", x["total"] <= MAX_PW_XRUNS, f"pw-top ERR total {x['total']} (max {MAX_PW_XRUNS})")
    else:
        report["pulseaudio_underrun_lines"] = pulse_underruns(a.out / "pulseaudio.log")

    report["checks"] = checks
    ok = all(c["ok"] for c in checks)
    report["ok"] = ok
    (a.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    lines = [f"sound rig, {a.server}: {'PASS' if ok else 'FAIL'}"]
    for k in ("recorded_s", "first_sound_s", "last_sound_s", "sounding_span_s", "rms_dbfs", "peak_dbfs"):
        if k in report:
            lines.append(f"  {k}: {report[k]}")
    for c in checks:
        lines.append(f"  [{'ok' if c['ok'] else 'FAIL'}] {c['check']}: {c['detail']}")
    lines.append(f"  (info) engine: {report['late_callbacks_info']}")
    if "pulseaudio_underrun_lines" in report:
        lines.append(f"  (info) PulseAudio log lines mentioning underruns: {report['pulseaudio_underrun_lines']}")
    text = "\n".join(lines) + "\n"
    (a.out / "report.txt").write_text(text)
    print(text, end="")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
