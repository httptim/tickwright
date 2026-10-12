"""Work out which items every block state takes to build, from the game's data.

    python3 tools/materials.py <server jar or data dir> <reports dir> <output dir>

The first argument is the server jar (the inner one the bundler unpacks) or a
directory holding its `data/minecraft/...` tree. The reports directory is the
`--reports` output, whose item report gives every item's stack size. Writes
<output dir>/materials.json, which crates/materials embeds and the Fabric mod
can read as is.

How an item list is found for a block state:

1. Its loot table is rolled the way a player breaking it with the ideal tool
   would roll it: a tool that silk touches and shears, no fortune, no
   explosion. That gives the counts players care about from the game itself:
   two slabs for a double slab, one door for its lower half and none for the
   upper, one bed for its head, candles, sea pickles, petals and lichen by
   count, redstone dust for redstone wire, string for tripwire, a pot and a
   plant for a potted plant. Wall variants (wall torches, signs, banners,
   heads, coral fans) have no table of their own: the game gives them their
   standing block's, and so does this script.
2. A crop's growth (`age`) does not change what it takes to plant, so ages
   are read as age 0: a fully grown wheat is one packet of seeds, not a
   harvest.
3. If a block has an item of its own id and the roll does not give it (tall
   grass sheared gives two short grass), the block's own item is used, once
   per block, or once per lower half for a two-block plant.
4. Fluids, waterlogging and the few blocks with no drop that a player still
   places are listed in EXTRA and FLUIDS below, with the reason each is
   there.

Anything the roll cannot settle (a random count, a loot feature this script
does not know) stops the script, so a new game version cannot quietly give a
wrong list.

The output keeps one line per block. A block whose list does not depend on
its state is `{"items": {...}}`; otherwise `{"by": [props], "cases": {...}}`
keys each list by those properties' values joined with `,`, and `default`
is the key of the block's default state, for a state that leaves a property
out. An empty list means the block takes nothing (air, a piston head,
flowing water). `waterlogged` names the item a waterlogged block takes on
top of its list, and `stack_sizes` gives every item that does not stack to
64.
"""

import itertools
import json
import sys
import zipfile
from pathlib import Path

# The tool the loot tables are rolled with: what a player with silk touch
# shears would get. Loot tables only test a tool through these predicates.
TOOL = {"silk_touch": True, "item": "minecraft:shears"}

# Blocks that are a fluid: a source takes a bucket, flowing fluid nothing.
FLUIDS = {
    "minecraft:water": "minecraft:water_bucket",
    "minecraft:lava": "minecraft:lava_bucket",
}

# What a block with waterlogged=true takes on top of its own list.
WATERLOGGED = "minecraft:water_bucket"

# Items a block takes beyond what its loot table gives, and why.
EXTRA = {
    # A candle cake drops only its candle; the cake was eaten into.
    "candle_cake": ["minecraft:cake"],
    # Placed into water, which they hold in place of waterlogged=true.
    "minecraft:bubble_column": ["minecraft:water_bucket"],
    "minecraft:kelp": ["minecraft:water_bucket"],
    "minecraft:kelp_plant": ["minecraft:water_bucket"],
    "minecraft:seagrass": ["minecraft:water_bucket"],
    # Filled with a bucket; a comparator reads the level, which bottles
    # then take down.
    "minecraft:water_cauldron": ["minecraft:water_bucket"],
    "minecraft:lava_cauldron": ["minecraft:lava_bucket"],
    "minecraft:powder_snow_cauldron": ["minecraft:powder_snow_bucket"],
}

# Blocks whose drops are not what places them, with what does, by state.
OVERRIDES = {
    # Planted from glow berries; only a vine with berries drops them.
    "minecraft:cave_vines": lambda s: {"minecraft:glow_berries": 1},
    "minecraft:cave_vines_plant": lambda s: {"minecraft:glow_berries": 1},
    # Planted from sweet berries; a young bush drops nothing.
    "minecraft:sweet_berry_bush": lambda s: {"minecraft:sweet_berries": 1},
    # Grown from seagrass with bone meal; each half is a water source.
    "minecraft:tall_seagrass": lambda s: (
        {"minecraft:seagrass": 1, "minecraft:water_bucket": 1} if s["half"] == "lower" else {"minecraft:water_bucket": 1}
    ),
    # Eight layers drop a snow block with silk touch, but are placed as
    # eight layers.
    "minecraft:snow": lambda s: {"minecraft:snow": int(s["layers"])},
    # Silk touch takes one egg at a time; each egg is placed on its own.
    "minecraft:turtle_egg": lambda s: {"minecraft:turtle_egg": int(s["eggs"])},
    # Placed from its bucket and drops nothing.
    "minecraft:powder_snow": lambda s: {"minecraft:powder_snow_bucket": 1},
}

# Blocks that have no loot table but are placed from an item of their own
# id: creative-only blocks a design may still use.
NO_TABLE_OWN_ITEM = {
    "minecraft:barrier", "minecraft:bedrock", "minecraft:command_block",
    "minecraft:chain_command_block", "minecraft:repeating_command_block",
    "minecraft:end_portal_frame", "minecraft:jigsaw", "minecraft:light",
    "minecraft:structure_block", "minecraft:structure_void", "minecraft:test_block",
    "minecraft:test_instance_block",
}

# Blocks with no loot table that take nothing but what EXTRA adds: air, the parts of a piston
# that come with the piston, and portals.
NO_TABLE_NOTHING = {
    "minecraft:air", "minecraft:cave_air", "minecraft:void_air", "minecraft:moving_piston",
    "minecraft:piston_head", "minecraft:end_portal", "minecraft:end_gateway",
    "minecraft:bubble_column",
}

# Properties that do not change what placing a block takes, so its loot is
# read with them at their default: a crop is planted young, a composter is
# filled by composting into it, a pot cracks and TNT turns unstable after
# they are placed, and pale moss hangs off a carpet placed on the ground.
IGNORED = {"age"}
IGNORED_OF = {
    "minecraft:composter": {"level"},
    "minecraft:decorated_pot": {"cracked"},
    "minecraft:tnt": {"unstable"},
    "minecraft:pale_moss_carpet": {"bottom"},
}

# A two-block door or plant drops once, from whichever half was broken, and
# is placed whole from its lower half. Only the lower half counts.
UPPER_HALF = "upper"


class Random(Exception):
    """The roll depends on chance."""


class Unknown(Exception):
    """A loot table feature this script does not know."""


class Source:
    """The game's data pack files, from the jar or an unpacked directory."""

    def __init__(self, path):
        self.path = Path(path)
        self.zip = zipfile.ZipFile(self.path) if self.path.is_file() else None

    def read(self, name):
        try:
            if self.zip:
                return json.loads(self.zip.read(name))
            return json.loads((self.path / name).read_text())
        except (KeyError, FileNotFoundError):
            return None

    def loot_table(self, block):
        return self.read(f"data/minecraft/loot_table/blocks/{block.split(':')[1]}.json")

    def predicate(self, ref):
        ns, path = ref.split(":")
        p = self.read(f"data/{ns}/predicate/{path}.json")
        if p is None:
            raise Unknown(f"predicate {ref}")
        return p


def as_list(x):
    if x is None:
        return []
    return x if isinstance(x, list) else [x]


def number(n, one=False):
    if isinstance(n, (int, float)):
        return n
    if isinstance(n, dict) and n.get("type") == "minecraft:constant":
        return n["value"]
    if one:
        return 1
    raise Random(f"number {n}")


def matches_value(want, have):
    if isinstance(want, dict):
        v = int(have)
        return ("min" not in want or v >= int(want["min"])) and ("max" not in want or v <= int(want["max"]))
    return str(want) == have


def block_listed(blocks, block):
    blocks = as_list(blocks)
    if any(b.startswith("#") for b in blocks):
        raise Unknown(f"block tag in {blocks}")
    return block in blocks


class Roll:
    def __init__(self, src, block, state, one=False):
        # one: read a random count as 1, for a block planted from one of
        # what it drops a handful of (a melon stem gives 0-3 seeds).
        self.src, self.block, self.state, self.one = src, block, state, one

    def test(self, cond):
        """Whether a condition (or list of them, all needed) holds."""
        if isinstance(cond, list):
            return all(self.test(c) for c in cond)
        if isinstance(cond, str):
            return self.test(self.src.predicate(cond))
        t = cond["type"]
        if t == "minecraft:survives_explosion":
            return True
        if t == "minecraft:match_block":
            return block_listed(cond["blocks"], self.block) and all(
                matches_value(v, self.state[k]) for k, v in cond.get("state", {}).items()
            )
        if t == "minecraft:any_of":
            return any(self.test(c) for c in cond["terms"])
        if t == "minecraft:all_of":
            return all(self.test(c) for c in cond["terms"])
        if t == "minecraft:inverted":
            return not self.test(cond["term"])
        if t == "minecraft:match_tool":
            return self.tool(cond["predicate"])
        if t == "minecraft:entity_properties" and cond.get("predicate") == {}:
            return True  # a player broke it
        if t == "minecraft:location_check":
            return True  # the other half of a tall plant is there
        if t in ("minecraft:random_chance", "minecraft:table_bonus", "minecraft:random_chance_with_enchanted_bonus"):
            raise Random(t)
        raise Unknown(f"condition {t}")

    def tool(self, pred):
        ok = True
        for k, v in pred.items():
            if k == "items":
                ok &= TOOL["item"] in as_list(v)
            elif k == "predicates" and set(v) == {"minecraft:enchantments"}:
                for e in v["minecraft:enchantments"]:
                    if as_list(e.get("enchantments")) != ["minecraft:silk_touch"]:
                        raise Unknown(f"tool enchantment {e}")
                    ok &= TOOL["silk_touch"]
            else:
                raise Unknown(f"tool predicate {k}")
        return ok

    def modify(self, mods, items):
        """Apply item modifiers to `items` (a dict of id to count) in place."""
        for m in as_list(mods):
            if "condition" in m and not self.test(m["condition"]):
                continue
            t = m["type"]
            if t == "minecraft:set_count":
                n = number(m["count"], self.one)
                for k in items:
                    items[k] = items[k] + n if m.get("add") else n
            elif t == "minecraft:limit_count":
                lim = m["limit"]
                lo, hi = (number(lim.get("min", 0)), number(lim.get("max", 1 << 30))) if isinstance(lim, dict) else (lim, lim)
                for k in items:
                    items[k] = max(lo, min(hi, items[k]))
            elif t == "minecraft:apply_bonus":
                # The tool has no fortune: ore_drops and uniform_bonus_count
                # add nothing, a binomial bonus still rolls its extra tries.
                if m.get("formula") == "minecraft:binomial_with_bonus_count" and m["parameters"].get("extra", 0) > 0:
                    raise Random("binomial bonus")
            elif t in ("minecraft:explosion_decay", "minecraft:copy_components", "minecraft:copy_state"):
                pass  # no explosion; data on the item, not how many
            else:
                raise Unknown(f"modifier {t}")
        for k in [k for k, v in items.items() if v <= 0]:
            del items[k]

    def entry(self, e):
        """The items an entry gives, or None if its condition fails."""
        if "condition" in e and not self.test(e["condition"]):
            return None
        t = e["type"]
        if t == "minecraft:item":
            items = {e["name"]: 1}
            self.modify(e.get("modifier"), items)
            return items
        if t == "minecraft:alternatives":
            for c in e["children"]:
                got = self.entry(c)
                if got is not None:
                    return got
            return None
        if t == "minecraft:empty":
            return {}
        if t == "minecraft:dynamic":
            return {}  # a container's contents or a pot's sherds: counted elsewhere
        raise Unknown(f"entry {t}")

    def table(self, table):
        out = {}
        for pool in table.get("pools", []):
            if "condition" in pool and not self.test(pool["condition"]):
                continue
            if "bonus_rolls" in pool and number(pool["bonus_rolls"]) != 0:
                raise Random("bonus rolls")
            got = [g for g in map(self.entry, pool["entries"]) if g is not None]
            if len(got) > 1:
                raise Random("weighted entries")
            for _ in range(int(number(pool.get("rolls", 1)))):
                for g in got:
                    self.modify(pool.get("modifier"), g)
                    for k, v in g.items():
                        out[k] = out.get(k, 0) + v
        self.modify(table.get("modifier"), out)
        return out


def standing(block, blocks):
    """The standing block a wall variant takes its loot from."""
    name = block.split(":")[1]
    for cand in (name.replace("_wall_", "_"), name.replace("wall_", "")):
        if cand != name and f"minecraft:{cand}" in blocks:
            return f"minecraft:{cand}"
    return None


def states(props):
    keys = list(props)
    for values in itertools.product(*(props[k] for k in keys)):
        yield dict(zip(keys, values))


def needs(src, block, info, blocks, items):
    """Every state's item list, as (state, {item: count}) pairs."""
    own = block if block in items else None
    table = src.loot_table(block)
    if table is None and (alt := standing(block, blocks)):
        table, block_for_loot = src.loot_table(alt), alt
    else:
        block_for_loot = block
    rows = []
    noted = set()
    for state in states(info["properties"]):
        upper = state.get("half") == UPPER_HALF
        if block in OVERRIDES:
            got = OVERRIDES[block](state)
        elif block in FLUIDS:
            got = {FLUIDS[block]: 1} if state["level"] == "0" else {}
        elif table is None:
            if block in NO_TABLE_OWN_ITEM and own:
                got = {own: 1}
            elif block in NO_TABLE_NOTHING:
                got = {}
            else:
                raise Unknown(f"{block} has no loot table")
        elif upper:
            got = {}
        else:
            ignored = IGNORED | IGNORED_OF.get(block, set())
            rolled = {k: (info["default"][k] if k in ignored else v) for k, v in state.items()}
            # A wall variant's table tests its standing block's states.
            if block_for_loot != block:
                base = blocks[block_for_loot]
                rolled = {**base["default"], **{k: v for k, v in rolled.items() if k in base["properties"]}}
            try:
                got = Roll(src, block_for_loot, rolled).table(table)
            except Random as r:
                if own:
                    got = {own: 1}
                    noted.add(f"{block}: drops by chance, counted as one of its own item")
                else:
                    try:
                        got = {k: 1 for k in Roll(src, block_for_loot, rolled, one=True).table(table)}
                    except Random:
                        raise Unknown(f"{block} {state}: drops by chance ({r}) and has no item of its own")
                    noted.add(f"{block}: drops a random count, counted as one {', '.join(got)}")
        rows.append((state, got))

    # A block that drops nothing at all, even with silk touch (a spawner,
    # budding amethyst), is placed from its own item.
    if table is not None and block not in OVERRIDES and own and not any(got for _, got in rows):
        rows = [(state, {} if state.get("half") == UPPER_HALF else {own: 1}) for state, _ in rows]
        noted.add(f"{block}: drops nothing, counted as one of its own item")

    out = []
    for state, got in rows:
        for k, extra in EXTRA.items():
            if block == k or (not k.startswith("minecraft:") and block.endswith(k)):
                for item in extra:
                    got[item] = got.get(item, 0) + 1
        for item in got:
            if item not in items:
                raise Unknown(f"{block} {state} gives {item}, which is not an item")
        out.append((state, dict(sorted(got.items()))))
    for n in sorted(noted):
        print(f"    {n}")
    return out


def condense(props, rows):
    """The smallest set of properties the lists depend on, and a table by them."""
    keys = list(props)
    for size in range(len(keys) + 1):
        for by in itertools.combinations(keys, size):
            table = {}
            ok = True
            for state, got in rows:
                key = ",".join(state[k] for k in by)
                if table.setdefault(key, got) != got:
                    ok = False
                    break
            if ok:
                return list(by), table
    raise AssertionError("unreachable: all properties always determine the list")


def item_stack_sizes(reports):
    """Every item's stack size from the item report: one items.json in older
    versions, one file per item under <namespace>/.../item/ in newer ones."""
    def components(v):
        return v.get("components", v)

    single = reports / "items.json"
    if single.exists():
        report = json.loads(single.read_text())
        return {k if ":" in k else f"minecraft:{k}": components(v)["minecraft:max_stack_size"] for k, v in report.items()}
    stacks = {}
    for f in sorted(reports.glob("*/**/item/*.json")):
        ns = f.relative_to(reports).parts[0]
        v = components(json.loads(f.read_text()))
        if "minecraft:max_stack_size" in v:
            stacks[f"{ns}:{f.stem}"] = v["minecraft:max_stack_size"]
    if not stacks:
        dirs = sorted({str(d.relative_to(reports)) for d in reports.glob("*/*/*") if d.is_dir()})
        raise Unknown(f"no item report in {reports}; its directories: {' '.join(dirs[:40])}")
    return stacks


def main(jar, reports, out):
    src = Source(jar)
    out = Path(out)
    blocks = json.loads((out / "blocks.json").read_text())
    stacks = item_stack_sizes(Path(reports))
    items = set(stacks)

    rows = []
    for block, info in blocks.items():
        # Waterlogging is one rule for every block (see WATERLOGGED), not
        # part of each block's table.
        props = {k: v for k, v in info["properties"].items() if k != "waterlogged"}
        by, table = condense(props, needs(src, block, info, blocks, items))
        if by:
            entry = {"by": by, "default": ",".join(info["default"][k] for k in by), "cases": table}
        else:
            entry = {"items": table[""]}
        rows.append(f"  {json.dumps(block)}: {json.dumps(entry, separators=(',', ':'))}")
    odd = {k: v for k, v in sorted(stacks.items()) if v != 64}
    with open(out / "materials.json", "w") as f:
        f.write("{\n")
        f.write(f'"waterlogged": {json.dumps(WATERLOGGED)},\n')
        f.write('"stack_sizes": {\n')
        f.write(",\n".join(f"  {json.dumps(k)}: {v}" for k, v in odd.items()))
        f.write('\n},\n"blocks": {\n')
        f.write(",\n".join(rows))
        f.write("\n}\n}\n")
    print(f"    {len(rows)} blocks, {len(odd)} items that stack to other than 64 -> {out / 'materials.json'}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
