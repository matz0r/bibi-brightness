#!/usr/bin/env python3
"""Linux lifecycle regression test, isolated from the real desktop and home.
Run: dbus-run-session -- python3 tests/tray_startup.py target/debug/brightness
Requires Python's system gi (Gio) bindings.
"""
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import tempfile
import threading
import time
from gi.repository import Gio, GLib

BINARY = Path(sys.argv[1]).resolve()
INTERFACE = "org.kde.StatusNotifierWatcher"
XML = f"""<node><interface name='{INTERFACE}'>
<method name='RegisterStatusNotifierItem'><arg type='s' direction='in'/></method>
<property name='IsStatusNotifierHostRegistered' type='b' access='read'/>
</interface></node>"""
loop = GLib.MainLoop()
threading.Thread(target=loop.run, daemon=True).start()


def until(predicate, message):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(message)


class Watcher:
    def __init__(self):
        self.registrations = []
        self.connection = Gio.DBusConnection.new_for_address_sync(
            os.environ["DBUS_SESSION_BUS_ADDRESS"],
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
            | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
        info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
        self.connection.register_object(
            "/StatusNotifierWatcher", info, self.method,
            lambda *args: GLib.Variant("b", True), None)
        self.connection.call_sync(
            "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
            "RequestName", GLib.Variant("(su)", (INTERFACE, 0)), None,
            Gio.DBusCallFlags.NONE, 1000, None)

    def method(self, connection, sender, path, interface, method, args, invocation):
        self.registrations.append(args.unpack()[0])
        invocation.return_value(GLib.Variant("()", ()))

    def close(self):
        self.connection.close_sync(None)


with tempfile.TemporaryDirectory(prefix="bibi-lifecycle-") as folder:
    root = Path(folder)
    env = dict(os.environ, HOME=folder, XDG_RUNTIME_DIR=folder,
               XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
    socket_path = root / "bibi-brightness.sock"

    def request(message):
        try:
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(2)
                client.connect(str(socket_path))
                client.sendall(message.encode())
                client.shutdown(socket.SHUT_WR)
                data = b""
                while chunk := client.recv(4096):
                    data += chunk
                return data.decode()
        except (OSError, TimeoutError):
            return None

    watcher = None
    primary = subprocess.Popen([BINARY, "--tray", "--report-ready"], env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        assert select.select([primary.stdout], [], [], 8)[0], "startup acknowledgement timed out"
        assert primary.stdout.readline().strip() == "ready"
        until(lambda: request("status") == "offline", "must wait when the desktop tray is absent")
        assert primary.poll() is None, "app must survive starting before Cinnamon's tray"
        duplicates = [subprocess.Popen([BINARY, "--tray"], env=env) for _ in range(6)]
        assert all(process.wait(timeout=8) == 0 for process in duplicates)
        assert primary.poll() is None
        watcher = Watcher()
        until(lambda: request("status") == "online" and watcher.registrations,
              "tray must register when the desktop becomes ready")
        # A normal window close exits the child; it must leave the tray alive
        # and permit a later reopen. A tiny child avoids touching the real display.
        window = root / "window-child"
        marker = root / "window-opened"
        window.write_text(f"#!/bin/sh\nprintf x >> '{marker}'\n")
        window.chmod(0o700)
        for expected in ["x", "xx"]:
            assert request(f"open\n{window}") == "ok"
            until(lambda: marker.exists() and marker.read_text() == expected,
                  "window child must be reopenable after close")
            assert primary.poll() is None
            assert request("status") == "online"
        watcher.close()
        until(lambda: request("status") == "offline", "must detect a stopped desktop tray")
        watcher = Watcher()
        until(lambda: request("status") == "online" and watcher.registrations,
              "tray must recover after a desktop restart")
        assert request("replace") == "bye"
        assert primary.wait(timeout=5) == 0
        assert not socket_path.exists(), "shutdown must remove its socket"
        print("PASS: delayed tray, concurrent launches, window close/reopen, desktop restart, acknowledged shutdown")

        autostart = root / "config/autostart/brightness-tray.desktop"
        autostart.parent.mkdir(parents=True)
        autostart.write_text('[Desktop Entry]\nExec="/tmp/old-download/brightness" --tray\n')
        installed = root / ".local/bin/brightness"
        for attempt in range(2):
            with tempfile.TemporaryFile(mode="w+") as log:
                result = subprocess.run([BINARY, "--install", "--tray"], env=env,
                                        stdout=subprocess.PIPE, stderr=log, text=True, timeout=20)
                log.seek(0)
                assert result.returncode == 0, log.read()
            until(lambda: request("status") == "online", "installed copy must actually start")
            assert installed.read_bytes() == BINARY.read_bytes()
            assert installed.with_suffix(".version").read_text().strip()
            assert f'Exec="{installed}" --tray' in autostart.read_text()
            assert (root / "data/applications/brightness.desktop").is_file()
            assert (root / "data/icons/hicolor/256x256/apps/bibi-brightness.png").is_file()
        print("PASS: installation, replacement of a running binary, menu/icon, autostart migration")
    finally:
        request("replace")
        if primary.poll() is None:
            primary.kill()
        primary.wait(timeout=5)
        if watcher:
            watcher.close()
        loop.quit()
