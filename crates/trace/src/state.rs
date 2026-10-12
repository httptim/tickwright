//! Block states written as strings, the way the game prints them:
//! `minecraft:repeater[delay=1,facing=west,locked=false,powered=false]`.
//!
//! The game prints properties in the order the block defines them, while
//! fixtures are written by hand in any order. Everything is compared in
//! canonical form: namespaced id, properties sorted by name.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockState {
    pub id: String,
    pub props: BTreeMap<String, String>,
}

impl BlockState {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (id, rest) = match s.find('[') {
            Some(i) => (&s[..i], Some(&s[i..])),
            None => (s, None),
        };
        if id.is_empty() {
            bail!("empty block id in {s:?}");
        }
        let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
        let mut props = BTreeMap::new();
        if let Some(rest) = rest {
            let inner = rest
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .with_context(|| format!("unbalanced brackets in {s:?}"))?;
            for pair in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').with_context(|| format!("property without '=' in {s:?}"))?;
                if props.insert(k.trim().to_string(), v.trim().to_string()).is_some() {
                    bail!("property {k:?} given twice in {s:?}");
                }
            }
        }
        Ok(BlockState { id, props })
    }

    pub fn air() -> Self {
        BlockState { id: "minecraft:air".into(), props: BTreeMap::new() }
    }

    pub fn is_air(&self) -> bool {
        matches!(self.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(String::as_str)
    }
}

impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)?;
        if !self.props.is_empty() {
            f.write_str("[")?;
            for (i, (k, v)) in self.props.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{k}={v}")?;
            }
            f.write_str("]")?;
        }
        Ok(())
    }
}

/// Canonical form of a state string, for comparing traces from different sources.
pub fn canonical(s: &str) -> Result<String> {
    Ok(BlockState::parse(s)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_properties_and_adds_namespace() {
        let s = BlockState::parse("repeater[facing=west, delay=2]").unwrap();
        assert_eq!(s.to_string(), "minecraft:repeater[delay=2,facing=west]");
    }

    #[test]
    fn plain_id_round_trips() {
        assert_eq!(canonical("minecraft:stone").unwrap(), "minecraft:stone");
    }

    #[test]
    fn rejects_malformed() {
        assert!(BlockState::parse("minecraft:lever[powered").is_err());
        assert!(BlockState::parse("minecraft:lever[powered]").is_err());
        assert!(BlockState::parse("minecraft:lever[a=1,a=2]").is_err());
    }
}
