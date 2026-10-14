//! SNBT, the text form of NBT that `/summon`, item components and saved
//! entity data use, read the way 26.3's `SnbtGrammar` reads it and written
//! back with every tag's type: bytes, shorts, ints, longs, floats, doubles,
//! strings, lists (which may mix types), compounds, and the typed arrays
//! `[B;..]`, `[I;..]` and `[L;..]`.
//!
//! The one escape the grammar knows that this reader refuses is a named
//! character, `\N{name}`: the engine carries no table of Unicode names, so
//! such a string is an error rather than a guess.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(m) => m.get(key),
            _ => None,
        }
    }

    /// A number tag's value, widened to a double.
    pub fn number(&self) -> Option<f64> {
        Some(match *self {
            Tag::Byte(v) => v as f64,
            Tag::Short(v) => v as f64,
            Tag::Int(v) => v as f64,
            Tag::Long(v) => v as f64,
            Tag::Float(v) => v as f64,
            Tag::Double(v) => v,
            _ => return None,
        })
    }

    /// A number tag as Java's `Number.intValue` reads it: integers
    /// wrapped, floats truncated toward zero and saturated.
    pub fn int_value(&self) -> Option<i32> {
        Some(match *self {
            Tag::Byte(v) => v as i32,
            Tag::Short(v) => v as i32,
            Tag::Int(v) => v,
            Tag::Long(v) => v as i32,
            Tag::Float(v) => v as i32,
            Tag::Double(v) => v as i32,
            _ => return None,
        })
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

    /// A compound's entries, or none.
    pub fn compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(m) => Some(m),
            _ => None,
        }
    }
}

/// SNBT with every tag's type: tags that are equal as NBT write the same
/// text, whatever spelling they were read from, and tags that differ (`1b`
/// and `1`, `[I;1]` and `[1]`, `0.1f` and `0.1d`) write different text.
pub fn write(t: &Tag) -> String {
    let mut out = String::new();
    write_into(t, &mut out, true);
    out
}

/// The text a value compares by when a codec decodes it, as it decodes
/// most item components: numbers by their value whatever their type (`1b`,
/// `1` and `1.0d` all read as 1), and a typed array as a list of numbers.
pub fn write_value(t: &Tag) -> String {
    let mut out = String::new();
    write_into(t, &mut out, false);
    out
}

fn write_into(t: &Tag, out: &mut String, typed: bool) {
    let suffix = |out: &mut String, s: &str| {
        if typed {
            out.push_str(s);
        }
    };
    match t {
        Tag::Byte(v) => {
            out.push_str(&v.to_string());
            suffix(out, "b");
        }
        Tag::Short(v) => {
            out.push_str(&v.to_string());
            suffix(out, "s");
        }
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => {
            out.push_str(&v.to_string());
            suffix(out, "L");
        }
        Tag::Float(v) if typed => {
            out.push_str(&float_text(*v as f64, format!("{v}"), format!("{v:e}")));
            out.push('f');
        }
        Tag::Float(v) => out.push_str(&float_text(*v as f64, format!("{}", *v as f64), format!("{:e}", *v as f64))),
        Tag::Double(v) => {
            out.push_str(&float_text(*v, format!("{v}"), format!("{v:e}")));
            suffix(out, "d");
        }
        Tag::String(s) => quote(s, out),
        Tag::ByteArray(v) => array(out, typed, "B;", v.iter().map(|x| format!("{x}{}", if typed { "b" } else { "" }))),
        Tag::IntArray(v) => array(out, typed, "I;", v.iter().map(|x| x.to_string())),
        Tag::LongArray(v) => array(out, typed, "L;", v.iter().map(|x| format!("{x}{}", if typed { "L" } else { "" }))),
        Tag::List(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_into(x, out, typed);
            }
            out.push(']');
        }
        Tag::Compound(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !k.is_empty() && k.chars().all(unquoted) {
                    out.push_str(k);
                } else {
                    quote(k, out);
                }
                out.push(':');
                write_into(v, out, typed);
            }
            out.push('}');
        }
    }
}

/// A float's shortest round-tripping text, `plain` (`0.25`, `-0`, `1`)
/// unless it is very large or very small, then in exponent form (`1e300`).
fn float_text(v: f64, plain: String, exp: String) -> String {
    if v == 0.0 || (1e-4..1e16).contains(&v.abs()) { plain } else { exp }
}

fn array(out: &mut String, typed: bool, prefix: &str, items: impl Iterator<Item = String>) {
    out.push('[');
    if typed {
        out.push_str(prefix);
    }
    out.push_str(&items.collect::<Vec<_>>().join(","));
    out.push(']');
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Read SNBT as `TagParser.parseFully` does: one value, then nothing but
/// whitespace.
pub fn parse(text: &str) -> Result<Tag> {
    let mut p = Parser { s: text.chars().collect(), i: 0 };
    let t = p.literal().with_context(|| format!("SNBT {text}"))?;
    p.ws();
    if p.i != p.s.len() {
        bail!("SNBT {text}: unexpected text at {}", p.i);
    }
    Ok(t)
}

/// `Character.isWhitespace`, which brigadier's `skipWhitespace` uses.
fn java_whitespace(c: char) -> bool {
    match c {
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{1c}'..='\u{1f}' => true,
        '\u{a0}' | '\u{2007}' | '\u{202f}' => false,
        ' ' | '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' => true,
        '\u{2028}' | '\u{2029}' | '\u{205f}' | '\u{3000}' => true,
        _ => false,
    }
}

/// `StringReader.isAllowedInUnquotedString`.
fn unquoted(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

/// `SnbtGrammar.canStartNumber`.
fn starts_number(c: char) -> bool {
    matches!(c, '+' | '-' | '.' | '0'..='9')
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Byte,
    Short,
    Int,
    Long,
}

fn kind_of(c: char) -> Option<Kind> {
    match c {
        'b' | 'B' => Some(Kind::Byte),
        's' | 'S' => Some(Kind::Short),
        'i' | 'I' => Some(Kind::Int),
        'l' | 'L' => Some(Kind::Long),
        _ => None,
    }
}

/// An integer literal's parts (`SnbtGrammar.IntegerLiteral`).
struct Integer {
    negative: bool,
    radix: u32,
    digits: String,
    signed: Option<bool>,
    kind: Option<Kind>,
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn ws(&mut self) {
        while self.s.get(self.i).copied().is_some_and(java_whitespace) {
            self.i += 1;
        }
    }

    fn at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.at(0)
    }

    /// A terminal character (`StringReaderTerms.character`): whitespace,
    /// then one accepted character, or nothing consumed.
    fn term(&mut self, accept: impl Fn(char) -> bool) -> Option<char> {
        let save = self.i;
        self.ws();
        match self.at(0) {
            Some(c) if accept(c) => {
                self.i += 1;
                Some(c)
            }
            _ => {
                self.i = save;
                None
            }
        }
    }

    fn expect(&mut self, c: char) -> Result<()> {
        if self.term(|x| x == c).is_none() {
            bail!("expected '{c}' at {}", self.i);
        }
        Ok(())
    }

    /// `literal`: a number, a quoted string, a compound, a list, or an
    /// unquoted string or built-in operation.
    fn literal(&mut self) -> Result<Tag> {
        match self.peek() {
            Some(c) if starts_number(c) => self.number(),
            Some('"' | '\'') => Ok(Tag::String(self.quoted()?)),
            Some('{') => self.compound(),
            Some('[') => self.list(),
            Some(_) => self.word_or_builtin(),
            None => bail!("expected a value at the end"),
        }
    }

    /// Values separated by commas, a trailing comma allowed
    /// (`Term.repeatedWithTrailingSeparator`), up to `close`.
    fn entries(&mut self, close: char, mut entry: impl FnMut(&mut Parser) -> Result<()>) -> Result<()> {
        if self.term(|c| c == close).is_some() {
            return Ok(());
        }
        loop {
            entry(self)?;
            if self.term(|c| c == ',').is_some() {
                if self.term(|c| c == close).is_some() {
                    return Ok(());
                }
                continue;
            }
            self.expect(close)?;
            return Ok(());
        }
    }

    fn compound(&mut self) -> Result<Tag> {
        self.expect('{')?;
        let mut m = BTreeMap::new();
        self.entries('}', |p| {
            let key = match p.peek() {
                Some('"' | '\'') => p.quoted()?,
                _ => p.unquoted_word()?,
            };
            if key.is_empty() {
                bail!("an empty key at {}", p.i);
            }
            p.expect(':')?;
            // A repeated key keeps its last value (`buildKeepingLast`).
            let v = p.literal()?;
            m.insert(key, v);
            Ok(())
        })?;
        Ok(Tag::Compound(m))
    }

    fn list(&mut self) -> Result<Tag> {
        self.expect('[')?;
        let save = self.i;
        let prefix = match self.term(|c| matches!(c, 'B' | 'I' | 'L')) {
            Some(c) if self.term(|x| x == ';').is_some() => Some(c),
            _ => {
                self.i = save;
                None
            }
        };
        let Some(prefix) = prefix else {
            let mut items = Vec::new();
            self.entries(']', |p| {
                items.push(p.literal()?);
                Ok(())
            })?;
            return Ok(Tag::List(items));
        };
        let mut values = Vec::new();
        self.entries(']', |p| {
            values.push(p.array_entry(prefix)?);
            Ok(())
        })?;
        Ok(match prefix {
            'B' => Tag::ByteArray(values.into_iter().map(|v| v as i8).collect()),
            'I' => Tag::IntArray(values.into_iter().map(|v| v as i32).collect()),
            _ => Tag::LongArray(values),
        })
    }

    /// An entry of a typed array: an integer whose type, if it names one,
    /// the array allows (`ArrayPrefix.isAllowed`).
    fn array_entry(&mut self, prefix: char) -> Result<i64> {
        let n = self.integer()?.with_context(|| format!("expected an integer at {}", self.i))?;
        let allowed = match (prefix, n.kind) {
            (_, None) => true,
            ('B', Some(k)) => k == Kind::Byte,
            ('I', Some(k)) => k != Kind::Long,
            (_, Some(_)) => true,
        };
        if !allowed {
            bail!("a [{prefix};] array can't hold that element type");
        }
        let default = match prefix {
            'B' => Kind::Byte,
            'I' => Kind::Int,
            _ => Kind::Long,
        };
        integer_value(&n, n.kind.unwrap_or(default))
    }

    /// A number: a float if it has a point, an exponent or an `f`/`d`
    /// suffix, else an integer (`float_literal`, then `integer_literal`).
    fn number(&mut self) -> Result<Tag> {
        if let Some(t) = self.float()? {
            return Ok(t);
        }
        let n = self.integer()?.with_context(|| format!("expected a number at {}", self.i))?;
        let kind = n.kind.unwrap_or(Kind::Int);
        let v = integer_value(&n, kind)?;
        Ok(match kind {
            Kind::Byte => Tag::Byte(v as i8),
            Kind::Short => Tag::Short(v as i16),
            Kind::Int => Tag::Int(v as i32),
            Kind::Long => Tag::Long(v),
        })
    }

    fn sign(&mut self) -> bool {
        self.term(|c| c == '+' || c == '-') == Some('-')
    }

    /// A run of digits in `radix`, with underscores between them
    /// (`NumberRunParseRule`, which skips whitespace first).
    fn run(&mut self, radix: u32) -> Result<Option<String>> {
        let save = self.i;
        self.ws();
        let start = self.i;
        while self.at(0).is_some_and(|c| c.is_digit(radix) || c == '_') {
            self.i += 1;
        }
        let run: String = self.s[start..self.i].iter().collect();
        if run.is_empty() {
            self.i = save;
            return Ok(None);
        }
        if run.starts_with('_') || run.ends_with('_') {
            bail!("an underscore can't start or end the number {run}");
        }
        Ok(Some(run.replace('_', "")))
    }

    fn exponent(&mut self) -> Result<Option<String>> {
        let save = self.i;
        if self.term(|c| c == 'e' || c == 'E').is_none() {
            return Ok(None);
        }
        let negative = self.sign();
        match self.run(10)? {
            Some(d) => Ok(Some(format!("{}{d}", if negative { "-" } else { "" }))),
            None => {
                self.i = save;
                Ok(None)
            }
        }
    }

    fn float(&mut self) -> Result<Option<Tag>> {
        let start = self.i;
        let negative = self.sign();
        let whole = self.run(10)?;
        let (fraction, exponent, suffix);
        if self.term(|c| c == '.').is_some() {
            fraction = self.run(10)?;
            if whole.is_none() && fraction.is_none() {
                bail!("expected digits after '.' at {}", self.i);
            }
            exponent = self.exponent()?;
            suffix = self.term(|c| matches!(c, 'f' | 'F' | 'd' | 'D'));
        } else if whole.is_some() {
            fraction = None;
            exponent = self.exponent()?;
            suffix = self.term(|c| matches!(c, 'f' | 'F' | 'd' | 'D'));
            if exponent.is_none() && suffix.is_none() {
                self.i = start;
                return Ok(None);
            }
        } else {
            self.i = start;
            return Ok(None);
        }
        let text = format!(
            "{}{}.{}e{}",
            if negative { "-" } else { "" },
            whole.as_deref().unwrap_or("0"),
            fraction.as_deref().unwrap_or("0"),
            exponent.as_deref().unwrap_or("0")
        );
        Ok(Some(match suffix {
            Some('f' | 'F') => {
                let v: f32 = text.parse()?;
                if !v.is_finite() {
                    bail!("{text} is too large for a float");
                }
                Tag::Float(v)
            }
            _ => {
                let v: f64 = text.parse()?;
                if !v.is_finite() {
                    bail!("{text} is too large for a double");
                }
                Tag::Double(v)
            }
        }))
    }

    /// An integer literal: a sign, `0x` hex, `0b` binary or decimal digits,
    /// and a suffix: `b`, `s`, `i` or `l`, each optionally after `u`
    /// (unsigned) or `s` (signed). `None` when there are no digits.
    fn integer(&mut self) -> Result<Option<Integer>> {
        let start = self.i;
        let negative = self.sign();
        let (radix, digits) = if self.term(|c| c == '0').is_some() {
            if self.term(|c| c == 'x' || c == 'X').is_some() {
                (16, self.run(16)?.with_context(|| format!("expected hex digits at {}", self.i))?)
            } else {
                let save = self.i;
                let binary = match self.term(|c| c == 'b' || c == 'B') {
                    Some(_) => self.run(2)?,
                    None => None,
                };
                match binary {
                    Some(b) => (2, b),
                    None => {
                        self.i = save;
                        if self.run(10)?.is_some() {
                            bail!("a number can't start with 0 at {}", self.i);
                        }
                        (10, "0".to_string())
                    }
                }
            }
        } else {
            match self.run(10)? {
                Some(d) => (10, d),
                None => {
                    self.i = start;
                    return Ok(None);
                }
            }
        };
        let save = self.i;
        let mut signed = None;
        let mut kind = None;
        for (prefix, is_signed) in [('u', false), ('s', true)] {
            if self.term(|c| c.eq_ignore_ascii_case(&prefix)).is_some() {
                if let Some(k) = self.term(|c| kind_of(c).is_some()) {
                    signed = Some(is_signed);
                    kind = kind_of(k);
                    break;
                }
                self.i = save;
            }
        }
        if kind.is_none()
            && let Some(k) = self.term(|c| kind_of(c).is_some())
        {
            kind = kind_of(k);
        }
        Ok(Some(Integer { negative, radix, digits, signed, kind }))
    }

    fn quoted(&mut self) -> Result<String> {
        let q = self.term(|c| c == '"' || c == '\'').context("expected a string")?;
        let mut out = String::new();
        // A high surrogate from an escape, waiting for its low half.
        let mut high: Option<u32> = None;
        loop {
            let Some(c) = self.at(0) else { bail!("an unterminated string") };
            self.i += 1;
            if c == q || c != '\\' {
                if high.is_some() {
                    bail!("a lone surrogate in a string");
                }
                if c == q {
                    return Ok(out);
                }
                out.push(c);
                continue;
            }
            let Some(e) = self.term(|_| true) else { bail!("an unterminated string") };
            let code = match e {
                'b' => '\u{8}' as u32,
                's' => ' ' as u32,
                't' => '\t' as u32,
                'n' => '\n' as u32,
                'f' => '\u{c}' as u32,
                'r' => '\r' as u32,
                '\\' | '\'' | '"' => e as u32,
                'x' | 'u' | 'U' => {
                    let n = match e {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let hex: String = (0..n).map_while(|k| self.at(k).filter(char::is_ascii_hexdigit)).collect();
                    if hex.len() != n {
                        bail!("expected {n} hex digits after \\{e}");
                    }
                    self.i += n;
                    let cp = u32::from_str_radix(&hex, 16)?;
                    if cp > 0x10FFFF {
                        bail!("U+{cp:08X} is not a code point");
                    }
                    cp
                }
                'N' => {
                    self.expect('{')?;
                    let start = self.i;
                    while self.at(0).is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ') {
                        self.i += 1;
                    }
                    let name: String = self.s[start..self.i].iter().collect();
                    bail!("the character name \\N{{{name}}} is not known here; write the character or a \\u escape")
                }
                _ => bail!("an unknown escape \\{e}"),
            };
            match (high.take(), code) {
                (None, 0xD800..=0xDBFF) => high = Some(code),
                (Some(h), 0xDC00..=0xDFFF) => {
                    let cp = 0x10000 + ((h - 0xD800) << 10) + (code - 0xDC00);
                    out.push(char::from_u32(cp).context("a bad surrogate pair")?);
                }
                (None, cp) => out.push(char::from_u32(cp).context("a lone surrogate in a string")?),
                (Some(_), _) => bail!("a lone surrogate in a string"),
            }
        }
    }

    fn unquoted_word(&mut self) -> Result<String> {
        self.ws();
        let start = self.i;
        while self.at(0).is_some_and(unquoted) {
            self.i += 1;
        }
        if start == self.i {
            bail!("expected a value at {}", self.i);
        }
        Ok(self.s[start..self.i].iter().collect())
    }

    /// An unquoted string, `true` or `false` (in any case), or a built-in
    /// operation: `bool(x)` or `uuid("...")`.
    fn word_or_builtin(&mut self) -> Result<Tag> {
        let w = self.unquoted_word()?;
        if w.starts_with(starts_number) {
            bail!("an unquoted string can't start with {w}");
        }
        if self.term(|c| c == '(').is_some() {
            let mut args = Vec::new();
            self.entries(')', |p| {
                args.push(p.literal()?);
                Ok(())
            })?;
            return match (w.as_str(), args.as_slice()) {
                ("bool", [a]) => match a.number() {
                    Some(n) => Ok(Tag::Byte((n != 0.0) as i8)),
                    None => bail!("bool() takes a number or a boolean"),
                },
                ("uuid", [a]) => match a.string().and_then(uuid_ints) {
                    Some(v) => Ok(Tag::IntArray(v)),
                    None => bail!("uuid() takes a UUID string"),
                },
                _ => bail!("no operation {w}/{}", args.len()),
            };
        }
        Ok(if w.eq_ignore_ascii_case("true") {
            Tag::Byte(1)
        } else if w.eq_ignore_ascii_case("false") {
            Tag::Byte(0)
        } else {
            Tag::String(w)
        })
    }
}

/// An integer's value as `IntegerLiteral.create` parses it for `kind`:
/// decimal is signed and hex and binary unsigned unless the suffix says,
/// in range for the type, an unsigned value wrapped into it.
fn integer_value(n: &Integer, kind: Kind) -> Result<i64> {
    let signed = n.signed.unwrap_or(n.radix == 10);
    if !signed && n.negative {
        bail!("an unsigned number can't be negative");
    }
    let text = if n.negative { format!("-{}", n.digits) } else { n.digits.clone() };
    let bits = match kind {
        Kind::Byte => 8,
        Kind::Short => 16,
        Kind::Int => 32,
        Kind::Long => 64,
    };
    let v = i128::from_str_radix(&text, n.radix).map_err(|e| anyhow!("{text}: {e}"))?;
    let (lo, hi) = if signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) } else { (0, (1i128 << bits) - 1) };
    if v < lo || v > hi {
        bail!("{text} is out of range");
    }
    // An unsigned value wraps into the signed type, as Java's casts do.
    Ok(if v > (1i128 << (bits - 1)) - 1 { (v - (1i128 << bits)) as i64 } else { v as i64 })
}

/// `UUIDUtil.uuidToIntArray(UUID.fromString(s))`, as leniently as
/// `UUID.fromString` reads: five groups of hex digits.
fn uuid_ints(s: &str) -> Option<Vec<i32>> {
    let parts: Vec<&str> = s.split('-').collect();
    if s.len() > 36 || parts.len() != 5 {
        return None;
    }
    let p: Vec<i64> = parts.iter().map(|g| i64::from_str_radix(g, 16).ok()).collect::<Option<_>>()?;
    let most = ((((p[0] & 0xffff_ffff) << 16) | (p[1] & 0xffff)) << 16) | (p[2] & 0xffff);
    let least = ((p[3] & 0xffff) << 48) | (p[4] & 0xffff_ffff_ffff);
    Some(vec![(most >> 32) as i32, most as i32, (least >> 32) as i32, least as i32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summon_tags() {
        let t = parse(r#"{Item:{id:"stone",count:3},PickupDelay:10}"#).unwrap();
        assert_eq!(t.get("Item").unwrap().get("id").unwrap().string(), Some("stone"));
        assert_eq!(t.get("Item").unwrap().get("count").unwrap(), &Tag::Int(3));
        let t = parse("{fuse:20s, Motion:[0.1d,-2.5d,0d,],}").unwrap();
        assert_eq!(t.get("fuse").unwrap(), &Tag::Short(20));
        assert_eq!(t.get("Motion"), Some(&Tag::List(vec![Tag::Double(0.1), Tag::Double(-2.5), Tag::Double(0.0)])));
        let t = parse("{UUID:[I;1,-2,3,4],Empty:[B;], mixed:[1b,\"a\",{}], 1a:2}").unwrap();
        assert_eq!(t.get("UUID").unwrap(), &Tag::IntArray(vec![1, -2, 3, 4]));
        assert_eq!(t.get("Empty").unwrap(), &Tag::ByteArray(vec![]));
        assert_eq!(
            t.get("mixed").unwrap(),
            &Tag::List(vec![Tag::Byte(1), Tag::String("a".into()), parse("{}").unwrap()])
        );
        assert_eq!(parse("{a:1,a:2}").unwrap().get("a"), Some(&Tag::Int(2)));
        assert_eq!(parse("[B]").unwrap(), Tag::List(vec![Tag::String("B".into())]));
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
    }

    #[test]
    fn numbers_read_as_the_game_reads_them() {
        let n = |s: &str| parse(s).unwrap_or_else(|e| panic!("{s}: {e:#}"));
        assert_eq!(n("1b"), Tag::Byte(1));
        assert_eq!(n("1s"), Tag::Short(1));
        assert_eq!(n("1"), Tag::Int(1));
        assert_eq!(n("1L"), Tag::Long(1));
        assert_eq!(n("1.0f"), Tag::Float(1.0));
        assert_eq!(n("1."), Tag::Double(1.0));
        assert_eq!(n("1.0"), Tag::Double(1.0));
        assert_eq!(n("1d"), Tag::Double(1.0));
        assert_eq!(n("1e3"), Tag::Double(1000.0));
        assert_eq!(n("01.5"), Tag::Double(1.5));
        assert_eq!(n("-.5"), Tag::Double(-0.5));
        assert_eq!(n("true"), Tag::Byte(1));
        assert_eq!(n("TRUE"), Tag::Byte(1));
        assert_eq!(n("False"), Tag::Byte(0));
        assert_eq!(n("0x01"), Tag::Int(1));
        assert_eq!(n("0x1f"), Tag::Int(31));
        assert_eq!(n("0xFFub"), Tag::Byte(-1));
        assert_eq!(n("0xFFFFFFFF"), Tag::Int(-1));
        assert_eq!(n("-0x80sb"), Tag::Byte(-128));
        assert_eq!(n("0b101"), Tag::Int(5));
        assert_eq!(n("0b"), Tag::Byte(0));
        assert_eq!(n("1_0"), Tag::Int(10));
        assert_eq!(n("_1"), Tag::String("_1".into()));
        assert_eq!(n("1ub"), Tag::Byte(1));
        assert_eq!(n("200ub"), Tag::Byte(-56));
        assert_eq!(n("-1sb"), Tag::Byte(-1));
        assert_eq!(n("1 b"), Tag::Byte(1));
        assert_eq!(n("1 . 5"), Tag::Double(1.5));
        assert_eq!(n("9007199254740993L"), Tag::Long(9007199254740993));
        assert_eq!(n("bool(2)"), Tag::Byte(1));
        assert_eq!(n("bool(0.0,)"), Tag::Byte(0));
        assert_eq!(n(r#"uuid("00000001-0000-0002-0000-000300000004")"#), Tag::IntArray(vec![1, 2, 3, 4]));
        assert_eq!(n(r#"uuid("1-2-3-4-5")"#), Tag::IntArray(vec![1, 0x0002_0003, 0x0004_0000, 5]));
        assert_eq!(n("[I;1b,2s]"), Tag::IntArray(vec![1, 2]));
        assert_eq!(n("[ L ; 1,2L]"), Tag::LongArray(vec![1, 2]));
        assert_eq!(n("[B;200ub]"), Tag::ByteArray(vec![-56]));
        for bad in [
            "01",
            "128b",
            "-1ub",
            "1_",
            "[B;1L]",
            "[I;1L]",
            "[B;200]",
            "1x",
            "0x",
            "1.5e",
            "1.5b",
            ".",
            "1e999",
            "1e40f",
            "-x",
            "{:1}",
            "{\"\":1}",
            "[,]",
            "[1,,]",
            "nope(1)",
            "bool()",
            "bool(\"a\")",
            "uuid(\"x\")",
        ] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn escapes_decode_and_strings_end() {
        let s = |t: &str| parse(t).unwrap_or_else(|e| panic!("{t}: {e:#}")).string().unwrap().to_string();
        assert_eq!(s(r#""a\nb\tc\sd""#), "a\nb\tc d");
        assert_eq!(s(r#""\x41B\U00000043""#), "ABC");
        assert_eq!(s(r#""😀""#), "\u{1F600}");
        assert_eq!(s(r#"'say "hi"'"#), "say \"hi\"");
        assert_eq!(s(r#""it's""#), "it's");
        assert_eq!(s(r#""\ n""#), "\n");
        for bad in [r#""a\"#, r#""a\""#, "'a", r#""\q""#, r#""\x4""#, r#""\uD800""#, r#""\N{SNOWMAN}""#] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn typed_text_tells_types_apart() {
        let w = |s: &str| write(&parse(s).unwrap());
        let all = ["1b", "1s", "1", "1L", "1.0f", "1.0d", "[I;1,2]", "[1,2]", "0.1f", "0.10000000149011612d", "-0.0d"];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(w(a), w(b), "{a} and {b}");
            }
        }
        assert_eq!(w("true"), w("1b"));
        assert_ne!(w("9007199254740993L"), w("9007199254740992L"));
        assert_eq!(w(r#"{b: 1b, "a": 'x"y', "c d": [0.5d, 2s]}"#), r#"{a:"x\"y",b:1b,"c d":[0.5d,2s]}"#);
        for text in [
            r#"{a:[L;1,2],b:"\n\u0001",c:[1b,"x"],d:1.5f,e:-0.0f,f:1e300,g:3.4028235e38f,h:[B;-1b]}"#,
            "{f:0.1f,d:0.1,big:1e20,small:1e-30f}",
        ] {
            let t = parse(text).unwrap();
            assert_eq!(parse(&write(&t)).unwrap(), t, "{text}");
        }
        // Read by a codec, types fall away.
        let v = |s: &str| write_value(&parse(s).unwrap());
        assert_eq!(v("1b"), v("1.0d"));
        assert_eq!(v("[I;1,2]"), v("[1,2]"));
        assert_ne!(v("0.1f"), v("0.1d"));
        assert_ne!(v("9007199254740993L"), v("9007199254740992L"));
    }
}
