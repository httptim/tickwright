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
JAR="$WORK/server-$MC_VERSION.jar"
read -r URL SHA1 < <(python3 - "$MC_VERSION" <<'PY'
import json, sys, urllib.request
version = sys.argv[1]
manifest = json.load(urllib.request.urlopen(
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"))
entry = next(v for v in manifest["versions"] if v["id"] == version)
server = json.load(urllib.request.urlopen(entry["url"]))["downloads"]["server"]
print(server["url"], server["sha1"])
PY
)
if [[ ! -f "$JAR" ]] || [[ "$(sha1sum "$JAR" | cut -d' ' -f1)" != "$SHA1" ]]; then
    echo "==> downloading Minecraft $MC_VERSION server"
    curl -sSL --fail -o "$JAR" "$URL"
fi
GOT="$(sha1sum "$JAR" | cut -d' ' -f1)"
if [[ "$GOT" != "$SHA1" ]]; then
    echo "server jar SHA-1 is $GOT, manifest says $SHA1" >&2
    exit 1
fi

# --- unpack the bundler -------------------------------------------------------
# The published jar wraps the real server and its libraries.
INNER_DIR="$WORK/inner-$MC_VERSION"
INNER="$INNER_DIR/META-INF/versions/$MC_VERSION/server-$MC_VERSION.jar"
if [[ ! -f "$INNER" ]]; then
    rm -rf "$INNER_DIR"
    unzip -o -q "$JAR" "META-INF/versions/*" "META-INF/libraries/*" -d "$INNER_DIR"
fi
CP="$INNER:$(find "$INNER_DIR/META-INF/libraries" -name '*.jar' | sort | tr '\n' ':')"

# --- build the agent and the driver ------------------------------------------
echo "==> compiling harness"
rm -rf "$WORK/agent-classes" "$WORK/classes"
javac -d "$WORK/agent-classes" $(find harness/agent -name '*.java')
printf 'Premain-Class: tickwright.harness.agent.Agent\n' > "$WORK/agent.mf"
jar --create --file "$WORK/agent.jar" --manifest "$WORK/agent.mf" -C "$WORK/agent-classes" .
javac -nowarn -cp "$WORK/agent.jar:$CP" -d "$WORK/classes" $(find harness/src -name '*.java')

# --- resolve fixtures ---------------------------------------------------------
echo "==> resolving fixtures"
rm -rf "$WORK/resolved"
cargo run --quiet --release -p tickwright -- resolve "$FIXTURES" --out "$WORK/resolved"

# --- capture ------------------------------------------------------------------
echo "==> capturing in Minecraft $MC_VERSION"
rm -rf "$WORK/traces"
java -javaagent:"$WORK/agent.jar" -cp "$WORK/classes:$WORK/agent.jar:$CP" \
    tickwright.harness.Capture \
    --fixtures "$WORK/resolved" --out "$WORK/traces" --universe "$WORK/universe" "$@" \
    2>&1 | grep -vE 'Saving |chunks are saved|dimensions are saved|Preparing '

# --- check the traces parse ---------------------------------------------------
echo "==> checking traces"
n=0
for t in "$WORK"/traces/*.json; do
    target/release/tickwright diff "$t" "$t" --level order >/dev/null
    n=$((n + 1))
done
echo "    $n trace(s) parse and self-compare"
echo "    traces total: $(du -sh "$WORK/traces" | cut -f1)"

# --- data reports -------------------------------------------------------------
# The game's own dump of every block, its properties and default state, and
# every registry. tools/registry.py condenses it into data/<version>/.
if [[ -n "${REPORTS:-}" ]]; then
    echo "==> generating data reports"
    rm -rf "$WORK/reports"
    (cd "$WORK" && java -DbundlerMainClass=net.minecraft.data.Main -jar "server-$MC_VERSION.jar" \
        --reports --output reports >/dev/null)
    python3 -I tools/registry.py "$WORK/reports/reports" "data/$MC_VERSION"
fi
