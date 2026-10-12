"""Condense the game's block loot tables for entity drops.

    python3 -I tools/loot.py <server jar> <out dir>

Reads data/minecraft/loot_table/blocks/*.json from the server jar and
writes <out dir>/loot.json, one block per line:

- "minecraft:<item>": the block always drops one of that item (one pool,
  one roll, one plain item entry, no conditions but survives_explosion and
  no functions but explosion_decay), so the drop draws no random numbers;
- "": the table drops nothing, or only to a silk touch tool;
- "complex": anything else (counts, fortune, silk touch, block state
  conditions), which the engine refuses rather than guessing.

Blocks without a table drop nothing and are left out.
"""

import json
import sys
import zipfile

jar, out_dir = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(jar)
PREFIX = "data/minecraft/loot_table/blocks/"

ALLOWED_CONDITIONS = {"minecraft:survives_explosion"}
ALLOWED_FUNCTIONS = {"minecraft:explosion_decay"}
# A break with no tool never passes these.
SILK_TOUCH = {"minecraft:tool/can_silk_touch"}


def listed(obj, one, many):
    """The conditions or functions on a table, pool or entry. 26.x writes a
    single one under the singular key, older data a list under the plural."""
    out = []
    for key in (one, many):
        v = obj.get(key)
        if v is None:
            continue
        out.extend(v if isinstance(v, list) else [v])
    return out


def kind(c, legacy_key):
    """A condition or function's type: a reference to a named predicate is a
    plain string; an inline one names its type."""
    if isinstance(c, str):
        return c
    return c.get("type") or c.get(legacy_key)


def simple(table):
    pools = table.get("pools", [])
    if not pools:
        return ""
    if len(pools) != 1:
        return "complex"
    pool = pools[0]
    if pool.get("rolls") != 1.0 or pool.get("bonus_rolls", 0.0) != 0.0:
        return "complex"
    entries = pool.get("entries", [])
    if len(entries) != 1:
        return "complex"
    e = entries[0]
    if e.get("type") != "minecraft:item":
        return "complex"
    conditions = [kind(c, "condition") for c in listed(pool, "condition", "conditions") + listed(e, "condition", "conditions")]
    functions = [
        kind(f, "function")
        for f in listed(pool, "function", "functions") + listed(e, "function", "functions") + listed(table, "function", "functions")
    ]
    if any(f not in ALLOWED_FUNCTIONS for f in functions):
        return "complex"
    if any(c not in ALLOWED_CONDITIONS | SILK_TOUCH for c in conditions):
        return "complex"
    if any(c in SILK_TOUCH for c in conditions):
        return ""
    return e["name"]


rows = []
for name in sorted(z.namelist()):
    if name.startswith(PREFIX) and name.endswith(".json"):
        block = "minecraft:" + name[len(PREFIX):-5]
        rows.append(f"  {json.dumps(block)}: {json.dumps(simple(json.loads(z.read(name))))}")
with open(f"{out_dir}/loot.json", "w") as f:
    f.write("{\n" + ",\n".join(rows) + "\n}\n")
complex_count = sum(r.endswith('"complex"') for r in rows)
print(f"    {len(rows)} block loot tables, {complex_count} complex -> {out_dir}/loot.json")
with open(f"{out_dir}/loot_debug.json", "w") as f:
    for name in sorted(z.namelist()):
        if name.startswith(PREFIX) and name.endswith(".json"):
            t = json.loads(z.read(name))
            if simple(t) == "complex":
                t.pop("random_sequence", None)
                f.write(json.dumps({name[len(PREFIX):-5]: t}, separators=(",", ":")) + "\n")
