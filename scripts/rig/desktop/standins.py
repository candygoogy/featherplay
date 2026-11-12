#!/usr/bin/env python3
"""Stand-ins for the desktop's services on the rig's session bus, recording every call.

- org.freedesktop.Notifications: a notification server (GNOME Shell, KDE Plasma, dunst
  and the rest are one). Each Notify is answered with an ID (the same one when it replaces
  an earlier notification) and written down.
- org.kde.StatusNotifierWatcher: the tray's watcher (KDE Plasma's, or GNOME's AppIndicator
  extension's). Items that register are written down and listed in
  RegisteredStatusNotifierItems; a host is said to be there.

Usage: standins.py <calls.jsonl>. Each call is one JSON line: {"call": ..., ...}.
Runs until killed. Needs python3-dbus and python3-gi.
"""

import json
import sys
import time

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

NOTIFICATIONS = "org.freedesktop.Notifications"
WATCHER = "org.kde.StatusNotifierWatcher"

DBusGMainLoop(set_as_default=True)
out = open(sys.argv[1], "a", buffering=1, encoding="utf-8")


def record(call, **fields):
    fields["call"] = call
    fields["at"] = round(time.time(), 3)
    out.write(json.dumps(fields, ensure_ascii=False) + "\n")


def plain(value):
    """A D-Bus value as plain JSON."""
    if isinstance(value, (dbus.Boolean, bool)):
        return bool(value)
    if isinstance(value, (dbus.String, dbus.ObjectPath, str)):
        return str(value)
    if isinstance(value, (int, float)):
        return value
    if isinstance(value, dict):
        return {str(k): plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [plain(v) for v in value]
    return str(value)


class Notifications(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, "/org/freedesktop/Notifications")
        self.next_id = 1

    @dbus.service.method(NOTIFICATIONS, in_signature="", out_signature="as")
    def GetCapabilities(self):
        record("GetCapabilities")
        return ["body", "body-markup"]

    @dbus.service.method(NOTIFICATIONS, in_signature="", out_signature="ssss")
    def GetServerInformation(self):
        return ("rig-notifications", "featherplay rig", "1", "1.2")

    @dbus.service.method(
        NOTIFICATIONS,
        in_signature="susssasa{sv}i",
        out_signature="u",
        sender_keyword="sender",
    )
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout, sender=None):
        if replaces:
            nid = int(replaces)
        else:
            nid = self.next_id
            self.next_id += 1
        record(
            "Notify",
            sender=str(sender),
            app=str(app),
            replaces=int(replaces),
            id=nid,
            summary=str(summary),
            body=str(body),
            hints=plain(dict(hints)),
            timeout=int(timeout),
        )
        return nid

    @dbus.service.method(NOTIFICATIONS, in_signature="u", out_signature="")
    def CloseNotification(self, nid):
        record("CloseNotification", id=int(nid))


class Watcher(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, "/StatusNotifierWatcher")
        self.items = []

    @dbus.service.method(WATCHER, in_signature="s", out_signature="", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        # The spec allows a bus name or an object path; a path means the sender's own name.
        name = str(sender) if str(service).startswith("/") else str(service)
        path = str(service) if str(service).startswith("/") else "/StatusNotifierItem"
        entry = name + path if str(service).startswith("/") else name
        if entry not in self.items:
            self.items.append(entry)
        record("RegisterStatusNotifierItem", service=str(service), sender=str(sender), item=entry)
        self.StatusNotifierItemRegistered(entry)

    @dbus.service.method(WATCHER, in_signature="s", out_signature="")
    def RegisterStatusNotifierHost(self, service):
        record("RegisterStatusNotifierHost", service=str(service))

    @dbus.service.signal(WATCHER, signature="s")
    def StatusNotifierItemRegistered(self, service):
        pass

    @dbus.service.signal(WATCHER, signature="s")
    def StatusNotifierItemUnregistered(self, service):
        pass

    @dbus.service.signal(WATCHER, signature="")
    def StatusNotifierHostRegistered(self):
        pass

    def props(self):
        return {
            "RegisteredStatusNotifierItems": dbus.Array(self.items, signature="s"),
            "IsStatusNotifierHostRegistered": dbus.Boolean(True),
            "ProtocolVersion": dbus.Int32(0),
        }

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, interface, prop):
        return self.props()[str(prop)]

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return self.props()

    def forget(self, name):
        """An item's owner left the bus: it's unregistered, as a real watcher does."""
        gone = [i for i in self.items if i == name or i.startswith(name + "/")]
        for item in gone:
            self.items.remove(item)
            record("StatusNotifierItemUnregistered", item=item)
            self.StatusNotifierItemUnregistered(item)


bus = dbus.SessionBus()
notifications = Notifications(bus)
watcher = Watcher(bus)


def owner_changed(name, old, new):
    if old and not new:
        watcher.forget(str(name))


bus.add_signal_receiver(
    owner_changed,
    signal_name="NameOwnerChanged",
    dbus_interface="org.freedesktop.DBus",
    path="/org/freedesktop/DBus",
)
# Held for the run: the names are asked for once the objects answer.
names = [dbus.service.BusName(n, bus, do_not_queue=True) for n in (NOTIFICATIONS, WATCHER)]
record("up", names=[NOTIFICATIONS, WATCHER])
GLib.MainLoop().run()
