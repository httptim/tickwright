//! Just enough of the game's SNBT syntax to read `/summon` data tags.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Number(f64),
    String(String),
    Compound(BTreeMap<String, Tag>),
    List(Vec<Tag>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(m) => m.get(key),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Tag::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn string(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        match self {
            Tag::Compound(m) => m.keys().map(|k| k.as_str()).collect(),
            _ => Vec::new(),
        }
    }
}

/// Canonical SNBT: every key and string quoted, a compound's keys in
/// order, numbers as plain decimals without a type. Tags that read the same
/// write the same, whatever spacing, quoting or number suffixes they were
/// written with.
pub fn write(t: &Tag) -> String {
    let mut out = String::new();
    write_into(t, &mut out);
    out
}

fn write_into(t: &Tag, out: &mut String) {
    match t {
        Tag::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{n}"));
            }
        }
        Tag::String(s) => quote(s, out),
        Tag::Compound(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(k, out);
                out.push(':');
                write_into(v, out);
            }
            out.push('}');
        }
        Tag::List(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_into(x, out);
            }
            out.push(']');
        }
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

pub fn parse(text: &str) -> Result<Tag> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let t = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        bail!("trailing text in {text}");
    }
    Ok(t)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            bail!("expected '{}' at {}", c as char, self.i);
        }
        self.i += 1;
        Ok(())
    }

    fn value(&mut self) -> Result<Tag> {
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Tag::Compound(m));
                }
                loop {
                    let k = self.word()?;
                    self.expect(b':')?;
                    m.insert(k, self.value()?);
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Tag::Compound(m));
                        }
                        _ => bail!("expected ',' or '}}' at {}", self.i),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                // Typed arrays read as plain lists: `[I;1,2]`.
                if let [b'B' | b'I' | b'L', b';', ..] = self.s[self.i..] {
                    self.i += 2;
                }
                let mut v = Vec::new();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Tag::List(v));
                }
                loop {
                    v.push(self.value()?);
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Tag::List(v));
                        }
                        _ => bail!("expected ',' or ']' at {}", self.i),
                    }
                }
            }
            // A quoted value is always a string, whatever it spells: "1",
            // "7b" and "true" are names, not numbers.
            Some(b'"' | b'\'') => Ok(Tag::String(self.word()?)),
            Some(_) => {
                let w = self.word()?;
                // At most one type suffix, as the game reads it.
                let (n, suffix) = match w.char_indices().last() {
                    Some((i, c)) if "bBsSlLfFdD".contains(c) && i > 0 => (&w[..i], Some(c.to_ascii_lowercase())),
                    _ => (w.as_str(), None),
                };
                match n.parse::<f64>() {
                    // A float tag holds a float, which Java widens when it
                    // reads it as a double (`0.1f` is 0.10000000149011612).
                    Ok(x) if suffix == Some('f') => Ok(Tag::Number(x as f32 as f64)),
                    Ok(x) if !n.is_empty() && x.is_finite() => Ok(Tag::Number(x)),
                    _ => match w.as_str() {
                        // Unquoted `true` and `false` are bytes in SNBT.
                        "true" => Ok(Tag::Number(1.0)),
                        "false" => Ok(Tag::Number(0.0)),
                        _ => Ok(Tag::String(w)),
                    },
                }
            }
            None => bail!("unexpected end"),
        }
    }

    fn word(&mut self) -> Result<String> {
        match self.peek() {
            Some(q @ (b'"' | b'\'')) => {
                self.i += 1;
                let mut out = Vec::new();
                while self.i < self.s.len() && self.s[self.i] != q {
                    if self.s[self.i] == b'\\' {
                        self.i += 1;
                    }
                    out.push(self.s[self.i]);
                    self.i += 1;
                }
                self.expect(q)?;
                Ok(String::from_utf8(out)?)
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric() || b"_-.+".contains(&self.s[self.i]))
                {
                    self.i += 1;
                }
                if start == self.i {
                    bail!("expected a value at {}", self.i);
                }
                Ok(String::from_utf8(self.s[start..self.i].to_vec())?)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summon_tags() {
        let t = parse(r#"{Item:{id:"stone",count:3},PickupDelay:10}"#).unwrap();
        assert_eq!(t.get("Item").unwrap().get("id").unwrap().string(), Some("stone"));
        assert_eq!(t.get("Item").unwrap().get("count").unwrap().number(), Some(3.0));
        assert_eq!(t.get("PickupDelay").unwrap().number(), Some(10.0));
        let t = parse("{fuse:20s, Motion:[0.1d,-2.5d,0d]}").unwrap();
        assert_eq!(t.get("fuse").unwrap().number(), Some(20.0));
        assert_eq!(t.get("Motion"), Some(&Tag::List(vec![Tag::Number(0.1), Tag::Number(-2.5), Tag::Number(0.0)])));
        let t = parse("{UUID:[I;1,-2,3,4],Empty:[B;]}").unwrap();
        assert_eq!(t.get("UUID").unwrap(), &Tag::List([1.0, -2.0, 3.0, 4.0].map(Tag::Number).to_vec()));
    }

    #[test]
    fn quoted_values_stay_strings() {
        for (a, b) in [("1", "01"), ("7", "7b"), ("true", "1"), ("1.0", "1"), ("1b", "1")] {
            let ta = parse(&format!(r#"{{n:"{a}"}}"#)).unwrap();
            let tb = parse(&format!(r#"{{n:"{b}"}}"#)).unwrap();
            assert_eq!(ta.get("n").unwrap().string(), Some(a));
            assert_ne!(write(&ta), write(&tb), "{a} and {b}");
        }
        assert_eq!(parse("{n:'7b'}").unwrap().get("n").unwrap().string(), Some("7b"));
        // Unquoted, they are numbers.
        assert_eq!(parse("{n:7b}").unwrap().get("n").unwrap().number(), Some(7.0));
        assert_eq!(parse("{n:true}").unwrap().get("n").unwrap().number(), Some(1.0));
        assert_eq!(parse("{n:0.1f}").unwrap().get("n").unwrap().number(), Some(0.1f32 as f64));
        assert_eq!(parse("{n:0.1d}").unwrap().get("n").unwrap().number(), Some(0.1));
        // A word that only looks like a number with a suffix is a string.
        assert_eq!(parse("{n:bb}").unwrap().get("n").unwrap().string(), Some("bb"));
    }

    #[test]
    fn canonical_text_ignores_spelling() {
        let a = parse(r#"{b: 1b, "a": 'x"y', c: [0.5d, 2s]}"#).unwrap();
        let b = parse(r#"{a:"x\"y",c:[0.5,2],b:1}"#).unwrap();
        assert_eq!(write(&a), r#"{"a":"x\"y","b":1,"c":[0.5,2]}"#);
        assert_eq!(write(&a), write(&b));
        assert_eq!(parse(&write(&a)).unwrap(), a);
    }
}
