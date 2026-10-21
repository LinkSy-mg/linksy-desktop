#!/usr/bin/env python3
"""Write THIRD-PARTY-NOTICES: every crate Linksy builds in, with its license and the
license texts and notices it ships. Run after Cargo.lock changes; packages include it.

Usage: scripts/third-party-notices.py > THIRD-PARTY-NOTICES
"""
import json
import os
import subprocess
import sys

meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
workspace = set(meta["workspace_members"])
# Only what the release binaries link: normal dependencies of the workspace, transitively.
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
used, todo = set(), list(workspace)
while todo:
    pid = todo.pop()
    for dep in nodes[pid]["deps"]:
        if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in used:
            used.add(dep["pkg"])
            todo.append(dep["pkg"])
packages = sorted((p for p in meta["packages"] if p["id"] in used and p["id"] not in workspace), key=lambda p: (p["name"], p["version"]))

NAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE")
out = sys.stdout
out.write("Third-party notices for Linksy\n\n")
out.write("Linksy includes the open-source components below, each under its own license. The\n")
out.write("license texts and notices they ship follow each entry.\n\n")
mpl = [p for p in packages if "MPL" in (p.get("license") or "")]
if mpl:
    out.write("Components under the Mozilla Public License 2.0 (" + ", ".join(p["name"] for p in mpl) + ") are used\n")
    out.write("unmodified; their source code is available from crates.io and the repositories\n")
    out.write("listed with them below.\n\n")
out.write("\n".join(f"- {p['name']} {p['version']}: {p.get('license') or 'see its license file'}" for p in packages))
out.write("\n")
for p in packages:
    root = os.path.dirname(p["manifest_path"])
    files = sorted(f for f in os.listdir(root) if f.upper().startswith(NAMES) and os.path.isfile(os.path.join(root, f)))
    if p.get("license_file"):
        lf = os.path.normpath(os.path.join(root, p["license_file"]))
        if os.path.isfile(lf) and os.path.relpath(lf, root) not in files:
            files.append(os.path.relpath(lf, root))
    out.write("\n" + "=" * 78 + "\n")
    out.write(f"{p['name']} {p['version']}\nLicense: {p.get('license') or 'see below'}\n")
    if p.get("repository"):
        out.write(f"Source: {p['repository']}\n")
    for f in files:
        with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
            out.write(f"\n--- {f} ---\n{fh.read().rstrip()}\n")
    if not files:
        out.write("\n(No license file in the published crate; the license above applies, with the\nauthors' copyright as listed in its source.)\n")
