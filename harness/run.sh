#!/usr/bin/env bash
# Capture ground-truth traces from the real Minecraft server.
#
#   harness/run.sh [--only <substring>]
#
# Downloads the official server jar (never committed or redistributed),
# checks its SHA-1 against Mojang's manifest, resolves the fixtures with the
# tickwright CLI, and runs every fixture in a headless server. Traces land in
# $WORK/traces. Needs Java 25+, cargo, python3, curl and unzip.
set -euo pipefail
cd "$(dirname "$0")/.."

MC_VERSION="${MC_VERSION:-26.3}"
WORK="${WORK:-harness/work}"
FIXTURES="${FIXTURES:-fixtures}"
mkdir -p "$WORK"

# --- server jar ---------------------------------------------------------------
. harness/server.sh

# --- build the agent and the driver ------------------------------------------
echo "==> compiling harness"
rm -rf "$WORK/agent-classes" "$WORK/classes"
javac -d "$WORK/agent-classes" $(find harness/agent -name '*.java')
printf 'Premain-Class: tickwright.harness.agent.Agent\n' > "$WORK/agent.mf"
jar --create --file "$WORK/agent.jar" --manifest "$WORK/agent.mf" -C "$WORK/agent-classes" .
if ! javac -nowarn -cp "$WORK/agent.jar:$CP" -d "$WORK/classes" $(find harness/src -name '*.java') 2> "$WORK/javac.log"; then
    cat "$WORK/javac.log" >&2
    # One annotation, readable through the API when the log is not.
    echo "::error::$(head -c 6000 "$WORK/javac.log" | sed 's/%/%25/g' | sed ':a;N;$!ba;s/\n/%0A/g')"
    exit 1
fi

# --- resolve fixtures ---------------------------------------------------------
echo "==> resolving fixtures"
rm -rf "$WORK/resolved"
cargo run --quiet --release -p tickwright -- resolve "$FIXTURES" --out "$WORK/resolved"

# --- capture ------------------------------------------------------------------
echo "==> capturing in Minecraft $MC_VERSION"
rm -rf "$WORK/traces"
java -javaagent:"$WORK/agent.jar" -cp "$WORK/classes:$WORK/agent.jar:$CP" \
    tickwright.harness.Capture \
    --fixtures "$WORK/resolved" --out "$WORK/traces" --universe "$WORK/universe" --physics "$WORK/physics.json" "$@" \
    2>&1 | grep -vE 'Saving |chunks are saved|dimensions are saved|Preparing |\[ \]$' | tee "$WORK/capture.log"

# --- check the traces parse ---------------------------------------------------
echo "==> checking traces"
n=0
for t in "$WORK"/traces/*.json; do
    target/release/tickwright diff "$t" "$t" --level order >/dev/null
    n=$((n + 1))
done
echo "    $n trace(s) parse and self-compare"
echo "    traces total: $(du -sh "$WORK/traces" | cut -f1)"

# --- committed copy -----------------------------------------------------------
# traces/<version>/ holds the ground truth the engine is tested against,
# gzipped (-n keeps the output byte-identical between runs).
if [[ $# -eq 0 ]]; then
    rm -rf "traces/$MC_VERSION"
    mkdir -p "traces/$MC_VERSION"
    for t in "$WORK"/traces/*.json; do
        gzip -n -9 -c "$t" > "traces/$MC_VERSION/$(basename "$t").gz"
    done
fi

# --- version ------------------------------------------------------------------
# The jar's version.json gives the data version files saved in this release
# carry; the schematic writer stamps it on exports.
if [[ $# -eq 0 ]]; then
    mkdir -p "data/$MC_VERSION"
    python3 -I - "data/$MC_VERSION/version.json" "$JAR" "$INNER" <<'PY'
import json, sys, zipfile
def read(jar):
    try:
        return json.loads(zipfile.ZipFile(jar).read("version.json"))
    except (OSError, KeyError):
        return None
v = next(filter(None, map(read, sys.argv[2:])))
out = {"name": v["name"], "data_version": v["world_version"], "protocol_version": v["protocol_version"]}
open(sys.argv[1], "w").write(json.dumps(out, indent=2) + "\n")
PY
    # Collision shapes and block friction, for entity physics.
    cp "$WORK/physics.json" "data/$MC_VERSION/physics.json"
    # What blocks drop when they break, for item entities.
    python3 -I tools/loot.py "$INNER" "data/$MC_VERSION"
    # Crafting recipes and item tags, for the crafter.
    python3 -I tools/recipes.py "$INNER" "data/$MC_VERSION"
    # The world clock's timelines (sky light and sun angle through the day),
    # for the daylight detector.
    python3 -I - "$INNER" "data/$MC_VERSION/timelines.json" <<'PY'
import json, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
out = {}
for name in sorted(z.namelist()):
    if name.endswith(".json") and (name.startswith("data/minecraft/timeline/")
                                   or name == "data/minecraft/dimension_type/overworld.json"):
        out[name.removeprefix("data/minecraft/")[:-5]] = json.loads(z.read(name))
open(sys.argv[2], "w").write(json.dumps(out, indent=1, sort_keys=True) + "\n")
print(f"    timelines: {sorted(out)}")
PY
fi

# --- data reports -------------------------------------------------------------
# The game's own dump of every block, its properties and default state, and
# every registry. tools/registry.py condenses it into data/<version>/, and
# tools/materials.py adds materials.json.
if [[ -n "${REPORTS:-}" ]]; then
    echo "==> generating data reports"
    rm -rf "$WORK/reports"
    (cd "$WORK" && java -DbundlerMainClass=net.minecraft.data.Main -jar "server-$MC_VERSION.jar" \
        --reports --output reports >/dev/null)
    python3 -I tools/registry.py "$WORK/reports/reports" "data/$MC_VERSION"
    # What each block takes to build, from the jar's loot tables and the
    # report's stack sizes (crates/materials and the mod read it).
    # Failures go out as an annotation too, since CI logs can't be read
    # from everywhere.
    if ! out="$(python3 -I tools/materials.py "$INNER" "$WORK/reports/reports" "data/$MC_VERSION" 2>&1)"; then
        echo "$out"
        reports="$(ls "$WORK/reports/reports" | tr '\n' ' ')"
        echo "::error::tools/materials.py failed (reports: $reports)%0A$(tail -n 15 <<<"$out" | sed 's/%/%25/g' | sed ':a;N;$!ba;s/\n/%0A/g')"
        exit 1
    fi
    echo "$out"
fi

# --- schematics in the game ---------------------------------------------------
# The engine's .nbt, .schem and .litematic exports, loaded in the game, must
# place what they were written from, and a structure the game saves must
# import (harness/schematics.sh).
if [[ $# -eq 0 ]]; then
    harness/schematics.sh
fi
