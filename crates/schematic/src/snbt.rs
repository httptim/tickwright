//! SNBT for schematic tags, through the shared reader and writer in
//! `tickwright_trace::snbt`, so a file's data reads and writes exactly as
//! the engine reads and writes it.
//!
//! A list may mix tag types, as 26.3's lists may. In a file such a list is
//! a list of compounds, each element that isn't a plain compound wrapped
//! as `{"": value}`; reading unwraps them, as `ListTag` does.

use anyhow::Result;
use tickwright_trace::snbt::{self, Tag as Text};

use crate::nbt::Tag;

/// `tag` as SNBT, with every tag's type.
pub fn write(tag: &Tag) -> String {
    snbt::write(&to_text(tag))
}

/// Parse SNBT as the game's `TagParser` does.
pub fn parse(text: &str) -> Result<Tag> {
    Ok(from_text(snbt::parse(text)?))
}

/// A file's tag as SNBT's, list wrappers taken off (`ListTag.addAndUnwrap`).
pub fn to_text(tag: &Tag) -> Text {
    match tag {
        Tag::Byte(v) => Text::Byte(*v),
        Tag::Short(v) => Text::Short(*v),
        Tag::Int(v) => Text::Int(*v),
        Tag::Long(v) => Text::Long(*v),
        Tag::Float(v) => Text::Float(*v),
        Tag::Double(v) => Text::Double(*v),
        Tag::String(s) => Text::String(s.clone()),
        Tag::ByteArray(v) => Text::ByteArray(v.clone()),
        Tag::IntArray(v) => Text::IntArray(v.clone()),
        Tag::LongArray(v) => Text::LongArray(v.clone()),
        Tag::List(_, items) => Text::List(
            items
                .iter()
                .map(|t| match t.entries() {
                    [(k, inner)] if k.is_empty() && matches!(t, Tag::Compound(_)) => to_text(inner),
                    _ => to_text(t),
                })
                .collect(),
        ),
        Tag::Compound(entries) => Text::Compound(entries.iter().map(|(k, v)| (k.clone(), to_text(v))).collect()),
    }
}

/// SNBT's tag as a file's: a list that mixes types becomes a list of
/// compounds, wrapping what needs it (`ListTag.write`).
pub fn from_text(tag: Text) -> Tag {
    match tag {
        Text::Byte(v) => Tag::Byte(v),
        Text::Short(v) => Tag::Short(v),
        Text::Int(v) => Tag::Int(v),
        Text::Long(v) => Tag::Long(v),
        Text::Float(v) => Tag::Float(v),
        Text::Double(v) => Tag::Double(v),
        Text::String(s) => Tag::String(s),
        Text::ByteArray(v) => Tag::ByteArray(v),
        Text::IntArray(v) => Tag::IntArray(v),
        Text::LongArray(v) => Tag::LongArray(v),
        Text::List(items) => {
            let items: Vec<Tag> = items.into_iter().map(from_text).collect();
            let first = items.first().map_or(0, Tag::id);
            let id = if items.iter().all(|t| t.id() == first) { first } else { 10 };
            if id != 10 {
                return Tag::List(id, items);
            }
            let wrapped = items
                .into_iter()
                .map(|t| match &t {
                    Tag::Compound(e) if !(e.len() == 1 && e[0].0.is_empty()) => t,
                    _ => Tag::Compound(vec![(String::new(), t)]),
                })
                .collect();
            Tag::List(10, wrapped)
        }
        Text::Compound(m) => Tag::Compound(m.into_iter().map(|(k, v)| (k, from_text(v))).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::compound;

    #[test]
    fn every_type_round_trips() {
        let tag = compound(vec![
            ("Item", compound(vec![("count", Tag::Int(1)), ("id", Tag::String("minecraft:stone".into()))])),
            ("b", Tag::Byte(-3)),
            ("ba", Tag::ByteArray(vec![1, -1])),
            ("d", Tag::Double(1.0)),
            ("empty", Tag::List(0, vec![])),
            ("f", Tag::Float(0.25)),
            ("i", Tag::Int(7)),
            ("ia", Tag::IntArray(vec![1, 2, 3])),
            ("l", Tag::Long(1 << 40)),
            ("la", Tag::LongArray(vec![5])),
            ("list", Tag::List(6, vec![Tag::Double(0.5), Tag::Double(-2.0)])),
            ("s", Tag::Short(300)),
            ("str", Tag::String("a \"b\"\n".into())),
        ]);
        let text = write(&tag);
        assert_eq!(parse(&text).unwrap(), tag, "{text}");
    }

    #[test]
    fn reads_what_people_type() {
        let t = parse("{fuse:80, Item:{id:\"stone\",count:1}, NoGravity:true}").unwrap();
        assert_eq!(t.get("fuse"), Some(&Tag::Int(80)));
        assert_eq!(t.get("NoGravity"), Some(&Tag::Byte(1)));
        assert_eq!(t.req("Item").unwrap().get("id").and_then(Tag::as_str), Some("stone"));
    }

    #[test]
    fn mixed_lists_are_wrapped_in_files() {
        let t = parse(r#"{l:[1b,"a",{x:1}]}"#).unwrap();
        let wrap = |v: Tag| compound(vec![("", v)]);
        let file =
            Tag::List(10, vec![wrap(Tag::Byte(1)), wrap(Tag::String("a".into())), compound(vec![("x", Tag::Int(1))])]);
        assert_eq!(t.get("l"), Some(&file));
        assert_eq!(write(&t), r#"{l:[1b,"a",{x:1}]}"#);
        // A compound shaped like a wrapper is wrapped again, so it reads
        // back as itself.
        let shaped = Tag::List(10, vec![wrap(wrap(Tag::Int(2)))]);
        assert_eq!(to_text(&shaped), Text::List(vec![to_text(&wrap(Tag::Int(2)))]));
        assert_eq!(from_text(to_text(&shaped)), shaped);
    }
}
