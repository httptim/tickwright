//! The game's structure `.nbt`, as structure blocks save it: a palette and
//! a list of blocks, each with its position and any block entity data.

use anyhow::{Context, Result};

use crate::nbt::{self, Tag};
use crate::{BlockEntity, Entity, Schematic, doubles, palette, state_from_nbt, state_to_nbt};

pub fn from_nbt(root: &Tag) -> Result<Schematic> {
    let size = root.req("size")?.ints().context("`size` is not three ints")?;
    // Structures with variants keep several palettes; the first is the default.
    let pal = match root.get("palette") {
        Some(p) => p,
        None => root.req("palettes")?.list().first().context("`palettes` is empty")?,
    };
    crate::check_volume([size[0], size[1], size[2]])?;
    let states: Vec<_> = pal.list().iter().map(state_from_nbt).collect::<Result<_>>()?;
    let mut out = Schematic {
        size: [size[0], size[1], size[2]],
        data_version: root.get("DataVersion").and_then(Tag::as_i64).map(|v| v as i32),
        ..Default::default()
    };
    for b in root.req("blocks")?.list() {
        let p = b.req("pos")?.ints().context("a block's `pos` is not three ints")?;
        let i = b.int("state")? as usize;
        let state = states.get(i).with_context(|| format!("block state {i} is not in the palette"))?;
        out.blocks.push(([p[0], p[1], p[2]], state.clone()));
        if let Some(data) = b.get("nbt") {
            out.block_entities.push(BlockEntity { pos: [p[0], p[1], p[2]], data: data.clone() });
        }
    }
    // Each entity's `pos` is where it is in the structure; its own `Pos`
    // is where it was when saved, which placing ignores.
    for e in root.get("entities").map(Tag::list).unwrap_or(&[]) {
        let pos = doubles(e.req("pos")?).context("an entity's `pos` is not three doubles")?;
        let mut entity = Entity::from_saved(e.req("nbt")?, [0.0; 3])?;
        entity.pos = pos;
        out.entities.push(entity);
    }
    Ok(out.finish())
}

/// Every position in the box is written, air included, so placing the
/// structure clears what was there, as a saved structure does.
pub fn write(s: &Schematic) -> Vec<u8> {
    let (states, cells) = palette(s);
    let [sx, sy, sz] = s.size;
    let mut blocks = Vec::with_capacity(cells.len());
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let state = cells[((y * sz + z) * sx + x) as usize];
                let mut e = vec![
                    ("pos", nbt::list(vec![Tag::Int(x), Tag::Int(y), Tag::Int(z)])),
                    ("state", Tag::Int(state as i32)),
                ];
                if let Some(be) = s.block_entities.iter().find(|be| be.pos == [x, y, z]) {
                    e.push(("nbt", be.data.clone()));
                }
                blocks.push(nbt::compound(e));
            }
        }
    }
    let root = nbt::compound(vec![
        ("DataVersion", Tag::Int(s.data_version.unwrap_or(crate::DATA_VERSION_26_3))),
        ("size", nbt::list(vec![Tag::Int(sx), Tag::Int(sy), Tag::Int(sz)])),
        ("palette", nbt::list(states.iter().map(state_to_nbt).collect())),
        ("blocks", Tag::List(10, blocks)),
        (
            "entities",
            Tag::List(
                10,
                s.entities
                    .iter()
                    .map(|e| {
                        let block = e.pos.map(|v| Tag::Int(v.floor() as i32));
                        nbt::compound(vec![
                            ("pos", Tag::List(6, e.pos.iter().map(|&v| Tag::Double(v)).collect())),
                            ("blockPos", nbt::list(block.to_vec())),
                            ("nbt", e.saved()),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    nbt::write_gz("", &root)
}
