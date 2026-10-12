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
            Some(_) => {
                let w = self.word()?;
                let n = w.trim_end_matches(['b', 'B', 's', 'S', 'l', 'L', 'f', 'F', 'd', 'D']);
                match n.parse::<f64>() {
                    Ok(x) if !n.is_empty() => Ok(Tag::Number(x)),
                    _ => match w.as_str() {
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
    }
}
