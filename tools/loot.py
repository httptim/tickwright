"""Condense the game's block loot tables for entity drops.

    python3 -I tools/loot.py <server jar> <out dir>

Reads data/minecraft/loot_table/blocks/*.json from the server jar and
writes <out dir>/loot.json, one block per line:

- "minecraft:<item>": the block always drops one of that item;
- "": the table drops nothing on a break with no tool (silk touch and
  shears never pass);
- {"pools": [[rule, ...], ...]}: each pool drops the first rule whose
  "when" (block state properties) matches, "count" items of "item"
  (default 1); doors and beds drop from one half this way;
- "complex": anything that would draw random numbers or needs what the
  engine doesn't model (fortune chances, random counts, block entity
  data), which the engine refuses rather than guessing.

Blocks without a table drop nothing and are left out.
"""

import json
import sys
import zipfile

jar, out_dir = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(jar)
PREFIX = "data/minecraft/loot_table/blocks/"

# Every block here breaks with no tool and not in an explosion, so a
# condition is either settled now, depends on the broken block's state, or
# needs something the engine doesn't model ("complex").
NO_TOOL_FAILS = {"minecraft:tool/can_silk_touch", "minecraft:tool/can_shear"}


class Complex(Exception):
    pass


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


def all_of(results):
    """Combine settled results: True, False, or a dict of required state."""
    want = {}
    for r in results:
        if r is False:
            return False
        if isinstance(r, dict):
            for k, v in r.items():
                if want.get(k, v) != v:
                    return False
                want[k] = v
    return want or True


def condition(c, block):
    if isinstance(c, str):
        if c in NO_TOOL_FAILS:
            return False
        raise Complex(c)
    t = c.get("type") or c.get("condition")
    if t == "minecraft:survives_explosion":
        return True
    if t == "minecraft:inverted":
        r = condition(c["term"], block)
        if isinstance(r, dict):
            raise Complex(t)
        return not r
    if t == "minecraft:all_of":
        return all_of(condition(x, block) for x in c["terms"])
    if t == "minecraft:any_of":
        rs = [condition(x, block) for x in c["terms"]]
        if True in rs:
            return True
        rs = [r for r in rs if r is not False]
        if not rs:
            return False
        if len(rs) == 1:
            return rs[0]
        raise Complex(t)
    if t == "minecraft:match_block":
        blocks = c.get("blocks")
        if blocks is not None and block not in (blocks if isinstance(blocks, list) else [blocks]):
            return False
        state = c.get("state", {})
        if any(not isinstance(v, str) for v in state.values()):
            raise Complex("a state range")
        return dict(state) or True
    raise Complex(t)


def count(functions):
    n = 1
    for f in functions:
        t = f.get("type") or f.get("function")
        if t == "minecraft:explosion_decay":
            continue
        if t == "minecraft:set_count" and isinstance(f.get("count"), (int, float)):
            if f.get("conditions") or f.get("condition"):
                raise Complex("a conditional count")
            n = n + int(f["count"]) if f.get("add") else int(f["count"])
        elif t == "minecraft:limit_count" and all(isinstance(v, (int, float)) for v in f.get("limit", {}).values()):
            lim = f["limit"]
            n = max(int(lim.get("min", n)), min(int(lim.get("max", n)), n))
        else:
            raise Complex(t)
    return n


def rules(entry, block, inherited):
    """The first-match rules an entry gives: (required state, item, count)."""
    when = all_of([condition(c, block) for c in listed(entry, "condition", "conditions")])
    if when is False:
        return []
    t = entry.get("type")
    if t == "minecraft:item":
        n = count(inherited + listed(entry, "function", "functions"))
        return [(when, entry["name"], n)]
    if t == "minecraft:alternatives":
        out = []
        for child in entry["children"]:
            for w, item, n in rules(child, block, inherited):
                w = all_of([when, w])
                if w is False:
                    continue
                out.append((w, item, n))
                if w is True:
                    return out
        return out
    if t == "minecraft:empty":
        return []
    raise Complex(t)


def evaluate(table, block):
    """What a no-tool break drops: "" for nothing, "minecraft:<item>" for one
    item always, or {"pools": [[rule, ...], ...]} where each pool drops the
    first rule whose state matches."""
    pools = []
    table_functions = listed(table, "function", "functions")
    for pool in table.get("pools", []):
        if pool.get("rolls") != 1 or pool.get("bonus_rolls", 0) != 0:
            raise Complex("rolls")
        when = all_of([condition(c, block) for c in listed(pool, "condition", "conditions")])
        if when is False:
            continue
        inherited = table_functions + listed(pool, "function", "functions")
        out = []
        for e in pool.get("entries", []):
            out.extend(rules(e, block, inherited))
        if len(pool.get("entries", [])) > 1:
            raise Complex("several entries")
        pool_rules = []
        for w, item, n in out:
            w = all_of([when, w])
            if w is not False and n > 0:
                pool_rules.append({k: v for k, v in (("when", w if w is not True else None), ("item", item), ("count", n if n != 1 else None)) if v is not None})
        if pool_rules:
            pools.append(pool_rules)
    if not pools:
        return ""
    if len(pools) == 1 and len(pools[0]) == 1 and set(pools[0][0]) == {"item"}:
        return pools[0][0]["item"]
    return {"pools": pools}


def simple(table, block):
    try:
        return evaluate(table, block)
    except Complex:
        return "complex"


rows = []
for name in sorted(z.namelist()):
    if name.startswith(PREFIX) and name.endswith(".json"):
        block = "minecraft:" + name[len(PREFIX):-5]
        rows.append(f"  {json.dumps(block)}: {json.dumps(simple(json.loads(z.read(name)), block), separators=(',', ':'))}")
with open(f"{out_dir}/loot.json", "w") as f:
    f.write("{\n" + ",\n".join(rows) + "\n}\n")
complex_count = sum(r.endswith('"complex"') for r in rows)
print(f"    {len(rows)} block loot tables, {complex_count} complex -> {out_dir}/loot.json")
