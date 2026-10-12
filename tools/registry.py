"""Condense the game's data reports into the registry files Tickwright uses.

    python3 tools/registry.py <reports dir> <output dir>

Reads blocks.json and registries.json from the server's --reports output and
writes, one entry per line so diffs between versions stay readable:

- blocks.json: every block with its properties (in the game's order, which is
  the order states are numbered in), its default state, and its numeric state
  id range.
- items.json: every item id in registry order.
- block_entities.json: every block entity type in registry order.
"""

import json
import sys
from pathlib import Path


def write_lines(path, header, rows, footer):
    with open(path, "w") as f:
        f.write(header + "\n")
        f.write(",\n".join(rows))
        f.write("\n" + footer + "\n")


def main(reports, out):
    reports, out = Path(reports), Path(out)
    out.mkdir(parents=True, exist_ok=True)

    blocks = json.loads((reports / "blocks.json").read_text())
    rows = []
    for block_id, b in blocks.items():
        states = b["states"]
        default = next(s for s in states if s.get("default"))
        entry = {
            "properties": b.get("properties", {}),
            "default": default.get("properties", {}),
            "first_state": min(s["id"] for s in states),
            "states": len(states),
        }
        rows.append(f"  {json.dumps(block_id)}: {json.dumps(entry, separators=(',', ':'))}")
    write_lines(out / "blocks.json", "{", rows, "}")

    registries = json.loads((reports / "registries.json").read_text())
    for name, file in [("minecraft:item", "items.json"), ("minecraft:block_entity_type", "block_entities.json")]:
        entries = registries[name]["entries"]
        ids = sorted(entries, key=lambda k: entries[k]["protocol_id"])
        write_lines(out / file, "[", [f"  {json.dumps(i)}" for i in ids], "]")

    print(f"    {len(blocks)} blocks, {len(registries['minecraft:item']['entries'])} items, "
          f"{len(registries['minecraft:block_entity_type']['entries'])} block entity types -> {out}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
