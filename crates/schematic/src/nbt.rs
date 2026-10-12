//! Named Binary Tag, the format every schematic is stored in: just enough
//! to read and write whole files (big-endian, usually gzipped).

use std::io::{Read, Write};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    /// A list and its element type id (0 when empty).
    List(u8, Vec<Tag>),
    /// Entries in file order.
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    /// A compound's entry.
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A compound's entry, or an error naming it.
    pub fn req(&self, key: &str) -> Result<&Tag> {
        self.get(key).with_context(|| format!("missing `{key}`"))
    }

    /// Any integer tag widened to `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v as i64),
            Tag::Short(v) => Some(v as i64),
            Tag::Int(v) => Some(v as i64),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Result<i32> {
        self.req(key)?.as_i64().map(|v| v as i32).with_context(|| format!("`{key}` is not a number"))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Tag] {
        match self {
            Tag::List(_, items) => items,
            _ => &[],
        }
    }

    pub fn entries(&self) -> &[(String, Tag)] {
        match self {
            Tag::Compound(e) => e,
            _ => &[],
        }
    }

    /// An int array, or a list of ints, as `i32`s.
    pub fn ints(&self) -> Option<Vec<i32>> {
        match self {
            Tag::IntArray(v) => Some(v.clone()),
            Tag::List(_, items) => items.iter().map(|t| t.as_i64().map(|v| v as i32)).collect(),
            _ => None,
        }
    }
}

/// A compound from `(key, tag)` pairs.
pub fn compound(entries: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A list of tags that share a type.
pub fn list(items: Vec<Tag>) -> Tag {
    Tag::List(items.first().map_or(0, Tag::id), items)
}

/// Read a file's bytes, gzipped or not, into its root name and compound.
pub fn read(bytes: &[u8]) -> Result<(String, Tag)> {
    let mut raw = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw).context("not valid gzip")?;
    } else {
        raw = bytes.to_vec();
    }
    let mut r = Reader { b: &raw, i: 0 };
    let id = r.u8()?;
    if id != 10 {
        bail!("the root tag is not a compound");
    }
    let name = r.string()?;
    let root = r.payload(10, 0)?;
    Ok((name, root))
}

/// Write `root` as a gzipped file under `name`.
pub fn write_gz(name: &str, root: &Tag) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.push(10);
    put_string(&mut raw, name);
    put_payload(&mut raw, root);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.i + n > self.b.len() {
            bail!("the file ends early");
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into()?))
    }

    fn len(&mut self) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 {
            bail!("negative length");
        }
        Ok(n as usize)
    }

    fn string(&mut self) -> Result<String> {
        let n = self.i16()? as u16 as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn payload(&mut self, id: u8, depth: u32) -> Result<Tag> {
        if depth > 512 {
            bail!("nested too deeply");
        }
        Ok(match id {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let elem = self.u8()?;
                let n = self.len()?;
                let mut items = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    items.push(self.payload(elem, depth + 1)?);
                }
                Tag::List(elem, items)
            }
            10 => {
                let mut entries = Vec::new();
                loop {
                    let id = self.u8()?;
                    if id == 0 {
                        break;
                    }
                    let key = self.string()?;
                    entries.push((key, self.payload(id, depth + 1)?));
                }
                Tag::Compound(entries)
            }
            11 => {
                let n = self.len()?;
                Tag::IntArray((0..n).map(|_| self.i32()).collect::<Result<_>>()?)
            }
            12 => {
                let n = self.len()?;
                Tag::LongArray((0..n).map(|_| self.i64()).collect::<Result<_>>()?)
            }
            _ => bail!("unknown tag type {id}"),
        })
    }
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|&b| b as u8));
        }
        Tag::String(s) => put_string(out, s),
        Tag::List(elem, items) => {
            out.push(*elem);
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for t in items {
                put_payload(out, t);
            }
        }
        Tag::Compound(entries) => {
            for (k, v) in entries {
                out.push(v.id());
                put_string(out, k);
                put_payload(out, v);
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let root = compound(vec![
            ("a", Tag::Byte(-3)),
            ("b", list(vec![Tag::Int(1), Tag::Int(2)])),
            ("c", Tag::LongArray(vec![i64::MIN, 7])),
            ("d", compound(vec![("e", Tag::String("hé".into())), ("f", Tag::Double(0.5))])),
            ("g", Tag::List(0, vec![])),
        ]);
        let bytes = write_gz("root", &root);
        assert_eq!(read(&bytes).unwrap(), ("root".to_string(), root));
    }
}
