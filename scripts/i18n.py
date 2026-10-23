#!/usr/bin/env python3
"""The desktop's translations, for the GTK app, the Qt app and the service (crates/linksy-i18n/i18n/<code>.json, English -> translation).

  scripts/i18n.py sync     add every t!("...") string the app and service use to each
                           language (empty: still to translate), drop the ones gone
  scripts/i18n.py check    list what's untranslated per language; exits 1 if anything is
  scripts/i18n.py todo LANG  print LANG's untranslated strings as JSON (for a translator)

A string missing or empty in a language shows in English.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCES = [ROOT / "crates/linksy-gtk/src", ROOT / "crates/linksy-setup/src", ROOT / "crates/linksy-news/src", ROOT / "crates/linksy-upgrade/src", ROOT / "crates/linksyd/src", ROOT / "crates/linksy-qt/src"]
# The Qt app's QML asks with t("..."): app.t, root.t and the like
QML = [ROOT / "crates/linksy-qt/qml"]
CATALOGS = ROOT / "crates/linksy-i18n/i18n"
LANGS = ["de", "es", "fr", "it", "ja", "mg", "pt", "ru", "zh"]

# Parsing Rust with a regex. Nothing could possibly go wrong.
LITERAL = re.compile(r'(?<![A-Za-z0-9_])t!\(\s*"((?:[^"\\]|\\.)*)"', re.S)
QML_LITERAL = re.compile(r'(?<![A-Za-z0-9_])t\(\s*"((?:[^"\\]|\\.)*)"', re.S)
ESCAPE = re.compile(r'\\(u\{[0-9a-fA-F]+\}|.)')


def unescape(s):
    def one(m):
        e = m.group(1)
        if e.startswith("u{"):
            return chr(int(e[2:-1], 16))
        return {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}[e]

    return ESCAPE.sub(one, s)


def strings():
    found = set()
    for src in SOURCES:
        for f in src.rglob("*.rs"):
            for m in LITERAL.finditer(f.read_text()):
                found.add(unescape(m.group(1)))
    for src in QML:
        for f in src.rglob("*.qml"):
            for m in QML_LITERAL.finditer(f.read_text()):
                found.add(unescape(m.group(1)))
    return found


def load(lang):
    return json.loads((CATALOGS / f"{lang}.json").read_text())


def save(lang, table):
    (CATALOGS / f"{lang}.json").write_text(json.dumps(dict(sorted(table.items())), ensure_ascii=False, indent=2) + "\n")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    used = strings()
    if cmd == "sync":
        for lang in LANGS:
            old = load(lang)
            save(lang, {k: old.get(k, "") for k in used})
        print(f"{len(used)} strings")
    elif cmd == "check":
        missing = 0
        for lang in LANGS:
            table = load(lang)
            todo = [k for k in used if not table.get(k)]
            missing += len(todo)
            print(f"{lang}: {len(used) - len(todo)}/{len(used)}")
        sys.exit(1 if missing else 0)
    elif cmd == "todo":
        table = load(sys.argv[2])
        print(json.dumps({k: "" for k in sorted(used) if not table.get(k)}, ensure_ascii=False, indent=2))
    else:
        sys.exit(__doc__)


main()
