"""Write the fixtures in fixtures/phase3 that place blocks as a player.

    python3 tools/placement_fixtures.py

Each fixture is a stone floor (and a few pillars) with `place` stimuli, one
per tick; the layers are generated so the coordinates in the stimuli stay
easy to read here.
"""
import json, sys
def build(name, desc, size, solids, stimuli, ticks, tags):
    sx, sy, sz = size
    grid = [[['.']*sx for _ in range(sz)] for _ in range(sy)]
    for (x,y,z) in solids:
        grid[y][z][x] = '#'
    st = []
    for i, s in enumerate(stimuli):
        d = {"tick": s.get("tick", i + 1), "action": "place", "pos": list(s["pos"]), "item": s["item"], "face": s.get("face", "up"), "look": s.get("look", "north")}
        for k in ("pitch", "hit", "sneak"):
            if k in s: d[k] = s[k]
        st.append(d)
    f = {"name": name, "description": desc, "ticks": ticks, "levels": ["state","phase","order"], "tags": tags,
         "palette": {"#": "stone"}, "layers": [["".join(r) for r in layer] for layer in grid], "stimuli": st}
    json.dump(f, open(f"fixtures/phase3/{name}.json", "w"), indent=2)
    open(f"fixtures/phase3/{name}.json", "a").write("\n")

floor = lambda sx, sz: [(x,0,z) for x in range(sx) for z in range(sz)]

build("place_diodes_and_dust",
  "Repeaters, a comparator, dust and redstone torches placed by a player: diodes face away from the look direction, dust connects to what is beside, above and below it, and a torch clicked onto a wall becomes a wall torch.",
  (7,3,5), floor(7,5) + [(3,1,4)], [
    dict(pos=(1,0,1), item="repeater", look="north", pitch=30),
    dict(pos=(1,0,2), item="redstone", look="north", pitch=60),
    dict(pos=(2,0,2), item="redstone", look="east", pitch=45),
    dict(pos=(3,0,2), item="comparator", look="east", pitch=30),
    dict(pos=(3,1,4), item="redstone", look="south", pitch=50),
    dict(pos=(3,0,3), item="redstone", look="south", pitch=50),
    dict(pos=(3,1,4), item="redstone_torch", face="west", look="east", pitch=10),
    dict(pos=(5,0,3), item="redstone_torch", look="north", pitch=45),
    dict(pos=(5,0,1), item="repeater", look="west", pitch=20),
    dict(pos=(6,0,4), item="redstone", look="west", pitch=40),
  ], 20, ["placement","dust","repeater","comparator","torch"])

build("place_levers_and_buttons",
  "Levers and buttons placed on floors, walls and a ceiling: the clicked face decides the attachment and the look direction the facing.",
  (7,4,5), floor(7,5) + [(x,y,2) for x in (0,2,4) for y in (1,2)] + [(6,3,4)], [
    dict(pos=(0,1,2), item="lever", face="east", look="west"),
    dict(pos=(2,2,2), item="stone_button", face="south", look="north", pitch=10),
    dict(pos=(4,0,1), item="lever", face="up", look="south", pitch=60),
    dict(pos=(6,3,4), item="lever", face="down", look="north", pitch=-60),
    dict(pos=(6,3,4), item="oak_button", face="north", look="south", pitch=-20),
    dict(pos=(0,1,2), item="lever", face="north", look="south"),
    dict(pos=(4,0,3), item="stone_button", face="up", look="east", pitch=80),
    dict(pos=(6,0,0), item="lever", face="up", look="west", pitch=0),
  ], 12, ["placement","lever","button"])

build("place_pistons_and_observers",
  "Pistons, observers, droppers, dispensers, hoppers and a barrel placed looking level, up and down; a piston placed beside a redstone block extends at once and pushes an observer.",
  (7,3,7), floor(7,7), [
    dict(pos=(1,0,1), item="piston", look="north", pitch=0),
    dict(pos=(3,0,1), item="sticky_piston", look="north", pitch=80),
    dict(pos=(5,0,1), item="observer", look="east", pitch=0),
    dict(pos=(1,0,3), item="observer", look="south", pitch=70),
    dict(pos=(3,0,3), item="dropper", look="west", pitch=-70),
    dict(pos=(5,0,3), item="dispenser", look="north", pitch=20),
    dict(pos=(5,1,1), item="hopper", face="west", look="east", pitch=10),
    dict(pos=(3,0,5), item="hopper", look="north", pitch=60),
    dict(pos=(1,0,5), item="redstone_block", look="north", pitch=60),
    dict(pos=(1,1,5), item="piston", face="north", look="south", pitch=0),
    dict(pos=(5,0,5), item="barrel", look="north", pitch=30),
  ], 24, ["placement","piston","observer","hopper","dropper"])

build("place_doors_trapdoors_slabs",
  "Doors (single, paired and powered), trapdoors clicked high and low on a wall and on the floor, and slabs placed bottom, top and merged into a double slab.",
  (7,4,6), floor(7,6) + [(4,1,3),(4,2,3)], [
    dict(pos=(1,0,1), item="oak_door", look="north", pitch=30),
    dict(pos=(2,0,1), item="oak_door", look="north", pitch=30),
    dict(pos=(4,1,3), item="oak_trapdoor", face="west", look="east", hit=[0,12,8]),
    dict(pos=(4,2,3), item="oak_trapdoor", face="east", look="west", hit=[16,4,8]),
    dict(pos=(6,0,1), item="oak_trapdoor", look="south", pitch=50),
    dict(pos=(1,0,4), item="stone_slab", look="north", pitch=50),
    dict(pos=(1,1,4), item="stone_slab", look="north", pitch=60),
    dict(pos=(4,1,3), item="stone_slab", face="south", look="north", hit=[8,12,16]),
    dict(pos=(6,0,4), item="redstone_block", look="west", pitch=50),
    dict(pos=(5,0,4), item="oak_door", look="east", pitch=30),
    dict(pos=(3,0,1), item="iron_trapdoor", look="west", pitch=50, hit=[8,16,4]),
  ], 20, ["placement","door","trapdoor","slab"])

build("place_rails_and_more",
  "A powered rail and an activator rail, a powered rail facing east, lamps and a copper bulb placed beside power, a note block, a log on its side, two chests that join, leaves, a tripwire hook and a bell.",
  (7,3,7), floor(7,7), [
    dict(pos=(1,0,1), item="powered_rail", look="north", pitch=50),
    dict(pos=(2,0,1), item="activator_rail", look="north", pitch=50),
    dict(pos=(4,0,1), item="powered_rail", look="east", pitch=50),
    dict(pos=(4,0,3), item="redstone_lamp", look="north", pitch=50),
    dict(pos=(5,0,3), item="redstone_block", look="north", pitch=50),
    dict(pos=(6,0,3), item="redstone_lamp", look="north", pitch=50),
    dict(pos=(5,0,4), item="waxed_copper_bulb", look="north", pitch=50),
    dict(pos=(1,0,3), item="note_block", look="north", pitch=50),
    dict(pos=(1,1,3), item="oak_log", face="east", look="west"),
    dict(pos=(1,0,5), item="chest", look="north", pitch=40),
    dict(pos=(2,0,5), item="chest", look="north", pitch=40),
    dict(pos=(3,0,5), item="oak_leaves", look="north", pitch=50),
    dict(pos=(4,1,3), item="tripwire_hook", face="south", look="north"),
    dict(pos=(6,0,1), item="bell", look="north", pitch=40),
    dict(pos=(5,0,6), item="target", look="north", pitch=40),
  ], 24, ["placement","rail","lamp","copper_bulb","note_block","chest","bell"])
