#!/usr/bin/env bash
# Check the engine's Math.sin and Math.cos against this machine's JVM, for
# the angles newly primed TNT kicks at.
#
#   harness/kick-trig.sh [count] [seed]
#
# Draws `count` angles (default 1048576) with the game's own random source,
# as PrimedTnt does, and has the JVM work out Math.sin and Math.cos of each:
# interpreted (-Xint), compiled by C1 only, compiled by C2 only, and with the
# default flags the captures run the server with. Then compares every result
# with crates/engine/src/math.rs bit for bit, and fails if any differs. All
# differences land in $WORK/kick-trig/diffs.tsv. Needs what run.sh needs.
set -euo pipefail
cd "$(dirname "$0")/.."

MC_VERSION="${MC_VERSION:-26.3}"
WORK="${WORK:-harness/work}"
COUNT="${1:-1048576}"
SEED="${2:-0}"
mkdir -p "$WORK"

. harness/server.sh

OUT="$WORK/kick-trig"
rm -rf "$OUT"
mkdir -p "$OUT/classes"
javac -nowarn -cp "$CP" -d "$OUT/classes" harness/kick-trig/KickTrig.java

echo "==> machine"
uname -m
if command -v lscpu >/dev/null; then
    lscpu | grep '^Model name'
    echo "flags: $(grep -m1 '^flags' /proc/cpuinfo | grep -owE 'sse3|sse4_2|avx|avx2|fma|avx512f' | tr '\n' ' ')"
else
    sysctl -n machdep.cpu.brand_string 2>/dev/null || true
fi
java -version 2>&1
java -XX:+UnlockDiagnosticVMOptions -XX:+PrintFlagsFinal -version 2>/dev/null |
    grep -E ' (UseLibmIntrinsic|InlineIntrinsics|InlineMathNatives|UseSSE|UseAVX|UseFMA) ' || true

# One JVM per mode; the compiled modes print what the JIT did with the loop.
kick() {
    local mode="$1" passes="$2"
    shift 2
    echo "==> $mode: java $*"
    java "$@" -cp "$OUT/classes:$CP" KickTrig "$SEED" "$COUNT" "$passes" "$OUT/$mode.bin" \
        > "$OUT/$mode.log" 2> "$OUT/$mode.vm.log" || {
        cat "$OUT/$mode.log" "$OUT/$mode.vm.log"
        exit 1
    }
    cat "$OUT/$mode.log"
    grep -E 'KickTrig::kick|Math::(sin|cos)' "$OUT/$mode.vm.log" | head -20 || true
}
JIT=(-XX:+UnlockDiagnosticVMOptions -XX:+PrintCompilation -XX:+PrintInlining -XX:+DisplayVMOutputToStderr)
kick interpreted 1 -Xint
kick c1 10 -XX:TieredStopAtLevel=1 "${JIT[@]}"
kick c2 10 -XX:-TieredCompilation "${JIT[@]}"
kick server 10

echo "==> comparing with the engine"
cargo run --quiet --release -p tickwright-engine --example kick_trig -- --diffs "$OUT/diffs.tsv" \
    "$OUT"/{interpreted,c1,c2,server}.bin | tee "$OUT/summary.txt"
