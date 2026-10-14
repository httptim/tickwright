# Tickwright

A tick-accurate simulator for Minecraft: Java Edition redstone, targeting
26.3. This repository holds the open-source parts: the simulation engine
and the test harness that checks it against the real game.

## How accuracy is measured

1. **Fixtures** (`fixtures/`) are small builds written as layered ASCII art,
   with stimuli such as "flip this lever at tick 2". Each one is captured in
   all eight rotations and mirrors, because default redstone dust behaves
   differently depending on world coordinates and direction.
2. **The capture harness** (`harness/`) runs every fixture in the real
   server, headless, and records a **trace**: every block change in the
   fixture's box, tagged with the tick and the tick phase it happened in.
3. **The engine** (`crates/engine`) runs the same fixtures and writes its
   own traces. `tickwright diff` compares the two and reports the first
   divergence. `cargo test -p tickwright-engine` runs every fixture against
   the committed game traces; every phase 1 fixture matches at the `order`
   level in all eight orientations.

Traces compare at three levels, each stricter than the last:

| Level   | Checks |
|---------|--------|
| `state` | every block's state at the end of every tick |
| `phase` | each block's changes within a tick, and the phase each happened in |
| `order` | every update, scheduled tick and block event, in the game's order |

The harness records all three. Comparator updates are not recorded yet;
phase 1 has no comparators.

## Layout

| Path | What it is |
|------|------------|
| `crates/trace` | Fixture, trace and diff formats (`tickwright-trace`) |
| `crates/engine` | The simulation engine (`tickwright-engine`) |
| `crates/schematic` | Reads and writes Litematica `.litematic`, Sponge `.schem` and structure `.nbt` files, and converts them to and from fixtures (`tickwright-schematic`) |
| `crates/materials` | Material lists: the items a build takes, with stack and shulker box counts, from `data/<version>/materials.json` (`tickwright-materials`) |
| `crates/wasm` | The engine as a WebAssembly module with a plain C ABI: fixture runs, JSON-command sessions for the editor (build, click, step, rewind, inspect, material list), and a fixture's material list |
| `crates/cli` | The `tickwright` command: `resolve`, `run`, `diff`, `bench`, `materials`, and `import`/`export` for schematics |
| `fixtures/phase1` | Phase 1 fixtures: dust, torches, repeaters, pistons, buttons |
| `fixtures/phase2` | Phase 2 fixtures: observers, comparators, note blocks, doors, trapdoors, rails, water, hoppers, droppers, dispensers, barrels, copper bulbs, bells |
| `fixtures/phase3` | Blocks placed as a player places them, written by `tools/placement_fixtures.py` |
| `fixtures/nucleation` | Builds from [Nucleation](https://github.com/Schem-at/Nucleation)'s conformance tests (MIT, © 2025 Schem-at), converted by `tools/nucleation.py` and recaptured on 26.3 |
| `fixtures/community` | Popular community builds, rebuilt from the Minecraft Wiki's schematics and credited to their designers in each fixture (see its README) |
| `harness/` | Java capture harness for the vanilla server |
| `traces/<version>/` | Ground-truth traces from the real game, gzipped |
| `data/<version>/` | Blocks, items and block entity types from the game's data reports, and `materials.json`: the items each block state takes and every item's stack size |
| `tools/registry.py` | Turns the game's data reports into `data/<version>/` |
| `tools/materials.py` | Rolls the server jar's block loot tables into `data/<version>/materials.json` |
| `tools/wasm-check.cjs` | Runs every fixture in headless Chromium, as a fixture run and as an editor session, and checks the traces match the native engine's; times the benchmark build both ways |

## Usage

```sh
# Validate fixtures and expand them into one file per orientation
cargo run -p tickwright -- resolve fixtures --out resolved

# Simulate resolved fixtures with the engine
cargo run -p tickwright -- run resolved --out traces-engine

# Compare two traces
cargo run -p tickwright -- diff expected.json actual.json --level phase

# Turn a schematic into a fixture, or a fixture's build into a schematic
cargo run -p tickwright -- import door.litematic --out door.json --ticks 100
cargo run -p tickwright -- export door.json --out door.schem

# The items a build takes, with stack and shulker box counts
cargo run -p tickwright -- materials door.litematic

# Ticks per second on a large dust-and-repeater build
cargo run --release -p tickwright -- bench
```

### Capturing traces

Needs Java 25 or newer, Rust, Python 3, curl and unzip.

```sh
harness/run.sh                      # every fixture
harness/run.sh --only torch_        # fixtures whose name contains "torch_"
```

The script downloads the official server jar from Mojang and checks its
SHA-1 against Mojang's manifest. The jar is never committed or
redistributed. Traces land in `harness/work/traces/`.

The `oracle` workflow runs the same script on GitHub Actions on every pull
request. It fails if the committed `traces/` or `data/` no longer match what
the game produces, which also catches a capture that is not deterministic.
After changing fixtures or the harness, run the workflow by hand with
`commit_data` to commit fresh copies to the branch.

## Writing a fixture

```json
{
  "name": "torch_inverter",
  "description": "A lever powers a block; the torch on its far side turns off.",
  "ticks": 10,
  "palette": {
    "#": "stone",
    "L": "lever[face=floor,facing=north,powered=false]",
    "T": "redstone_wall_torch[facing=east,lit=true]"
  },
  "layers": [
    ["##."],
    ["L#T"]
  ],
  "stimuli": [{ "tick": 2, "action": "use", "pos": [0, 1, 0] }]
}
```

- `layers[y][z]` is a row; each character is one block along x. `.` is air.
- Write block states in full. The harness places them exactly as written,
  without updates, so the build starts in the state you describe.
- Stimuli run just before their tick, where player input lands in the game.
  Actions are `use` (right-click with an empty hand), `set`, `destroy` and
  `place`: right-click a `face` of the block holding an `item`, as a player
  facing `look` and looking `pitch` degrees down (optionally at a `hit`
  point in sixteenths, or sneaking), so the block's own placement rules
  choose its state. A `set` or `destroy` with `"discard_contents": true` removes a
  block entity the way `/setblock` and the editor do: a container's items,
  a lectern's book and a jukebox's disc go with the block rather than
  spilling out as items.
- Fixtures are placed at `[0, 100, 0]` unless `origin` says otherwise.
- Every fixture is captured in all eight orientations unless `transforms`
  lists fewer. A fixture with `"entities": true` runs with the level random
  seeded from its run's id, so a build whose outcome is random (a TNT
  cannon) can add `"seeds": 100` to capture its first orientation 100 more
  times, as `<name>@r0~1` to `<name>@r0~100`, each drawing different
  numbers. That shows the spread without capturing every orientation 100
  times.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Minecraft is a trademark of Mojang AB. Tickwright is not affiliated with
Mojang or Microsoft.
