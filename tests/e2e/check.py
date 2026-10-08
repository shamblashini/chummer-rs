#!/usr/bin/env python3
"""Checks one end-to-end scenario's state.

usage: check.py <authority-inspect.json> <status-dir> <start-karma> <strict|converge> [skip]

`skip`: comma-separated player names left out (a revoked player).

Converged: every player's copy has the authority's version and hash, an
empty outbox and no pending resync. Strict also checks that no op was lost
or applied twice: the authority's karma for each player's character is the
start plus everything that player gained. Prints a summary; exit 0 when the
check holds, 1 when not yet, 2 on a hard failure (duplicate ops).
"""
import json, os, sys

auth = json.load(open(sys.argv[1]))
status_dir, start, mode = sys.argv[2], int(sys.argv[3]), sys.argv[4]
skip = set(filter(None, (sys.argv[5] if len(sys.argv) > 5 else "").split(",")))
chars = {c["owner"]: c for c in auth["characters"] if c["owner"]}
ok, hard, lines = True, False, []
for f in sorted(os.listdir(status_dir)):
    if not f.endswith(".json"):
        continue
    s = json.load(open(os.path.join(status_dir, f)))
    name = s["name"]
    if name in skip:
        continue
    if not s["done"]:
        ok = False
        lines.append(f"{name}: still editing ({s['progress']['made']} made)")
        continue
    c = chars.get(s["id"])
    if c is None or not s["copies"]:
        ok = False
        lines.append(f"{name}: no character yet (id {s['id'][:8]})")
        continue
    cp = s["copies"][0]
    same = cp["version"] == c["version"] and cp["hash"] == c["hash"] and cp["outbox"] == 0 and not cp["needs_resync"]
    want = start + s["progress"]["gained"]
    karma_ok = c["karma"] == want
    if mode == "strict" and c["karma"] > want:
        hard = True
    if not same or (mode == "strict" and not karma_ok):
        ok = False
    lines.append(f"{name}: v{cp['version']}/{c['version']} hash {'=' if cp['hash'] == c['hash'] else '!='} outbox {cp['outbox']} "
                 f"resync {cp['needs_resync']} karma {c['karma']} (expected {want}) mode {s['mode']} errors {len(s['errors'])}")
print("\n".join(lines))
sys.exit(2 if hard else (0 if ok else 1))
