"""The songs jukeboxes play, for the jukebox.

    python3 -I tools/jukebox.py <server jar> <reports dir> <out dir>

Reads each item's jukebox_playable component from the item report and the
song it names from data/minecraft/jukebox_song/ in the server jar, and
writes <out dir>/jukebox_songs.json: for every item a jukebox plays, one per
line, its song, the song's length in seconds and the comparator output it
gives in a jukebox.
"""

import json
import sys
import zipfile
from pathlib import Path

SONGS = "data/minecraft/jukebox_song/"


def playable_items(reports):
    """Item id -> song id, from the item report: one items.json in older
    versions, one file per item under <namespace>/.../item/ in newer ones."""
    def components(v):
        return v.get("components", v)

    single = reports / "items.json"
    if single.exists():
        report = {k if ":" in k else f"minecraft:{k}": components(v) for k, v in json.loads(single.read_text()).items()}
    else:
        report = {}
        for f in sorted(reports.glob("*/**/item/*.json")):
            ns = f.relative_to(reports).parts[0]
            report[f"{ns}:{f.stem}"] = components(json.loads(f.read_text()))
    return {item: c["minecraft:jukebox_playable"] for item, c in report.items() if "minecraft:jukebox_playable" in c}


def main(jar, reports, out):
    z = zipfile.ZipFile(jar)
    songs = {}
    for name in z.namelist():
        if name.startswith(SONGS) and name.endswith(".json"):
            songs["minecraft:" + name[len(SONGS):-5]] = json.loads(z.read(name))
    items = playable_items(Path(reports))
    if not items:
        sys.exit(f"no item in {reports} has a jukebox_playable component")
    rows = []
    for item in sorted(items):
        song = items[item]
        s = songs[song]
        entry = {"song": song, "length_in_seconds": s["length_in_seconds"], "comparator_output": s["comparator_output"]}
        rows.append(f"  {json.dumps(item)}: {json.dumps(entry, separators=(',', ':'))}")
    with open(Path(out) / "jukebox_songs.json", "w") as f:
        f.write("{\n" + ",\n".join(rows) + "\n}\n")
    print(f"    {len(rows)} jukebox songs -> {out}/jukebox_songs.json")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
