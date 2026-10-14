"""Item components the engine reads, from the game's item report.

    python3 -I tools/item_components.py <server jar> <reports dir> <out dir>

The reports directory is the `--reports` output, which gives every item's
default components under minecraft/components/item/. The server jar (the
inner one the bundler unpacks) gives the damage type tags. Writes
<out dir>/item_components.json:

- "damage_resistant": every item whose default components include
  `minecraft:damage_resistant`, with the damage types it resists, tags
  expanded and sorted. An item entity holding one isn't hurt by those
  damage types (`ItemStack.canBeHurtBy`): a nether star by explosions,
  netherite by fire and lava.
"""

import json
import sys
import zipfile
from pathlib import Path

TAGS = "data/minecraft/tags/damage_type/"


def damage_types(jar, spec, seen=()):
    """The damage types a `types` value names: one id, a list, or a tag."""
    if isinstance(spec, list):
        out = set()
        for s in spec:
            out |= damage_types(jar, s, seen)
        return out
    if not spec.startswith("#"):
        return {spec if ":" in spec else f"minecraft:{spec}"}
    name = spec[1:]
    ns, path = name.split(":", 1) if ":" in name else ("minecraft", name)
    if ns != "minecraft" or name in seen:
        raise SystemExit(f"cannot expand damage type tag {spec}")
    tag = json.loads(jar.read(f"{TAGS}{path}.json"))
    out = set()
    for v in tag["values"]:
        v = v["id"] if isinstance(v, dict) else v
        out |= damage_types(jar, v, seen + (name,))
    return out


def main(jar_path, reports, out):
    jar = zipfile.ZipFile(jar_path)
    items = sorted(Path(reports).glob("*/components/item/*.json"))
    if not items:
        raise SystemExit(f"no item component report in {reports}")
    resistant = {}
    for f in items:
        ns = f.relative_to(reports).parts[0]
        c = json.loads(f.read_text())
        c = c.get("components", c)
        if "minecraft:damage_resistant" in c:
            resistant[f"{ns}:{f.stem}"] = sorted(damage_types(jar, c["minecraft:damage_resistant"]["types"]))
    rows = [f"  {json.dumps(k)}: {json.dumps(v)}" for k, v in sorted(resistant.items())]
    with open(Path(out) / "item_components.json", "w") as o:
        o.write('{\n"damage_resistant": {\n' + ",\n".join(rows) + "\n}\n}\n")
    print(f"    {len(items)} items, {len(resistant)} damage resistant -> {Path(out) / 'item_components.json'}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
