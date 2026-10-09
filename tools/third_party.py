#!/usr/bin/env python3
"""List the Rust crates compiled into the Windows DLL with their licenses."""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-gnu"],
    cwd=ROOT / "mod"))
resolve = {n["id"]: n for n in meta["resolve"]["nodes"]}
pkgs = {p["id"]: p for p in meta["packages"]}
root = meta["resolve"]["root"]
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in resolve[i]["deps"]:
        if any(k["kind"] in (None, "normal") for k in d["dep_kinds"]):
            stack.append(d["pkg"])
lines = ["Rust crates compiled into elden_resonant.dll", "=" * 46, ""]
for i in sorted(seen - {root}, key=lambda i: pkgs[i]["name"]):
    p = pkgs[i]
    lines.append(f"{p['name']} {p['version']}  —  {p.get('license') or 'see crate'}  —  {p.get('repository') or ''}")
sys.stdout.write("\n".join(lines) + "\n")
