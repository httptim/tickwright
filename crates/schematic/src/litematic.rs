//! Litematica's `.litematic`: one or more regions, each with a palette and
//! its block indices bit-packed across a long array.

use anyhow::{Context, Result, bail};

use crate::nbt::{self, Tag};
use crate::states::{self, LEGACY};
use crate::{BlockEntity, Entity, Schematic, palette, strip_keys};

/// `LitematicaBitArray` entry width for a palette of `len` states.
fn bits_for(len: usize) -> u32 {
    (usize::BITS - (len.max(2) - 1).leading_zeros()).max(2)
}

fn xyz(tag: &Tag, key: &str) -> Result<[i32; 3]> {
    let t = tag.req(key)?;
    Ok([t.int("x")?, t.int("y")?, t.int("z")?])
}

pub fn from_nbt(root: &Tag) -> Result<Schematic> {
    let regions = root.req("Regions")?.entries();
    if regions.is_empty() {
        bail!("the litematic has no regions");
    }
    // Each region's least corner and size, in schematic space, and its
    // `Position`, the corner its selection started from.
    let mut boxes = Vec::new();
    let mut corners = Vec::new();
    for (name, r) in regions {
        let pos = xyz(r, "Position").with_context(|| format!("region {name}"))?;
        let size = xyz(r, "Size").with_context(|| format!("region {name}"))?;
        let min: Vec<i32> = (0..3).map(|i| pos[i] + if size[i] < 0 { size[i] + 1 } else { 0 }).collect();
        crate::check_volume(size).with_context(|| format!("region {name}"))?;
        boxes.push(([min[0], min[1], min[2]], size.map(i32::abs)));
        corners.push(pos);
    }
    let lo: Vec<i32> = (0..3).map(|i| boxes.iter().map(|b| b.0[i]).min().unwrap()).collect();
    let hi: Vec<i32> = (0..3).map(|i| boxes.iter().map(|b| b.0[i] + b.1[i]).max().unwrap()).collect();
    crate::check_volume([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])?;
    let mut out = Schematic {
        size: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        data_version: root.get("MinecraftDataVersion").and_then(Tag::as_i64).map(|v| v as i32),
        ..Default::default()
    };
    for (((name, r), (min, size)), corner) in regions.iter().zip(&boxes).zip(&corners) {
        let states: Vec<_> = r
            .req("BlockStatePalette")?
            .list()
            .iter()
            .map(|t| states::read(t, out.data_version))
            .collect::<Result<_>>()
            .with_context(|| format!("region {name}"))?;
        let longs = match r.req("BlockStates")? {
            Tag::LongArray(v) => v,
            _ => bail!("region {name}: BlockStates is not a long array"),
        };
        let bits = bits_for(states.len()) as u64;
        let mask = (1u64 << bits) - 1;
        let [sx, sy, sz] = *size;
        let volume = sx as u64 * sy as u64 * sz as u64;
        if (longs.len() as u64) * 64 < volume * bits {
            bail!("region {name}: BlockStates is too short for its size");
        }
        for i in 0..volume {
            let start = i * bits;
            let (a, b) = ((start >> 6) as usize, (((i + 1) * bits - 1) >> 6) as usize);
            let off = start & 63;
            let v = if a == b {
                (longs[a] as u64 >> off) & mask
            } else {
                ((longs[a] as u64 >> off) | ((longs[b] as u64) << (64 - off))) & mask
            };
            let state = states.get(v as usize).with_context(|| format!("region {name}: index {v} past the palette"))?;
            if state.is_air() {
                continue;
            }
            let i = i as i32;
            let (x, y, z) = (i % sx, i / (sx * sz), (i / sx) % sz);
            out.blocks.push(([min[0] - lo[0] + x, min[1] - lo[1] + y, min[2] - lo[2] + z], state.clone()));
        }
        for te in r.get("TileEntities").map(Tag::list).unwrap_or(&[]) {
            let p = [te.int("x")?, te.int("y")?, te.int("z")?];
            out.block_entities.push(BlockEntity {
                pos: [min[0] - lo[0] + p[0], min[1] - lo[1] + p[1], min[2] - lo[2] + p[2]],
                data: strip_keys(te, &["x", "y", "z"]),
            });
        }
        // Litematica keeps entity positions relative to the region's
        // `Position`, which is its far corner on an axis with a negative size.
        let offset = [(corner[0] - lo[0]) as f64, (corner[1] - lo[1]) as f64, (corner[2] - lo[2]) as f64];
        for e in r.get("Entities").map(Tag::list).unwrap_or(&[]) {
            out.entities.push(Entity::from_saved(e, offset).with_context(|| format!("region {name}"))?);
        }
    }
    Ok(out.finish())
}

pub fn write(s: &Schematic) -> Vec<u8> {
    let (states, cells) = palette(s);
    let bits = bits_for(states.len()) as u64;
    let mut longs = vec![0u64; ((cells.len() as u64 * bits).div_ceil(64)) as usize];
    for (i, &v) in cells.iter().enumerate() {
        let start = i as u64 * bits;
        let (a, b) = ((start >> 6) as usize, (((i as u64 + 1) * bits - 1) >> 6) as usize);
        let off = start & 63;
        longs[a] |= (v as u64) << off;
        if a != b {
            longs[b] |= (v as u64) >> (64 - off);
        }
    }
    let [sx, sy, sz] = s.size;
    let vec3 = |x: i32, y: i32, z: i32| nbt::compound(vec![("x", Tag::Int(x)), ("y", Tag::Int(y)), ("z", Tag::Int(z))]);
    let tiles = s
        .block_entities
        .iter()
        .map(|be| {
            let mut e = vec![
                ("x".to_string(), Tag::Int(be.pos[0])),
                ("y".to_string(), Tag::Int(be.pos[1])),
                ("z".to_string(), Tag::Int(be.pos[2])),
            ];
            e.extend(be.data.entries().iter().cloned());
            Tag::Compound(e)
        })
        .collect();
    let region = nbt::compound(vec![
        ("Position", vec3(0, 0, 0)),
        ("Size", vec3(sx, sy, sz)),
        // Litematica's own palettes say `Name` and `Properties` in every
        // version; its reader for 26.3 takes those or `id` and `properties`.
        ("BlockStatePalette", nbt::list(states.iter().map(|st| states::write(st, LEGACY)).collect())),
        ("BlockStates", Tag::LongArray(longs.into_iter().map(|l| l as i64).collect())),
        ("TileEntities", Tag::List(10, tiles)),
        ("Entities", Tag::List(10, s.entities.iter().map(Entity::saved).collect())),
        ("PendingBlockTicks", Tag::List(10, vec![])),
        ("PendingFluidTicks", Tag::List(10, vec![])),
    ]);
    let metadata = nbt::compound(vec![
        ("Name", Tag::String("Tickwright export".into())),
        ("Author", Tag::String("Tickwright".into())),
        ("Description", Tag::String(String::new())),
        ("RegionCount", Tag::Int(1)),
        ("TotalVolume", Tag::Int(sx * sy * sz)),
        ("TotalBlocks", Tag::Int(s.blocks.len() as i32)),
        ("TimeCreated", Tag::Long(0)),
        ("TimeModified", Tag::Long(0)),
        ("EnclosingSize", vec3(sx, sy, sz)),
    ]);
    let root = nbt::compound(vec![
        ("MinecraftDataVersion", Tag::Int(s.written_version())),
        ("Version", Tag::Int(7)),
        ("SubVersion", Tag::Int(1)),
        ("Metadata", metadata),
        ("Regions", Tag::Compound(vec![("Tickwright".to_string(), region)])),
    ]);
    nbt::write_gz("", &root)
}

#[cfg(test)]
mod tests {
    use super::{bits_for, from_nbt};
    use crate::nbt::{self, Tag};

    #[test]
    fn entities_sit_relative_to_the_region_position() {
        let vec3 =
            |x: i32, y: i32, z: i32| nbt::compound(vec![("x", Tag::Int(x)), ("y", Tag::Int(y)), ("z", Tag::Int(z))]);
        // Selected from the far x corner: Position x=2 with size x=-3 covers
        // x 0..=2, and the cart stored at x -0.5 from Position is at 1.5.
        let cart = nbt::compound(vec![
            ("id", Tag::String("minecraft:minecart".into())),
            ("Pos", Tag::List(6, vec![Tag::Double(-0.5), Tag::Double(0.0), Tag::Double(0.5)])),
        ]);
        let region = nbt::compound(vec![
            ("Position", vec3(2, 0, 0)),
            ("Size", vec3(-3, 1, 1)),
            ("BlockStatePalette", nbt::list(vec![nbt::compound(vec![("Name", Tag::String("minecraft:air".into()))])])),
            ("BlockStates", Tag::LongArray(vec![0])),
            ("TileEntities", Tag::List(10, vec![])),
            ("Entities", Tag::List(10, vec![cart])),
        ]);
        let root = nbt::compound(vec![("Regions", Tag::Compound(vec![("R".to_string(), region)]))]);
        let s = from_nbt(&root).unwrap();
        assert_eq!(s.size, [3, 1, 1]);
        assert_eq!(s.entities[0].pos, [1.5, 0.0, 0.5]);
    }

    #[test]
    fn widths_match_litematica() {
        assert_eq!([1, 2, 4, 5, 8, 9, 16, 17].map(bits_for), [2, 2, 2, 3, 3, 4, 4, 5]);
    }
}
