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
| `crates/wasm` | The engine as a WebAssembly module with a plain C ABI |
| `crates/cli` | The `tickwright` command: `resolve`, `run`, `diff` and `bench` |
| `fixtures/phase1` | Phase 1 fixtures: dust, torches, repeaters, pistons, buttons |
| `fixtures/nucleation` | Builds from [Nucleation](https://github.com/Schem-at/Nucleation)'s conformance tests (MIT, © 2025 Schem-at), converted by `tools/nucleation.py` and recaptured on 26.3 |
| `harness/` | Java capture harness for the vanilla server |
| `traces/<version>/` | Ground-truth traces from the real game, gzipped |
| `data/<version>/` | Blocks, items and block entity types from the game's data reports |
| `tools/registry.py` | Turns the game's data reports into `data/<version>/` |
| `tools/wasm-check.cjs` | Runs every fixture in headless Chromium and checks the traces match the native engine's |

## Usage

```sh
# Validate fixtures and expand them into one file per orientation
cargo run -p tickwright -- resolve fixtures --out resolved

# Simulate resolved fixtures with the engine
cargo run -p tickwright -- run resolved --out traces-engine

# Compare two traces
cargo run -p tickwright -- diff expected.json actual.json --level phase

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
  Actions are `use` (right-click with an empty hand), `set` and `destroy`.
- Fixtures are placed at `[0, 100, 0]` unless `origin` says otherwise.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Minecraft is a trademark of Mojang AB. Tickwright is not affiliated with
Mojang or Microsoft.
