# Sourced by run.sh and kick-trig.sh, from the repository root, with
# MC_VERSION and WORK set.
#
# Downloads the official server jar (never committed or redistributed),
# checks its SHA-1 against Mojang's manifest, and unpacks the bundler. Sets
# JAR, INNER (the server itself) and CP (the server and its libraries).

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
