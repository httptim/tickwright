#!/usr/bin/env bash
# Check the engine's Math.sin and Math.cos against this machine's JVM, for
# the angles newly primed TNT kicks at.
#
#   harness/kick-trig.sh [count] [seed...]
#
# For each seed, draws `count` angles (default 1048576) with the game's own
# random source, as PrimedTnt does, and has the JVM work out Math.sin and
# Math.cos of each: interpreted (-Xint), compiled by C1 only, compiled by C2
# only, and with the default flags the captures run the server with. Then
# compares every result with crates/engine/src/math.rs bit for bit, and fails
# if any differs. The seeds default to 0 and one that changes every run (the
# workflow run's id, or the time), so repeated runs cover new angles; the
# summary names each seed, so a failure can be run again. All differences
# land in $WORK/kick-trig/diffs.tsv. Needs what run.sh needs.
#
# It passes only on x86-64: math.rs follows the stub HotSpot runs there, and
# HotSpot on arm64 gives other bits on about one angle in six.
set -euo pipefail
cd "$(dirname "$0")/.."

MC_VERSION="${MC_VERSION:-26.3}"
WORK="${WORK:-harness/work}"
COUNT="${1:-1048576}"
shift $(($# > 0 ? 1 : 0))
SEEDS=("$@")
if [[ ${#SEEDS[@]} -eq 0 ]]; then
    SEEDS=(0 "${GITHUB_RUN_ID:-$(date +%s)}")
fi
mkdir -p "$WORK"

. harness/server.sh

OUT="$WORK/kick-trig"
rm -rf "$OUT"
mkdir -p "$OUT/classes"
javac -nowarn -cp "$CP" -d "$OUT/classes" harness/kick-trig/KickTrig.java

echo "==> machine"
uname -m
if [[ "$(uname -m)" != x86_64 ]]; then
    echo "    not x86-64: expect differences, since math.rs follows HotSpot on x86-64"
fi
if command -v lscpu >/dev/null; then
    lscpu | grep '^Model name'
    echo "flags: $(grep -m1 '^flags' /proc/cpuinfo | grep -owE 'sse3|sse4_2|avx|avx2|fma|avx512f' | tr '\n' ' ')"
else
    sysctl -n machdep.cpu.brand_string 2>/dev/null || true
fi
java -version 2>&1
java -XX:+UnlockDiagnosticVMOptions -XX:+PrintFlagsFinal -version 2>/dev/null |
    grep -E ' (UseLibmIntrinsic|InlineIntrinsics|InlineMathNatives|UseSSE|UseAVX|UseFMA) ' || true

# One JVM per mode and seed; the compiled modes print what the JIT did with
# the loop, shown for the first seed.
kick() {
    local mode="$1" seed="$2" passes="$3"
    shift 3
    local name="$mode-$seed"
    echo "==> $name: java $*"
    java "$@" -cp "$OUT/classes:$CP" KickTrig "$seed" "$COUNT" "$passes" "$OUT/$name.bin" \
        > "$OUT/$name.log" 2> "$OUT/$name.vm.log" || {
        cat "$OUT/$name.log" "$OUT/$name.vm.log"
        exit 1
    }
    cat "$OUT/$name.log"
    if [[ "$seed" == "${SEEDS[0]}" ]]; then
        grep -E 'KickTrig::kick|Math::(sin|cos)' "$OUT/$name.vm.log" | head -20 || true
    fi
}
JIT=(-XX:+UnlockDiagnosticVMOptions -XX:+PrintCompilation -XX:+PrintInlining -XX:+DisplayVMOutputToStderr)
files=()
for seed in "${SEEDS[@]}"; do
    kick interpreted "$seed" 1 -Xint
    kick c1 "$seed" 10 -XX:TieredStopAtLevel=1 "${JIT[@]}"
    kick c2 "$seed" 10 -XX:-TieredCompilation "${JIT[@]}"
    kick server "$seed" 10
    files+=("$OUT"/{interpreted,c1,c2,server}-"$seed".bin)
done

echo "==> comparing with the engine"
cargo run --quiet --release -p tickwright-engine --example kick_trig -- --diffs "$OUT/diffs.tsv" "${files[@]}" |
    tee "$OUT/summary.txt"
