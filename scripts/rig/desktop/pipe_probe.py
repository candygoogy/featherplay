#!/usr/bin/env python3
"""A look at a browser that didn't answer the sign-in's pipe, for live.sh: started as the
sign-in starts it (a new profile, --remote-debugging-pipe on descriptors 3 and 4, the
switches of QUIET_START in crates/platform/src/common/browsers.rs), on about:blank, with
its log on stderr kept this time. Asks Storage.getCookies and waits up to <timeout> s, so a
slow first start shows apart from one that never answers; then Browser.close.

  pipe_probe.py <exe> <folder> [timeout]

Writes <folder>/stderr.log (the browser's own log); prints when the reply came, if it did.
Exits 0 either way: it only reports.
"""

import fcntl
import json
import os
import select
import shutil
import signal
import subprocess
import sys
import time

exe, folder = sys.argv[1], sys.argv[2]
timeout = float(sys.argv[3]) if len(sys.argv) > 3 else 60
profile = os.path.join(folder, "profile")
os.makedirs(profile, exist_ok=True)
switches = ["--no-first-run", "--no-default-browser-check", "--password-store=basic",
            "--disable-blink-features=AutomationControlled", "--test-type",
            "--enable-logging=stderr", "--v=0"]

# The browser's ends of the two pipes, above 4 so the shell can place them on 3 and 4.
to_read, to_write = os.pipe()
from_read, from_write = os.pipe()
theirs = [fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 20) for fd in (to_read, from_write)]
os.close(to_read)
os.close(from_write)
place = f'exec 3<&{theirs[0]} 4>&{theirs[1]} {theirs[0]}<&- {theirs[1]}>&-; exec "$@"'
args = [exe, f"--user-data-dir={profile}", "--remote-debugging-pipe", *switches, "about:blank"]
log = open(os.path.join(folder, "stderr.log"), "wb")
start = time.monotonic()
p = subprocess.Popen(["bash", "-c", place, "probe", *args], stdin=subprocess.DEVNULL,
                     stdout=log, stderr=log, pass_fds=theirs, start_new_session=True)
for fd in theirs:
    os.close(fd)


def send(id_, method):
    try:
        os.write(to_write, json.dumps({"id": id_, "method": method}).encode() + b"\0")
        return True
    except BrokenPipeError:
        return False


reply = None
buf = b""
if send(1, "Storage.getCookies"):
    while time.monotonic() - start < timeout and reply is None:
        if p.poll() is not None:
            print(f"probe: {exe} exited ({p.returncode}) after {time.monotonic() - start:.1f} s")
            break
        ready, _, _ = select.select([from_read], [], [], 0.5)
        if not ready:
            continue
        chunk = os.read(from_read, 1 << 16)
        if not chunk:
            print(f"probe: {exe} closed the pipe after {time.monotonic() - start:.1f} s")
            break
        buf += chunk
        while b"\0" in buf and reply is None:
            message, buf = buf.split(b"\0", 1)
            if json.loads(message).get("id") == 1:
                reply = message
if reply is not None:
    print(f"probe: {exe} answered after {time.monotonic() - start:.1f} s: {reply[:120].decode()}")
    if send(2, "Browser.close"):
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            pass
elif p.poll() is None:
    print(f"probe: no answer from {exe} within {timeout:.0f} s")
if p.poll() is None:
    os.killpg(p.pid, signal.SIGKILL)
    p.wait()
log.close()
shutil.rmtree(profile, ignore_errors=True)
