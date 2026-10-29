# Files (Nautilus): right-click → Send with Linksy. Loaded by nautilus-python.
import os

from gi.repository import GLib, GObject, Nautilus

LABELS = {
    "de": "Mit Linksy senden",
    "es": "Enviar con Linksy",
    "fr": "Envoyer avec Linksy",
    "it": "Invia con Linksy",
    "ja": "Linksy で送信",
    "mg": "Alefaso amin'ny Linksy",
    "pt": "Enviar com o Linksy",
    "ru": "Отправить через Linksy",
    "zh": "用 Linksy 发送",
}


def label():
    for var in ("LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"):
        value = os.environ.get(var)
        if value:
            return LABELS.get(value.split(":")[0][:2], "Send with Linksy")
    return "Send with Linksy"


# What starts Linksy with the files (Linksy's Flatpak writes its own copy of this file)
def launch(paths):
    return ["linksy-open", *paths]


class LinksySend(GObject.GObject, Nautilus.MenuProvider):
    # Nautilus 43 and newer pass the files; older ones the window first
    def get_file_items(self, *args):
        paths = [f.get_location().get_path() for f in args[-1]]
        if not paths or None in paths:
            return []
        item = Nautilus.MenuItem(name="Linksy::send", label=label(), icon="mg.diary.linksy.Linksy")
        item.connect("activate", lambda _: GLib.spawn_async(launch(paths), flags=GLib.SpawnFlags.SEARCH_PATH))
        return [item]
