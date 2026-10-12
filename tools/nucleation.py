#!/usr/bin/env python3
"""Convert Nucleation's corpus of captured builds into Tickwright fixtures.

Nucleation (https://github.com/Schem-at/Nucleation, MIT, (c) 2025 Schem-at)
keeps Java structure SNBT for each build it checked against the game, and
the inputs it applied are written in its conformance tests. Only the builds
and inputs are taken: the traces are recaptured on our target version by
our own harness, so the ground truth stays ours.

    python3 tools/nucleation.py <nucleation checkout> fixtures/nucleation

Builds with entities, block entity data, or inputs this format cannot
express are skipped and listed.
"""
import json
import re
import sys
from pathlib import Path

CHARS = "#ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@$%&*+=?!<>^~"


def parse_snbt(text):
    i = 0

    def ws():
        nonlocal i
        while i < len(text) and text[i] in " \t\r\n":
            i += 1

    def value():
        nonlocal i
        ws()
        c = text[i]
        if c == "{":
            i += 1
            out = {}
            ws()
            if text[i] == "}":
                i += 1
                return out
            while True:
                ws()
                k = key()
                ws()
                assert text[i] == ":", text[i - 20 : i + 20]
                i += 1
                out[k] = value()
                ws()
                if text[i] == ",":
                    i += 1
                    continue
                assert text[i] == "}"
                i += 1
                return out
        if c == "[":
            i += 1
            ws()
            # Typed arrays: [I; 1, 2]
            m = re.match(r"[BIL];", text[i:])
            if m:
                i += 2
            out = []
            ws()
            if text[i] == "]":
                i += 1
                return out
            while True:
                out.append(value())
                ws()
                if text[i] == ",":
                    i += 1
                    continue
                assert text[i] == "]"
                i += 1
                return out
        if c in "\"'":
            return string()
        m = re.match(r"[^,\]}\s]+", text[i:])
        i += m.end()
        tok = m.group(0)
        n = re.fullmatch(r"(-?\d+)[bBsSlL]?", tok)
        if n:
            return int(n.group(1))
        f = re.fullmatch(r"(-?\d*\.?\d+(?:[eE]-?\d+)?)[fFdD]?", tok)
        if f:
            return float(f.group(1))
        return {"true": True, "false": False}.get(tok, tok)

    def string():
        nonlocal i
        q = text[i]
        i += 1
        out = []
        while text[i] != q:
            if text[i] == "\\":
                i += 1
            out.append(text[i])
            i += 1
        i += 1
        return "".join(out)

    def key():
        nonlocal i
        if text[i] in "\"'":
            return string()
        m = re.match(r"[A-Za-z0-9_.+-]+", text[i:])
        i += m.end()
        return m.group(0)

    v = value()
    return v


def state_string(entry):
    name = entry["Name"].removeprefix("minecraft:")
    props = entry.get("Properties") or {}
    if not props:
        return name
    return name + "[" + ",".join(f"{k}={v}" for k, v in sorted(props.items())) + "]"


def runs(conformance):
    """Each conformance test: the structure, the golden name and the inputs."""
    calls = []
    for m in re.finditer(r"run_conformance\w*\(\s*\"([^\"]+)\.snbt\",\s*\"([^\"]+)\.json\"", conformance):
        # The call runs to the matching close paren.
        depth, j = 0, m.start()
        while True:
            c = conformance[j]
            depth += c == "("
            depth -= c == ")"
            j += 1
            if c == ")" and depth == 0:
                break
        body = conformance[m.start() : j]
        actions = []
        for a in re.finditer(r"\(\s*(\d+),\s*Actuate::(Use|Place)\(\s*Pos::new\(([^)]*)\)\s*(?:,\s*\"([^\"]+)\")?\s*\)\s*,?\s*\)", body):
            pos = [int(v) for v in a.group(3).replace(" ", "").split(",")]
            actions.append((int(a.group(1)), a.group(2), pos, a.group(4)))
        expected = len(re.findall(r"Actuate::", body))
        settle = re.search(r"Settle::(\w+)", body)
        # The test's own description: the comment block just above `#[test]`'s fn.
        head = conformance[: m.start()]
        fn = head.rfind("fn ")
        start = head.rfind("#[test]", 0, fn)
        comment = re.findall(r"^\s*// ?(.*)$", head[start:], re.M)
        calls.append(
            {
                "structure": m.group(1),
                "golden": m.group(2),
                "actions": actions,
                "complete": expected == len(actions),
                "settle": settle.group(1) if settle else "Placement",
                "comment": " ".join(comment).strip(),
            }
        )
    return calls


def convert(root, out):
    tests = root / "crates/mc-tick/tests"
    conformance = (tests / "conformance.rs").read_text()
    dirs = [tests / "corpus/structures", root / "tools/gametest/pack/data/nucleation/structure"]
    written, skipped = [], []
    seen = set()
    for run in runs(conformance):
        name = run["golden"]
        if name in seen:
            continue
        seen.add(name)
        path = next((d / f"{run['structure']}.snbt" for d in dirs if (d / f"{run['structure']}.snbt").exists()), None)
        if path is None:
            skipped.append((name, "structure not found"))
            continue
        s = parse_snbt(path.read_text())
        if s.get("entities"):
            skipped.append((name, "has entities"))
            continue
        if any("nbt" in b for b in s["blocks"]):
            skipped.append((name, "has block entity data"))
            continue
        if not run["complete"]:
            skipped.append((name, "inputs not understood"))
            continue
        golden = tests / "traces" / f"{name}.json"
        last = 0
        if golden.exists():
            g = json.loads(golden.read_text())
            last = max((t["tick"] for t in g["ticks"]), default=0)
        last = max([last] + [a[0] for a in run["actions"]])

        palette = [state_string(p) for p in s["palette"]]
        sx, sy, sz = s["size"]
        # Some inputs land just outside the saved structure.
        for _, _, (x, y, z), _ in run["actions"]:
            sx, sy, sz = max(sx, x + 1), max(sy, y + 1), max(sz, z + 1)
        grid = [[["."] * sx for _ in range(sz)] for _ in range(sy)]
        keys, chars = {}, iter(CHARS)
        for b in s["blocks"]:
            st = palette[b["state"]]
            if st in ("air", "structure_void"):
                continue
            if st not in keys:
                keys[st] = next(chars)
            x, y, z = b["pos"]
            grid[y][z][x] = keys[st]

        stimuli = []
        if run["settle"] == "Placement" and not run["actions"]:
            # Nucleation placed these with block updates, which is what set
            # them going. We place quietly, so the redstone blocks that power
            # them are placed as the first input instead.
            at = {tuple(b["pos"]): palette[b["state"]] for b in s["blocks"]}

            def drives(p):
                # Placing the block only updates its six neighbors, so it
                # stands in for placement only if one of them reacts.
                for d in ((1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)):
                    n = at.get((p[0] + d[0], p[1] + d[1], p[2] + d[2]), "air").split("[")[0]
                    if n not in ("air", "stone", "redstone_block") and not n.endswith("_concrete"):
                        return True
                return False

            sources = [b["pos"] for b in s["blocks"] if palette[b["state"]] == "redstone_block"]
            if not sources or not any(drives(p) for p in sources):
                skipped.append((name, "only runs from placement updates"))
                continue
            for x, y, z in sources:
                grid[y][z][x] = "."
                stimuli.append({"tick": 1, "action": "set", "pos": [x, y, z], "state": "redstone_block"})
            keys = {k: v for k, v in keys.items() if any(v in row for layer in grid for row in layer)}
        for tick, kind, pos, state in run["actions"]:
            st = {"tick": tick + 1, "pos": pos}
            if kind == "Use":
                st["action"] = "use"
            elif state == "minecraft:air":
                st["action"] = "destroy"
            else:
                st["action"] = "set"
                st["state"] = state.removeprefix("minecraft:")
            stimuli.append({"tick": st["tick"], "action": st["action"], "pos": st["pos"], **({"state": st["state"]} if "state" in st else {})})

        desc = run["comment"] or f"Nucleation's {name} build."
        settle = ""
        if run["settle"] == "Placement" and not run["actions"]:
            settle = " Nucleation's placement updates set it going; here its redstone blocks are placed on the first tick instead."
        fixture = {
            "name": f"nucleation_{name}",
            "description": f"{desc} (From Nucleation, MIT, (c) 2025 Schem-at.{settle})",
            "ticks": last + 12,
            "levels": ["state", "phase", "order"],
            "tags": ["nucleation"],
            "palette": {v: k for k, v in keys.items()},
            "layers": [["".join(row) for row in layer] for layer in grid],
            "stimuli": stimuli,
        }
        (out / f"{name}.json").write_text(json.dumps(fixture, indent=2) + "\n")
        written.append(name)
    return written, skipped


if __name__ == "__main__":
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    written, skipped = convert(root, out)
    print(f"wrote {len(written)} fixtures")
    for name, why in skipped:
        print(f"skipped {name}: {why}")
