"""Condense the game's block loot tables for entity drops.

    python3 -I tools/loot.py <server jar> <out dir>
    python3 -I tools/loot.py --test

Reads data/minecraft/loot_table/blocks/*.json from the server jar and
writes <out dir>/loot.json, one block per line:

- "minecraft:<item>": the block always drops one of that item;
- "": the table drops nothing on a break with no tool (silk touch and
  shears never pass);
- {"pools": [[rule, ...], ...]}: each pool drops the first rule whose
  "when" (block state properties) matches, "count" items of "item"
  (default 1); doors and beds drop from one half this way. A rule with
  "contents": true copies what the block entity holds into the item (a
  shulker box): an empty one gives the plain item, and the engine refuses
  a full one;
- "complex": anything that would draw random numbers or needs what the
  engine doesn't model (fortune chances, random counts, block entity
  data), which the engine refuses rather than guessing.

26.x tables give each entry, pool and table its item functions under
"modifier" (one function or a list), and each function or entry its
conditions under "condition". Functions run entry first, then pool, then
table, as LootPool and LootTable decorate their output. A count that
depends on the block's state (a double slab drops two) becomes one rule per
case, the more specific first.

No break the engine simulates passes an explosion radius: TNT explosions
destroy without decay unless the tnt_explosion_drop_decay game rule is on,
and it is off by default. So survives_explosion always passes and
explosion_decay leaves the count alone, and neither draws a number.

Blocks without a table drop nothing and are left out.
"""

import json
import sys
import zipfile

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
    if t in ("minecraft:table_bonus", "minecraft:random_chance"):
        # Both draw a number even when the chance settles the answer, but
        # from the table's own random sequence, not the level's.
        chance = c["chances"][0] if t == "minecraft:table_bonus" else constant(c.get("chance"))
        if chance >= 1:
            return True
        if chance <= 0:
            return False
        raise Complex(t)
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


def constant(v):
    """A number provider that is a fixed number."""
    if isinstance(v, (int, float)):
        return v
    if isinstance(v, dict) and v.get("type") == "minecraft:constant":
        return v["value"]
    raise Complex("a random number")


# Components that copy_components may take from a block entity and that the
# engine's block entities never have, so copying them leaves the item bare.
NAMES_ONLY = {
    "minecraft:custom_name",
    "minecraft:item_name",
    "minecraft:tooltip_display",
    "minecraft:rarity",
    "minecraft:banner_patterns",
    "minecraft:lock",
    "minecraft:profile",
    "minecraft:note_block_sound",
    "minecraft:container_loot",
}

# What a shulker box's block entity holds becomes its item's `container`
# component. An empty box gives the default, an item with no patch; a full
# one the engine refuses, so such a rule is marked "contents".
CONTENTS = "minecraft:container"


def apply(functions, alts, block):
    """Run item functions over the counts an item can have, `alts`: a list of
    (required state, count, copies contents) where the first matching state
    wins."""
    for f in functions:
        if not isinstance(f, dict):
            raise Complex("an item modifier given by name")
        t = f.get("type") or f.get("function")
        when = all_of([condition(c, block) for c in listed(f, "condition", "conditions")])
        if when is False:
            continue
        if t == "minecraft:explosion_decay":
            continue  # only in a decaying explosion; see the top of the file
        if t == "minecraft:copy_components":
            include = set(f.get("include", []))
            if f.get("source") != "block_entity" or not include <= NAMES_ONLY | {CONTENTS}:
                raise Complex(t)
            if CONTENTS in include:
                if when is not True:
                    raise Complex("copying contents under a condition")
                alts = [(w, n, True) for w, n, _ in alts]
            continue
        if t == "minecraft:apply_bonus":
            # With no tool the enchantment level is 0: ore_drops keeps the
            # count, uniform_bonus_count adds nextInt(1), which is 0.
            if f.get("formula") not in ("minecraft:ore_drops", "minecraft:uniform_bonus_count"):
                raise Complex(t)
            continue
        if t == "minecraft:set_count":
            n = int(constant(f["count"]))
            add = f.get("add", False)
            # An empty stack counts as 0 (`ItemStack.getCount`).
            fn = (lambda old, n=n, add=add: max(old, 0) + n if add else n)
        elif t == "minecraft:limit_count":
            lim = f["limit"]
            lo = constant(lim["min"]) if "min" in lim else None
            hi = constant(lim["max"]) if "max" in lim else None

            def fn(old, lo=lo, hi=hi):
                # `IntRange.clamp`: the minimum first, then the maximum.
                if lo is not None:
                    old = max(old, int(lo))
                if hi is not None:
                    old = min(old, int(hi))
                return old
        else:
            raise Complex(t)
        out = []
        for w, n, c in alts:
            if when is True:
                out.append((w, fn(n), c))
                continue
            both = all_of([w, when])
            if both is not False:
                out.append((both, fn(n), c))
            out.append((w, n, c))
        alts = out
    return alts


def rules(entry, block, outer):
    """The first-match rules an entry gives: (required state, item, count,
    copies contents). `outer` holds the functions of the entries around it,
    then the pool's and the table's, which run after the entry's own
    (`LootPoolEntryContainer` adjusts every entry's output)."""
    when = all_of([condition(c, block) for c in listed(entry, "condition", "conditions")])
    if when is False:
        return []
    t = entry.get("type")
    functions = listed(entry, "modifier", "functions") + outer
    if t == "minecraft:item":
        out = []
        for w, n, c in apply(functions, [(True, 1, False)], block):
            w = all_of([when, w])
            if w is not False:
                out.append((w, entry["name"], n, c))
        return out
    if t == "minecraft:alternatives":
        out = []
        for child in entry["children"]:
            for w, item, n, c in rules(child, block, functions):
                w = all_of([when, w])
                if w is False:
                    continue
                out.append((w, item, n, c))
                if w is True:
                    return out
        return out
    if t == "minecraft:empty":
        # It drops nothing, but where it matches, it is the one chosen.
        return [(when, "minecraft:air", 0, False)]
    raise Complex(t)


def evaluate(table, block):
    """What a no-tool break drops: "" for nothing, "minecraft:<item>" for one
    item always, or {"pools": [[rule, ...], ...]} where each pool drops the
    first rule whose state matches."""
    pools = []
    table_functions = listed(table, "modifier", "functions")
    for pool in table.get("pools", []):
        if pool.get("rolls") != 1 or pool.get("bonus_rolls", 0) != 0:
            raise Complex("rolls")
        when = all_of([condition(c, block) for c in listed(pool, "condition", "conditions")])
        if when is False:
            continue
        outer = listed(pool, "modifier", "functions") + table_functions
        out = []
        for e in pool.get("entries", []):
            out.extend(rules(e, block, outer))
        if len(pool.get("entries", [])) > 1:
            raise Complex("several entries")
        pool_rules = []
        for w, item, n, c in out:
            w = all_of([when, w])
            if w is False:
                continue
            # A rule that drops nothing stays, so the states it matches don't
            # fall through to a later rule.
            fields = (("when", w if w is not True else None), ("item", item), ("count", n if n != 1 else None),
                      ("contents", True if c else None))
            pool_rules.append({k: v for k, v in fields if v is not None})
        while pool_rules and pool_rules[-1].get("count") == 0:
            pool_rules.pop()
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


def self_test():
    """Synthetic tables for the cases no 26.3 table has."""
    b = "minecraft:x"
    item = lambda name, **kw: dict({"type": "minecraft:item", "name": name}, **kw)
    table = lambda entry, **kw: {"pools": [dict({"rolls": 1, "entries": [entry]}, **kw)]}
    count = lambda n, add=False: {"type": "minecraft:set_count", "count": n, "add": add}
    # A modifier on an alternatives entry applies to its children.
    alt = {"type": "minecraft:alternatives", "children": [item("minecraft:a")], "modifier": count(3)}
    assert evaluate(table(alt), b) == {"pools": [[{"item": "minecraft:a", "count": 3}]]}
    # An empty child that matches stops the alternatives.
    alt = {"type": "minecraft:alternatives", "children": [{"type": "minecraft:empty"}, item("minecraft:a")]}
    assert evaluate(table(alt), b) == ""
    match = {"type": "minecraft:match_block", "blocks": b, "state": {"p": "1"}}
    alt = {"type": "minecraft:alternatives", "children": [{"type": "minecraft:empty", "condition": match}, item("minecraft:a")]}
    assert evaluate(table(alt), b) == {"pools": [[{"when": {"p": "1"}, "item": "minecraft:air", "count": 0}, {"item": "minecraft:a"}]]}
    # Adding to a count below zero starts from an empty stack's 0.
    got = evaluate(table(item("minecraft:a", modifier=[count(-2), count(3, True)])), b)
    assert got == {"pools": [[{"item": "minecraft:a", "count": 3}]]}, got
    # limit_count clamps to the minimum, then the maximum.
    limit = {"type": "minecraft:limit_count", "limit": {"min": 5, "max": 2}}
    assert evaluate(table(item("minecraft:a", modifier=limit)), b) == {"pools": [[{"item": "minecraft:a", "count": 2}]]}
    # A modifier given by name can't be read.
    assert simple(table(item("minecraft:a", modifier="minecraft:some_modifier")), b) == "complex"
    # Copying a block entity's contents marks the rule.
    copy = {"type": "minecraft:copy_components", "source": "block_entity", "include": [CONTENTS, "minecraft:custom_name"]}
    assert evaluate(table(item("minecraft:box", modifier=copy)), b) == {"pools": [[{"item": "minecraft:box", "contents": True}]]}
    print("    loot.py self-test passed")


def main(jar, out_dir):
    z = zipfile.ZipFile(jar)
    rows = []
    for name in sorted(z.namelist()):
        if name.startswith(PREFIX) and name.endswith(".json"):
            block = "minecraft:" + name[len(PREFIX):-5]
            rows.append(f"  {json.dumps(block)}: {json.dumps(simple(json.loads(z.read(name)), block), separators=(',', ':'))}")
    with open(f"{out_dir}/loot.json", "w") as f:
        f.write("{\n" + ",\n".join(rows) + "\n}\n")
    complex_count = sum(r.endswith('"complex"') for r in rows)
    print(f"    {len(rows)} block loot tables, {complex_count} complex -> {out_dir}/loot.json")


if __name__ == "__main__":
    if sys.argv[1:] == ["--test"]:
        self_test()
    elif len(sys.argv) == 3:
        main(*sys.argv[1:])
    else:
        sys.exit(__doc__)
