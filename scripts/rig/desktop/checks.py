#!/usr/bin/env python3
"""The rig's checks of a running featherplay, over the session bus, as a desktop sees it.

  checks.py wait-step <perf.jsonl> <step> [<arg>] [--timeout S]
      waits for the harness to record that step (the script's steps write one JSON line
      each when they're done; `wait` steps write none).
  checks.py mpris <pid> <standins.jsonl>
      MPRIS: the player's name, its metadata and status; Pause, Play and Next work; the
      song's start sent one notification, and so did the change Next made.
  checks.py tray <pid> <standins.jsonl>
      the tray item registered with the watcher, and its menu lists the entries.
  checks.py tray-quit <pid> <standins.jsonl>
      chooses Quit in the tray menu, as a click on it does.
  checks.py mpris-quit <pid>
      asks the player to quit over MPRIS.

Prints what it saw; exits 1 on the first check that fails. Needs python3-dbus.
"""

import json
import sys
import time

import dbus

PROPS = "org.freedesktop.DBus.Properties"
MPRIS_PATH = "/org/mpris/MediaPlayer2"
ROOT = "org.mpris.MediaPlayer2"
PLAYER = "org.mpris.MediaPlayer2.Player"
ITEM = "org.kde.StatusNotifierItem"
MENU = "com.canonical.dbusmenu"

# The tray menu's entries (crates/platform/src/linux/tray.rs), with a song playing; Like
# reads Liked for a liked song.
TRAY_ENTRIES = ["Pause", "Previous", "Next", ("Like", "Liked"), "Not for me", "Show featherplay", "Quit"]


def fail(msg):
    print(f"FAIL: {msg}", flush=True)
    sys.exit(1)


def ok(msg):
    print(f"ok: {msg}", flush=True)


def until(what, timeout, step=0.25):
    """Calls `what` until it returns something truthy; None after `timeout` seconds."""
    deadline = time.monotonic() + timeout
    while True:
        try:
            got = what()
        except dbus.DBusException:
            got = None
        if got:
            return got
        if time.monotonic() >= deadline:
            return None
        time.sleep(step)


def calls(path, name):
    with open(path, encoding="utf-8") as f:
        return [c for c in map(json.loads, f) if c.get("call") == name]


def bus_pid(bus, name):
    dbus_obj = bus.get_object("org.freedesktop.DBus", "/org/freedesktop/DBus")
    return int(dbus_obj.GetConnectionUnixProcessID(name, dbus_interface="org.freedesktop.DBus"))


def wait_step(args):
    path, step = args[0], args[1]
    arg = args[2] if len(args) > 2 and not args[2].startswith("--") else None
    timeout = float(args[args.index("--timeout") + 1]) if "--timeout" in args else 120

    def seen():
        try:
            with open(path, encoding="utf-8") as f:
                for line in f:
                    r = json.loads(line)
                    if r.get("step") == step and (arg is None or r.get("arg") == arg):
                        return r
        except (FileNotFoundError, json.JSONDecodeError):
            return None
        return None

    r = until(seen, timeout)
    if not r:
        fail(f"the harness didn't record `{step} {arg or ''}` within {timeout:.0f} s")
    print(json.dumps(r, ensure_ascii=False), flush=True)
    if r.get("error"):
        fail(f"`{step} {arg or ''}`: {r['error']}")
    if "--in-time" in args and r.get("timeout"):
        fail(f"`{step} {arg or ''}` timed out (a song step: it never became audible)")


def player(bus, pid):
    name = f"org.mpris.MediaPlayer2.featherplay.instance{pid}"
    obj = until(lambda: bus.get_object(name, MPRIS_PATH), 30)
    if not obj:
        fail(f"no {name} on the session bus")
    return name, obj


def mpris(args):
    pid, standins = int(args[0]), args[1]
    bus = dbus.SessionBus()
    name, obj = player(bus, pid)
    props = dbus.Interface(obj, PROPS)
    ok(f"{name} is on the bus")
    root = props.GetAll(ROOT)
    print(f"  Identity={root.get('Identity')} DesktopEntry={root.get('DesktopEntry')}")
    if not root.get("Identity"):
        fail("no Identity")

    def get(prop):
        return props.Get(PLAYER, prop)

    def title():
        return str(get("Metadata").get("xesam:title", ""))

    def status(want):
        return until(lambda: str(get("PlaybackStatus")) == want, 15)

    if not until(lambda: str(get("PlaybackStatus")) == "Playing" and title(), 40):
        fail(f"not playing: PlaybackStatus={get('PlaybackStatus')} title={title()!r}")
    meta = get("Metadata")
    first = title()
    shown = {str(k): [str(x) for x in v] if isinstance(v, dbus.Array) else str(v) for k, v in meta.items()}
    ok(f"metadata while playing: {json.dumps(shown, ensure_ascii=False)}")
    for key in ("mpris:trackid", "xesam:title", "xesam:artist", "mpris:length"):
        if key not in meta:
            fail(f"metadata has no {key}")
    owner = str(bus.get_name_owner(name))

    def notes():
        return [c for c in calls(standins, "Notify") if c.get("sender") == owner]

    started = notes()
    if len(started) != 1:
        fail(f"{len(started)} notifications for the first song's start, not 1: {started}")
    if first not in started[0]["summary"] + started[0]["body"]:
        fail(f"the notification doesn't name {first!r}: {started[0]}")
    ok(f"one notification for the song's start: {started[0]['summary']!r} / {started[0]['body']!r}")

    iface = dbus.Interface(obj, PLAYER)
    iface.Pause()
    if not status("Paused"):
        fail(f"Pause: PlaybackStatus is {get('PlaybackStatus')}")
    ok("Pause: Paused")
    iface.Play()
    if not status("Playing"):
        fail(f"Play: PlaybackStatus is {get('PlaybackStatus')}")
    ok("Play: Playing")
    iface.Next()
    second = until(lambda: title() != first and str(get("PlaybackStatus")) == "Playing" and title(), 40)
    if not second:
        fail(f"Next: still {title()!r}, {get('PlaybackStatus')}")
    ok(f"Next: {first!r} -> {second!r}")
    # The notification goes out as the song starts; give it a moment, then count.
    time.sleep(3)
    after = notes()
    if len(after) != 2:
        fail(f"{len(after) - 1} notifications for the change Next made, not 1: {after[1:]}")
    if second not in after[1]["summary"] + after[1]["body"]:
        fail(f"the second notification doesn't name {second!r}: {after[1]}")
    if after[1]["replaces"] != started[0]["id"]:
        fail(f"the second notification doesn't replace the first: {after[1]}")
    ok(f"one notification for the change, replacing the first: {after[1]['summary']!r}")


def tray_item(bus, pid, standins):
    def find():
        for c in reversed(calls(standins, "RegisterStatusNotifierItem")):
            try:
                if bus_pid(bus, c["sender"]) == pid:
                    return c
            except dbus.DBusException:
                continue
        return None

    reg = until(find, 30)
    if not reg:
        fail(f"no tray item from process {pid} registered with the watcher")
    item = reg["item"]
    name, _, path = item.partition("/")
    return name, "/" + path if path else "/StatusNotifierItem"


def labels_of(layout, out):
    _id, props, children = layout
    out.append(
        {
            "id": int(_id),
            "label": str(props.get("label", "")).replace("__", "\0").replace("_", "").replace("\0", "_"),
            "type": str(props.get("type", "standard")),
            "enabled": bool(props.get("enabled", True)),
        }
    )
    for child in children:
        labels_of(child, out)
    return out


def menu_entries(bus, name, path):
    item = bus.get_object(name, path)
    props = dbus.Interface(item, PROPS).GetAll(ITEM)
    menu_path = str(props.get("Menu", "/MenuBar"))
    menu = dbus.Interface(bus.get_object(name, menu_path), MENU)
    try:
        menu.AboutToShow(0)
    except dbus.DBusException:
        pass
    _rev, layout = menu.GetLayout(0, -1, dbus.Array([], signature="s"))
    return props, menu, labels_of(layout, [])[1:]


def tray(args):
    pid, standins = int(args[0]), args[1]
    bus = dbus.SessionBus()
    name, path = tray_item(bus, pid, standins)
    ok(f"tray item {name}{path} registered")
    props, _menu, entries = menu_entries(bus, name, path)
    print(
        f"  Id={props.get('Id')} Title={props.get('Title')} Status={props.get('Status')} "
        f"icons={len(props.get('IconPixmap', []))} Menu={props.get('Menu')}"
    )
    if str(props.get("Status")) not in ("Active", "Passive", "NeedsAttention"):
        fail(f"Status {props.get('Status')}")
    if not props.get("IconPixmap") and not props.get("IconName"):
        fail("the item has no icon")
    shown = [e["label"] for e in entries if e["type"] != "separator"]
    print(f"  menu: {json.dumps(shown, ensure_ascii=False)}")
    wanted = [e if isinstance(e, tuple) else (e,) for e in TRAY_ENTRIES]
    missing = [" or ".join(e) for e in wanted if not any(label in shown for label in e)]
    if missing:
        fail(f"the tray menu lacks {missing}")
    header = entries[0] if entries else {}
    if header.get("enabled", True) or not header.get("label"):
        fail(f"the menu's first entry isn't the song, greyed: {header}")
    ok(f"the tray menu lists the song ({header['label']!r}) and its {len(wanted)} entries")


def tray_quit(args):
    pid, standins = int(args[0]), args[1]
    bus = dbus.SessionBus()
    name, path = tray_item(bus, pid, standins)
    _props, menu, entries = menu_entries(bus, name, path)
    quit_ = next((e for e in entries if e["label"] == "Quit"), None)
    if not quit_:
        fail("no Quit in the tray menu")
    menu.Event(quit_["id"], "clicked", dbus.Int32(0, variant_level=1), dbus.UInt32(int(time.time())))
    ok(f"chose Quit (entry {quit_['id']}) in the tray menu")


def mpris_quit(args):
    pid = int(args[0])
    bus = dbus.SessionBus()
    _name, obj = player(bus, pid)
    dbus.Interface(obj, ROOT).Quit()
    ok("asked to quit over MPRIS")


COMMANDS = {
    "wait-step": wait_step,
    "mpris": mpris,
    "tray": tray,
    "tray-quit": tray_quit,
    "mpris-quit": mpris_quit,
}

if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
    print(__doc__)
    sys.exit(2)
COMMANDS[sys.argv[1]](sys.argv[2:])
