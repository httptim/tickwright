//! Sponge `.schem`, as WorldEdit writes it: version 3 (blocks under
//! `Blocks`) and version 2 (blocks at the top level). Block indices are
//! varints in x, z, y order.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use tickwright_trace::BlockState;

use crate::nbt::{self, Tag};
use crate::{BlockEntity, Schematic, palette, strip_keys};

pub fn from_nbt(root: &Tag) -> Result<Schematic> {
    // Version 3 nests everything in `Schematic`; version 2's root is it.
    let s = root.get("Schematic").unwrap_or(root);
    let version = s.int("Version").unwrap_or(2);
    let [w, h, l] = [s.int("Width")?, s.int("Height")?, s.int("Length")?];
    let [w, h, l] = [w as u16 as i32, h as u16 as i32, l as u16 as i32];
    crate::check_volume([w, h, l])?;
    let (pal, data, entities) = if version >= 3 {
        let b = s.req("Blocks")?;
        (b.req("Palette")?, b.req("Data")?, b.get("BlockEntities"))
    } else {
        (s.req("Palette")?, s.req("BlockData")?, s.get("BlockEntities"))
    };
    let mut states: BTreeMap<u32, BlockState> = BTreeMap::new();
    for (text, idx) in pal.entries() {
        let i = idx.as_i64().context("a palette index is not a number")? as u32;
        states.insert(i, BlockState::parse(text)?);
    }
    let Tag::ByteArray(bytes) = data else { bail!("block data is not a byte array") };
    let mut out = Schematic {
        size: [w, h, l],
        data_version: s.get("DataVersion").and_then(Tag::as_i64).map(|v| v as i32),
        ..Default::default()
    };
    let mut it = bytes.iter().map(|&b| b as u8);
    for i in 0..w * h * l {
        let mut v = 0u32;
        let mut shift = 0;
        loop {
            let b = it.next().context("block data ends early")?;
            v |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                bail!("a varint in the block data is too long");
            }
        }
        let state = states.get(&v).with_context(|| format!("block index {v} is not in the palette"))?;
        let (x, z, y) = (i % w, (i / w) % l, i / (w * l));
        out.blocks.push(([x, y, z], state.clone()));
    }
    for be in entities.map(Tag::list).unwrap_or(&[]) {
        let p = be.req("Pos")?.ints().context("`Pos` is not three ints")?;
        let id = be.get("Id").and_then(Tag::as_str).map(str::to_string);
        // Version 3 keeps the data in `Data`; version 2 inlines it.
        let mut data = match be.get("Data") {
            Some(d) => d.clone(),
            None => strip_keys(be, &["Pos", "Id"]),
        };
        if let (Some(id), Tag::Compound(entries)) = (id, &mut data)
            && !entries.iter().any(|(k, _)| k == "id")
        {
            entries.insert(0, ("id".to_string(), Tag::String(id)));
        }
        out.block_entities.push(BlockEntity { pos: [p[0], p[1], p[2]], data });
    }
    Ok(out.finish())
}

/// Write version 3.
pub fn write(s: &Schematic) -> Vec<u8> {
    let (states, cells) = palette(s);
    let [w, h, l] = s.size;
    // Our cells are y, z, x like Sponge's, so the order carries over.
    let mut data = Vec::new();
    for mut v in cells {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                data.push(b as i8);
                break;
            }
            data.push((b | 0x80) as i8);
        }
    }
    let pal = states.iter().enumerate().map(|(i, st)| (st.to_string(), Tag::Int(i as i32))).collect();
    let entities = s
        .block_entities
        .iter()
        .map(|be| {
            let id = be.data.get("id").cloned().unwrap_or(Tag::String(s.block(be.pos).id));
            nbt::compound(vec![
                ("Pos", Tag::IntArray(be.pos.to_vec())),
                ("Id", id),
                ("Data", strip_keys(&be.data, &["id"])),
            ])
        })
        .collect();
    let blocks = nbt::compound(vec![
        ("Palette", Tag::Compound(pal)),
        ("Data", Tag::ByteArray(data)),
        ("BlockEntities", Tag::List(10, entities)),
    ]);
    let schem = nbt::compound(vec![
        ("Version", Tag::Int(3)),
        ("DataVersion", Tag::Int(s.data_version.unwrap_or(crate::DATA_VERSION_26_3))),
        ("Width", Tag::Short(w as i16)),
        ("Height", Tag::Short(h as i16)),
        ("Length", Tag::Short(l as i16)),
        ("Offset", Tag::IntArray(vec![0, 0, 0])),
        ("Blocks", blocks),
    ]);
    nbt::write_gz("", &nbt::compound(vec![("Schematic", schem)]))
}
