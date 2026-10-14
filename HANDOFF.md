# Engine handoff

Where the Tickwright engine stands as of 2026-10-12, what to do next, and
how to check your work. The [README](README.md) explains the layout, the
CLI and how to write fixtures. This file is the short version for someone
picking the work up.

## Status

- The engine targets Java Edition 26.3. Every behaviour is checked against
  traces captured from the real 26.3 server (`traces/26.3`).
- There are 269 fixtures (`fixtures/`) and 2231 traces: one per fixture
  and orientation, and 100 more seeds of `phase4/tnt_cannon`. Every trace
  matches at the strictest level ("order"), and no fixture is refused.
  The fixtures listed in `crates/engine/tests/expected.txt` must keep
  matching in every captured run.
- What is simulated:
  - **Redstone:** dust (with and without Redstone Experiments), torches,
    repeaters, comparators, observers, pistons and sticky pistons with
    their update order and quasi-connectivity, lamps, levers, buttons,
    note blocks, doors, trapdoors, copper bulbs, bells, target blocks,
    daylight detectors, lecterns, tripwire, and sculk and calibrated
    sculk sensors.
  - **Containers:** chests, barrels, hoppers, droppers, dispensers,
    crafters and shelves. Droppers and dispensers pick a filled slot with
    the level's random source, as the game does.
    Every item stacks as high as the game's item
    data says (64, 16 or 1, from `data/26.3/materials.json`), in
    containers and minecarts, in comparator readings, and when item
    entities merge. Explosion drops merge by the same rule in the game's
    code (`addOrAppendStack`), but no capture covers that yet.
  - **Item components:** a stack keeps its component patch (a custom
    name, say) through containers, entities, sessions and saved files, and
    stacks merge only when their patches match, as
    `isSameItemSameComponents` decides. The engine models no component
    itself; one that changes behaviour it simulates (`max_stack_size`,
    `damage_resistant`) marks the build unsupported, by name, as does an
    item `tag` from a file saved before 1.20.5.
  - **Rails:** plain, powered, activator and detector rails.
  - **Fluids:** water.
  - **Entities (phase 4):**
    - items, including those droppers, dispensers and crafters throw out
    - falling blocks
    - primed TNT and explosions
    - minecarts (plain, furnace, chest and hopper)
    - slime bounces and launches
    - moving blocks shoving entities, slime launching them, and honey
      carrying them sideways, flying machines included
    - honey blocks that entities slide down
    - items caught inside blocks working their way out
    - bubble columns
    - cobwebs
    - pressure plates
    - arrows
    - small fireballs, and the fire they light
  - **Known duplication glitches:** the TNT dupe and the rail and carpet
    dupes behave as they do in 26.3.
  - **Update skipping:** a chain of neighbor and shape updates drops what
    it queues past `max-chained-neighbor-updates` (1,000,000 unless a
    fixture lowers it), as the game's `CollectingNeighborUpdater` does.
- Mechanics the engine can't simulate yet are refused by name rather than
  guessed. A run stops with an "unsupported" message, or with
  `run --partial` it records the ticks it could simulate.
  `grep -rn 'unsupported("' crates/engine/src` lists every refusal.
- `fixtures/QUIRKS.md` tracks which known quirks have fixtures.
- `fixtures/community` holds popular community builds, rebuilt from the
  Minecraft Wiki's schematics and credited to their designers in each fixture; its
  README lists them.
- **Editor sessions** (`session.rs`, and the wasm `session` commands) keep
  a copy of the world at most every 32 ticks, every edit, and each tick's
  events packed to about a byte each (`record::Packer`). Any other tick is
  rebuilt by running the ticks after the copy before it again. On the
  benchmark build a session keeps 7 to 8 KB a tick (it was 369 KB), and a
  rewind takes at most about 11 ms in a browser. `step` with
  `"events": false` answers only whether anything happened. Known limit:
  a checkpoint also comes after 256 KB of packed events, so a build that
  packs that much in every tick (about 75 times the benchmark) is copied
  every tick, as before this change: no worse, but its memory still grows
  without bound.

## Next engine work, in order

1. **Lava.** Its spread delay draws from the level random, which the
   engine already reproduces. Add lava with source rules, cobblestone and
   stone generation, and items burning.
2. **Projectiles hitting entities and burning arrows.** Arrows and
   fireballs that would hit an entity, a bell or TNT are refused. Flaming
   arrows light TNT.
3. **TNT minecarts.** Their explosion power depends on speed, and the
   activator rail primes them.
4. **Fire beyond fireballs.** Fire on flammable blocks, and fire beside
   obsidian (portals), are refused. Captures never spread fire because no
   player is nearby, so the engine only reschedules it.
5. **Performance.** The bench went from about 540 to more than 2000 ticks
   a second. The next hotspots are `Registry::value` name comparisons,
   wire `update_power_strength` and `run_updates`. Profile with
   `valgrind --tool=callgrind target/release/tickwright bench`, because
   wall-clock timings are noisy.
6. **Minecart Improvements** (an opt-in experiment in 26.3). The engine
   runs the default `OldMinecartBehavior` only.
7. **New game versions.** Add a rule module under
   `crates/engine/src/rules/` layered on 26.3, set `MC_VERSION` in the
   oracle workflow, and capture a new `traces/<version>`.

## Running the conformance tests

You need Rust stable. Captures also need Java 25 and network access to
Mojang's servers.

```sh
# Everything, including conformance
cargo test --release --workspace

# The conformance table: one row per fixture, orientations matching
cargo test --release -p tickwright-engine --test conformance -- --nocapture

# One fixture, or the first divergence for one orientation
TICKWRIGHT_ONLY=arrow_hits cargo test --release -p tickwright-engine --test conformance -- --nocapture
TICKWRIGHT_SHOW=arrow_hits@r0 cargo test --release -p tickwright-engine --test conformance -- --nocapture

# Lint and format, as CI runs them
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Orientations are `r0`, `r90`, `r180` and `r270`, and their mirrored forms
`mr0` to `mr270`.

## Capturing new traces

Traces only come from the real game. Never write or edit one by hand.

1. Add a fixture JSON under `fixtures/<phase>/`, then validate it with
   `cargo run --release -p tickwright -- resolve <file> --out /tmp/res`.
2. Push the branch, then run the `oracle` workflow on it with
   `commit_data=true`:
   ```sh
   gh workflow run oracle.yml --ref <branch> -f commit_data=true
   ```
   It downloads the official 26.3 server jar from Mojang's version
   manifest and checks its SHA-1 (`harness/run.sh`). The jar is never
   committed. The workflow then captures every fixture with the agent in
   `harness/` and commits the traces back to the branch.
3. Pull the bot's commit, then make your engine change match the traces.
   `-f only=<name>` captures a subset when you are debugging, and does not
   commit.

On `main` and on pull requests, the oracle job fails if a fresh capture
differs from the committed traces. That keeps them honest.

## Gotchas

- **Determinism:** entity fixtures depend on the harness reseeding the
  level random, resetting entity ids, turning spawners, weather and
  random ticks off, and waiting for lighting to settle. Captures repeat
  byte for byte. If one stops repeating, look for a new source of
  randomness in the game before touching the engine.
- **Write the vanilla code path:** match its order of operations,
  float32 constants (`0.98f32 as f64`) and its use of the level random.
  When a rule is unknown, capture a fixture that sweeps the input and fit
  the rule from the trace.
- **Summon positions:** the summon command centres whole-number x and z,
  so `[1, y, 1]` means `[1.5, y, 1.5]`. Write positions as short exact
  decimals; a whole number also turns wrongly, since it is mirrored before
  the game centres it.
- **Trigonometry:** where the game calls `Math.sin` or `Math.cos` (the
  kick of newly primed TNT), use `crate::math`, which rounds correctly as
  the captures do. The platform's `sin` is an ulp off in a few cases.
- **Check the capture:** make sure a new fixture really does what its
  description says. It's easy to aim an arrow or drop an item that never
  touches the block under test.
- **Session edits must replay:** a session rebuilds past ticks by running
  them again with the edits it logged. A new session command that changes
  the world must go through `Session::edit` with an `Input`, never change
  the level directly, or a rewind will quietly lose it. In tests and debug
  builds each new checkpoint is checked against the one before it run
  again with the logged edits, which panics on such an edit. The session
  tests also check every fixture against a session that keeps every tick.
- **Blocks an entity passes** feel it by the game's own walk
  (`entity::for_each_block_between`, a port of
  `forEachBlockIntersectedBetween`), one axis at a time for the entity's
  own moves, the vertical leg first, at most 16 steps of it all
  (`tnt_fast_walk_limit`, `tnt_fast_walk_limit_rising`). A block is felt
  only if the box sweeps through its `getEntityInsideCollisionShape`
  (`entity::moved_into`): the whole block for everything the engine
  simulates but tripwire, whose thin string is traced
  (`tripwire_item_falls_through`), and powder snow, whose shape depends on
  how far the entity fell and which refuses any entity it feels. A block
  that is missed doesn't count as visited. No capture can show that, as
  the simulated entities make one move before they feel blocks and a
  missed string stays missed for the rest of that move; a unit test
  (`entity::tests`) checks it. Captures also prove the paths that used to
  follow the game's code unproven: a minecart feels the blocks twice on
  every piston push, even one the 0.51 limit cut to nothing, and other
  entities once (`piston_push_two_pistons_bubble_column`); an item without
  physics is pushed past the limit (`piston_push_item_no_physics`); and a
  moving piston that fills its block counts as full for an item stuck next
  to it (`item_stuck_beside_moving_piston`).
- **Piston push reactions** are a list by kind (`piston::push_reaction`),
  not data. 26.3 calls breaking a block in a piston's way `POPPED`
  (`DESTROY` before). When a block gets a kind, check its reaction in the
  game (`BlockState.getPistonPushReaction`): torches, pressure plates and
  moss carpet were pushed until `piston_pops_torches_and_plates`. A block
  a piston moves arrives dry: the game clears `waterlogged` when it places
  it (`piston_moves_waterlogged_blocks`), but not when a moving block is
  finished early (`finalTick`).
- **Entity moves allocate nothing** except next to moving pistons:
  walks and collisions visit blocks through callbacks, and the lists they
  need are buffers kept on the level (`entity::Scratch`) or on the
  entity. A build of 576 items runs 1.8 times as fast for it. A moving
  piston's shape (`moving_collision_shape`, and
  `collision_shape_full_block`, which reads it) is still built per call.
  Keep new per-tick entity code free of allocations too.
- **The entity blocks see:** an entity is out of the level's list while
  it ticks or a piston moves it. Take it out with `Level::take_entity`
  and put it back with `Level::put_entity`. Its slot meanwhile holds an
  empty entity that counts as removed, so no query finds it (a plain
  `std::mem::take` leaves one at the origin that plates and tripwire
  would feel). `Level::with_listed` puts the entity back in its own slot
  while blocks and containers look at it, and block handlers get that
  index. Entities a block spawns meanwhile, such as TNT a pressure plate
  lights, go on the end of the list. Don't read `entities.last()` for
  the entity a block feels.
- **Item entities only exist where entities are simulated:** a fixture
  with `"entities": true` (or a session built from one). Elsewhere a
  dropper firing into the air, or a block broken with drops, loses the
  item, as before. A dropper or dispenser picks its slot with the level's
  random source in both, but the harness seeds that source only for
  entity fixtures, so a build that fires one with several filled slots
  can only be captured as an entity fixture.
- **Editor edits remove block entities as `/setblock` does:** session
  edits add `flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS` (`set_block` logs it
  with the edit; a break uses `Level::destroy_block_with`), so a
  container's items and a lectern's book go with the block. A fixture
  stimulus says the same with `"discard_contents": true`, which the
  harness maps to the same flag. Without it, a removed container spills
  what it holds (`blocks::pre_remove_side_effects`, a port of
  `Containers.dropContents`): every slot, even an empty one, draws from
  the level's random source, and a lectern removed with its book in an
  entity run is refused.
- **Some refused cases are close to supported:** an arrow is refused once
  the block it is stuck in goes, a lectern once it is removed with its
  book, and a dispenser firing an item whose own behaviour the engine
  lacks (TNT a hopper brought in, say). The full list is in the code.
