#!/usr/bin/env bash
# Load the engine's schematic exports in the real game and check that each
# places what it was written from, block for block (SchematicCheck.java):
#
# - every fixture, unturned, exported as .nbt, .schem and .litematic;
# - harness/schematics: builds whose entities carry block states;
# - a structure the game saves itself, imported by the engine and written
#   again in every format at data versions on both sides of 5006, where the
#   game started saving block states as `id` and `properties`.
#
# The structure the game saves must also import as the committed copy in
# crates/schematic/tests/data does.
#
#   harness/schematics.sh
#
# harness/run.sh calls this at the end of a full capture; it also runs on
# its own. Needs Java 25+, cargo, python3, curl and unzip.
set -euo pipefail
cd "$(dirname "$0")/.."

MC_VERSION="${MC_VERSION:-26.3}"
WORK="${WORK:-harness/work}"
mkdir -p "$WORK"
. harness/server.sh
rm -rf "$WORK/schematic-classes"
javac -nowarn -cp "$CP" -d "$WORK/schematic-classes" harness/src/tickwright/harness/SchematicCheck.java
CP="$WORK/schematic-classes:$CP"
TW=target/release/tickwright
cargo build --quiet --release -p tickwright
cargo build --quiet --release -p tickwright-schematic --example rewrite
CASES="$WORK/schematics"
# The game's log prefix, off what the check prints.
message() { sed -E 's/^\[[^]]*\] \[[^]]*\]: (\[STD(OUT|ERR)\]: )?//'; }
rm -rf "$CASES" "$WORK/schematic-fixtures"
mkdir -p "$CASES"

echo "==> exporting fixtures"
"$TW" resolve fixtures --out "$WORK/schematic-fixtures" >/dev/null
"$TW" resolve harness/schematics --out "$WORK/schematic-fixtures" >/dev/null
for f in "$WORK"/schematic-fixtures/*@r0.json; do
    name="$(basename "$f" @r0.json)"
    mkdir -p "$CASES/$name"
    cp "$f" "$CASES/$name/expected.json"
    for ext in nbt schem litematic; do
        "$TW" export "$f" --out "$CASES/$name/export.$ext" >/dev/null
    done
done

echo "==> saving a structure in Minecraft $MC_VERSION"
java -cp "$CP" tickwright.harness.SchematicCheck --universe "$WORK/universe-schematics" \
    --save "$WORK/game_saved.nbt" 2>&1 | message | grep -E '^(Minecraft|saved|Exception|\s+at )' || true
"$TW" import "$WORK/game_saved.nbt" --name game_saved --out "$WORK/game_saved.json"
"$TW" import "crates/schematic/tests/data/game_saved_$MC_VERSION.nbt" --name game_saved \
    --out "$WORK/game_saved_committed.json" >/dev/null
if ! diff "$WORK/game_saved_committed.json" "$WORK/game_saved.json"; then
    echo "the game now saves the sample structure differently from crates/schematic/tests/data/game_saved_$MC_VERSION.nbt" >&2
    exit 1
fi
mkdir -p "$CASES/game_saved"
cp "$WORK/game_saved.nbt" "$CASES/game_saved/reference.nbt"
target/release/examples/rewrite "$WORK/game_saved.nbt" "$CASES/game_saved" 5023 5006 5005 3955

echo "==> loading $(find "$CASES" -name 'export.*' -o -name '[0-9]*.*' | wc -l | tr -d ' ') schematic file(s) in Minecraft $MC_VERSION"
java -cp "$CP" tickwright.harness.SchematicCheck --universe "$WORK/universe-schematics" --cases "$CASES" \
    2>&1 | message | tee "$WORK/schematics.log" | grep -E '^(Minecraft|loaded|  DIFFERS|Exception|\s+at )'
