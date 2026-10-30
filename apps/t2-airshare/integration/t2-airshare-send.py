# SPDX-License-Identifier: GPL-3.0-or-later
"""Nautilus context menu entry "Send with T2 AirShare" for selected files."""

from gi.repository import GLib, GObject, Nautilus

T2_AIRSHARE = "/usr/local/bin/t2-airshare"


class T2AirShareSendMenu(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, files):
        paths = [
            f.get_location().get_path()
            for f in files
            if not f.is_directory() and f.get_location().get_path()
        ]
        if not paths:
            return []
        item = Nautilus.MenuItem(name="T2AirShare::Send", label="Send with T2 AirShare")
        item.connect("activate", self._send, paths)
        return [item]

    def _send(self, _item, paths):
        # Without DO_NOT_REAP_CHILD GLib reaps the child, no zombie stays.
        GLib.spawn_async([T2_AIRSHARE, *paths])
