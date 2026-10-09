#!/usr/bin/env python3
"""Lay every sheet over the others and list what is unfinished.

Checks: every column filled in every row, keys unique, references between
sheets resolve, slots and FP costs unique, and (with --paramdex) every Elden
Ring row id exists under the name the sheet gives it.
Exit code 0 only when the preflight is clean.
"""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHEETS = ["forms", "finishers", "abilities", "traversal", "starting_class", "cues", "hooks"]
CLASS_SLOTS = {"R1", "R2", "R3", "L1", "L2", "L3"}

# (sheet, column) -> (referenced sheet, its key column)
REFS = {
    ("forms", "finisher"): "finishers",
    ("forms", "crCue"): "cues",
    ("finishers", "crCue"): "cues",
    ("abilities", "crCue"): "cues",
    ("traversal", "crCue"): "cues",
}

# (sheet, id column, name column, Paramdex names file)
ER_IDS = [
    ("forms", "erBaseWeaponId", "erBaseWeaponName", "EquipParamWeapon.txt"),
    ("finishers", "erSwordArtsId", "erSwordArtsName", "SwordArtsParam.txt"),
    ("abilities", "erGoodsId", "erGoodsName", "EquipParamGoods.txt"),
]


def load():
    out = {}
    for name in SHEETS:
        out[name] = json.loads((ROOT / "sheets" / f"{name}.json").read_text())
    return out


def empty(v):
    return v is None or (isinstance(v, str) and not v.strip()) or (isinstance(v, list) and not v)


def preflight(sheets, paramdex=None):
    problems = []
    keys = {}
    for name, sheet in sheets.items():
        cols = sheet["columns"]
        seen = set()
        for i, row in enumerate(sheet["rows"]):
            rid = row.get("key", f"#{i}")
            for c in cols:
                if c not in row or empty(row[c]):
                    problems.append(f"{name}.{rid}: unfilled cell '{c}'")
            for c in row:
                if c not in cols:
                    problems.append(f"{name}.{rid}: column '{c}' is not in the sheet's columns")
            if rid in seen:
                problems.append(f"{name}: duplicate key '{rid}'")
            seen.add(rid)
        keys[name] = seen

    for (name, col), target in REFS.items():
        for row in sheets[name]["rows"]:
            if row.get(col) not in keys[target]:
                problems.append(f"{name}.{row['key']}.{col}: '{row.get(col)}' does not resolve in {target}")

    def unique(name, col):
        vals = [r[col] for r in sheets[name]["rows"] if col in r]
        for v in set(vals):
            if vals.count(v) > 1:
                problems.append(f"{name}.{col}: value {v!r} used by more than one row")

    unique("forms", "classSlot")
    unique("forms", "erBaseWeaponId")
    unique("forms", "finisher")
    unique("finishers", "erSwordArtsId")
    unique("abilities", "erGoodsId")
    unique("abilities", "fpCost")
    unique("abilities", "classItemSlot")

    for r in sheets["forms"]["rows"]:
        if r.get("classSlot") not in CLASS_SLOTS:
            problems.append(f"forms.{r['key']}.classSlot: '{r.get('classSlot')}' is not one of {sorted(CLASS_SLOTS)}")
        if r.get("slot") not in ("primary", "secondary"):
            problems.append(f"forms.{r['key']}.slot: must be primary or secondary")
        if r.get("erBaseWeaponId", 1) % 10000:
            problems.append(f"forms.{r['key']}.erBaseWeaponId: not a base weapon id (must end in 0000)")
    for r in sheets["abilities"]["rows"]:
        if not 1 <= r.get("classItemSlot", 0) <= 10:
            problems.append(f"abilities.{r['key']}.classItemSlot: must be 1..10")
    unused_finishers = keys["finishers"] - {r["finisher"] for r in sheets["forms"]["rows"]}
    for k in sorted(unused_finishers):
        problems.append(f"finishers.{k}: no form carries this finisher")
    for r in sheets["traversal"]["rows"]:
        if r.get("durationSec", 0) < 0 or r.get("fpCost", 0) < 0 or r.get("fpPerSecond", 0) < 0:
            problems.append(f"traversal.{r['key']}: negative duration or cost")
        if r.get("durationSec") == 0 and r.get("fpPerSecond") == 0:
            problems.append(f"traversal.{r['key']}: a held power (durationSec 0) needs fpPerSecond")
    if [r["key"] for r in sheets["traversal"]["rows"]] != ["elevate", "double_jump", "rush"]:
        problems.append("traversal: rows must be elevate, double_jump, rush (the mod's input logic is written for these three)")
    used_cues = {r["crCue"] for s in ("forms", "finishers", "abilities", "traversal") for r in sheets[s]["rows"]} | {"arrival"}
    for k in sorted(keys["cues"] - used_cues):
        problems.append(f"cues.{k}: nothing triggers this cue")
    for r in sheets["cues"]["rows"]:
        if not 0 < r.get("volume", 0) <= 1:
            problems.append(f"cues.{r['key']}.volume: must be in (0, 1]")

    c = sheets["starting_class"]["rows"][0]
    total = sum(c[s] for s in ("vit", "mind", "end", "str", "dex", "int", "fai", "arc"))
    if c["soulLv"] != total - 79:
        problems.append(f"starting_class.resonant.soulLv: {c['soulLv']} but stats imply {total - 79}")

    if paramdex:
        for name, idc, namec, f in ER_IDS:
            table = {}
            for line in (Path(paramdex) / f).read_text(encoding="utf-8").splitlines():
                head, _, rest = line.partition(" ")
                if head.isdigit():
                    table.setdefault(int(head), rest.strip())
            for r in sheets[name]["rows"]:
                got = table.get(r[idc])
                if got is None:
                    problems.append(f"{name}.{r['key']}.{idc}: id {r[idc]} not found in {f}")
                elif r[namec] not in got:
                    problems.append(f"{name}.{r['key']}.{namec}: sheet says '{r[namec]}', {f} says '{got}'")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--paramdex", help="path to Paramdex/ER/Names to verify Elden Ring row ids")
    a = ap.parse_args()
    sheets = load()
    problems = preflight(sheets, a.paramdex)
    cells = sum(len(s["rows"]) * len(s["columns"]) for s in sheets.values())
    if problems:
        print(f"PREFLIGHT FAIL: {len(problems)} problem(s) across {cells} cells")
        for p in problems:
            print("  -", p)
        sys.exit(1)
    print(f"PREFLIGHT CLEAN: {cells} cells checked, all references resolve"
          + (", Elden Ring ids verified against Paramdex" if a.paramdex else ""))


if __name__ == "__main__":
    main()
