//! SNBT, the text form of NBT that `/summon` and fixtures use, written and
//! read with every tag's type kept: `1b`, `2s`, `3`, `4L`, `1.5f`, `2.0d`,
//! `"text"`, `[...]`, `[B;...]`, `[I;...]`, `[L;...]` and `{key:value}`.

use anyhow::{Context, Result, bail};

use crate::nbt::Tag;

/// `tag` as SNBT.
pub fn write(tag: &Tag) -> String {
    let mut out = String::new();
    write_to(&mut out, tag);
    out
}

fn write_to(out: &mut String, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push_str(&format!("{v}b")),
        Tag::Short(v) => out.push_str(&format!("{v}s")),
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => out.push_str(&format!("{v}L")),
        Tag::Float(v) => out.push_str(&format!("{v:?}f")),
        Tag::Double(v) => out.push_str(&format!("{v:?}d")),
        Tag::String(s) => quote(out, s),
        Tag::ByteArray(v) => array(out, 'B', v.iter().map(|b| format!("{b}b"))),
        Tag::IntArray(v) => array(out, 'I', v.iter().map(|i| i.to_string())),
        Tag::LongArray(v) => array(out, 'L', v.iter().map(|l| format!("{l}L"))),
        Tag::List(_, items) => {
            out.push('[');
            for (i, t) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_to(out, t);
            }
            out.push(']');
        }
        Tag::Compound(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !k.is_empty() && k.chars().all(bare) {
                    out.push_str(k);
                } else {
                    quote(out, k);
                }
                out.push(':');
                write_to(out, v);
            }
            out.push('}');
        }
    }
}

fn bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

fn array(out: &mut String, kind: char, items: impl Iterator<Item = String>) {
    out.push('[');
    out.push(kind);
    out.push(';');
    out.push_str(&items.collect::<Vec<_>>().join(","));
    out.push(']');
}

/// Parse SNBT. Numbers without a suffix are ints, or doubles when they
/// have a point or exponent; `true` and `false` are bytes.
pub fn parse(text: &str) -> Result<Tag> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let tag = p.value().with_context(|| format!("SNBT {text}"))?;
    p.skip_ws();
    if p.i != p.s.len() {
        bail!("SNBT {text}: trailing text at {}", p.i);
    }
    Ok(tag)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            bail!("expected {:?} at {}", c as char, self.i);
        }
        self.i += 1;
        Ok(())
    }

    fn value(&mut self) -> Result<Tag> {
        match self.peek() {
            Some(b'{') => self.compound(),
            Some(b'[') => self.list(),
            Some(b'"' | b'\'') => Ok(Tag::String(self.quoted()?)),
            Some(_) => {
                let word = self.word();
                if word.is_empty() {
                    bail!("expected a value at {}", self.i);
                }
                Ok(scalar(&word))
            }
            None => bail!("expected a value at the end"),
        }
    }

    fn word(&mut self) -> String {
        self.skip_ws();
        let start = self.i;
        while self.i < self.s.len() && bare(self.s[self.i] as char) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn quoted(&mut self) -> Result<String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i).context("unterminated string")?;
            self.i += 1;
            if c == b'\\' {
                out.push(*self.s.get(self.i).context("unterminated string")?);
                self.i += 1;
            } else if c == q {
                break;
            } else {
                out.push(c);
            }
        }
        Ok(String::from_utf8(out)?)
    }

    fn compound(&mut self) -> Result<Tag> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Tag::Compound(entries));
        }
        loop {
            let key = match self.peek() {
                Some(b'"' | b'\'') => self.quoted()?,
                _ => self.word(),
            };
            if key.is_empty() {
                bail!("expected a key at {}", self.i);
            }
            self.expect(b':')?;
            entries.push((key, self.value()?));
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Tag::Compound(entries));
                }
                _ => bail!("expected ',' or '}}' at {}", self.i),
            }
        }
    }

    fn list(&mut self) -> Result<Tag> {
        self.expect(b'[')?;
        if let [k @ (b'B' | b'I' | b'L'), b';', ..] = self.s[self.i..] {
            self.i += 2;
            let mut items = Vec::new();
            while self.peek() != Some(b']') {
                let w = self.word();
                let n: i64 =
                    w.trim_end_matches(['b', 'B', 'L', 'l']).parse().with_context(|| format!("array item {w}"))?;
                items.push(n);
                if self.peek() == Some(b',') {
                    self.i += 1;
                }
            }
            self.i += 1;
            return Ok(match k {
                b'B' => Tag::ByteArray(items.into_iter().map(|n| n as i8).collect()),
                b'I' => Tag::IntArray(items.into_iter().map(|n| n as i32).collect()),
                _ => Tag::LongArray(items),
            });
        }
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Tag::List(0, items));
        }
        loop {
            items.push(self.value()?);
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    let id = items.first().map_or(0, Tag::id);
                    if items.iter().any(|t| t.id() != id) {
                        bail!("a list mixes tag types");
                    }
                    return Ok(Tag::List(id, items));
                }
                _ => bail!("expected ',' or ']' at {}", self.i),
            }
        }
    }
}

/// A bare word: a number with its suffix, a boolean, or a string.
fn scalar(w: &str) -> Tag {
    let (body, suffix) = w.split_at(w.len() - 1);
    let parsed = match suffix {
        "b" | "B" => body.parse().ok().map(Tag::Byte),
        "s" | "S" => body.parse().ok().map(Tag::Short),
        "l" | "L" => body.parse().ok().map(Tag::Long),
        "f" | "F" => body.parse().ok().map(Tag::Float),
        "d" | "D" => body.parse().ok().map(Tag::Double),
        _ => None,
    };
    if let Some(t) = parsed {
        return t;
    }
    if let Ok(i) = w.parse::<i32>() {
        return Tag::Int(i);
    }
    if w.contains(['.', 'e', 'E'])
        && let Ok(d) = w.parse::<f64>()
    {
        return Tag::Double(d);
    }
    match w {
        "true" => Tag::Byte(1),
        "false" => Tag::Byte(0),
        _ => Tag::String(w.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::compound;

    #[test]
    fn every_type_round_trips() {
        let tag = compound(vec![
            ("b", Tag::Byte(-3)),
            ("s", Tag::Short(300)),
            ("i", Tag::Int(7)),
            ("l", Tag::Long(1 << 40)),
            ("f", Tag::Float(0.25)),
            ("d", Tag::Double(1.0)),
            ("str", Tag::String("a \"b\"".into())),
            ("list", Tag::List(6, vec![Tag::Double(0.5), Tag::Double(-2.0)])),
            ("empty", Tag::List(0, vec![])),
            ("ba", Tag::ByteArray(vec![1, -1])),
            ("ia", Tag::IntArray(vec![1, 2, 3])),
            ("la", Tag::LongArray(vec![5])),
            ("Item", compound(vec![("id", Tag::String("minecraft:stone".into())), ("count", Tag::Int(1))])),
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
}
