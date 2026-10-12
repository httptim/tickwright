"""Condense the game's crafting recipes and item tags for the crafter.

    python3 -I tools/recipes.py <server jar> <out dir>

Reads data/minecraft/recipe/*.json and data/minecraft/tags/item/*.json from
the server jar and writes <out dir>/recipes.json: the shaped and shapeless
crafting recipes, in the order the game's recipe map lists them (by id), and
every item tag resolved to its items. Other recipe types (the special
crafting recipes, smelting, stonecutting and so on) are counted but left out.
"""

import json
import sys
import zipfile
from collections import Counter

jar, out_dir = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(jar)

RECIPES = "data/minecraft/recipe/"
TAGS = "data/minecraft/tags/item/"

raw_tags = {}
for name in z.namelist():
    if name.startswith(TAGS) and name.endswith(".json"):
        tag = "minecraft:" + name[len(TAGS):-5]
        raw_tags[tag] = json.loads(z.read(name))["values"]


def resolve(tag, seen=()):
    items = []
    for v in raw_tags[tag]:
        if isinstance(v, dict):  # {"id": ..., "required": false}
            v = v["id"]
        if v.startswith("#"):
            if v[1:] in raw_tags and v[1:] not in seen:
                items += resolve(v[1:], seen + (tag,))
        elif v not in items:
            items.append(v)
    return items


tags = {t: resolve(t) for t in sorted(raw_tags)}


def ingredient(v):
    """An ingredient as a list of items or a single tag reference."""
    if isinstance(v, list):
        return [x for x in v]
    return v


skipped = Counter()
recipes = []
for name in sorted(n for n in z.namelist() if n.startswith(RECIPES) and n.endswith(".json")):
    rid = "minecraft:" + name[len(RECIPES):-5]
    r = json.loads(z.read(name))
    kind = r["type"].removeprefix("minecraft:")
    if kind == "crafting_shaped":
        recipes.append({
            "id": rid,
            "type": "shaped",
            "pattern": r["pattern"],
            "key": {k: ingredient(v) for k, v in sorted(r["key"].items())},
            "symmetrical": r.get("symmetrical"),
            "result": {"id": r["result"]["id"], "count": r["result"].get("count", 1)},
        })
    elif kind == "crafting_shapeless":
        recipes.append({
            "id": rid,
            "type": "shapeless",
            "ingredients": [ingredient(v) for v in r["ingredients"]],
            "result": {"id": r["result"]["id"], "count": r["result"].get("count", 1)},
        })
    else:
        skipped[kind] += 1

for r in recipes:
    if r.get("symmetrical") is None:
        r.pop("symmetrical", None)

used = set()
for r in recipes:
    for ing in list(r.get("key", {}).values()) + r.get("ingredients", []):
        if isinstance(ing, str) and ing.startswith("#"):
            used.add(ing[1:])

out = {
    "recipes": recipes,
    "tags": {t: tags[t] for t in sorted(used) if t in tags},
    "skipped": dict(sorted(skipped.items())),
}
with open(f"{out_dir}/recipes.json", "w") as f:
    json.dump(out, f, indent=1, sort_keys=False)
    f.write("\n")
print(f"    {len(recipes)} crafting recipes, {len(out['tags'])} tags; skipped {dict(skipped)}")
