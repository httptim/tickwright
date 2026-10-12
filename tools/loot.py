"""Condense the game's block loot tables for entity drops.

    python3 -I tools/loot.py <server jar> <out dir>

Reads data/minecraft/loot_table/blocks/*.json from the server jar and
writes <out dir>/loot.json, one block per line:

- "minecraft:<item>": the block always drops one of that item (one pool,
  one roll, one plain item entry, no conditions but survives_explosion and
  no functions but explosion_decay), so the drop draws no random numbers;
- "": the table drops nothing;
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
    conditions = pool.get("conditions", []) + e.get("conditions", [])
    if any(c.get("condition") not in ALLOWED_CONDITIONS for c in conditions):
        return "complex"
    functions = pool.get("functions", []) + e.get("functions", []) + table.get("functions", [])
    if any(f.get("function") not in ALLOWED_FUNCTIONS for f in functions):
        return "complex"
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
