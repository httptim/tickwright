//! Rule modules: everything about the game's behaviour that can differ
//! between versions, kept apart from the kernel ([`crate::level`]) that runs
//! it.
//!
//! A [`Module`] describes one game version as a change to an older one:
//! "26.3 is 26.2 plus these blocks behaving differently". Loading a version
//! walks its chain of modules once and resolves it into [`Rules`], a flat
//! table the kernel reads, so the hot loop never branches on the version.
//!
//! Adding a version means adding a module and its generated block data; the
//! kernel does not change. A behaviour that genuinely differs gets its own
//! [`Kind`], and the new module maps the affected blocks to it.

mod v26_3;

use anyhow::{Result, bail};

use crate::blocks::Kind;
use crate::registry::{BlockId, Registry, StateId};

/// One game version's rules, stated as what differs from `base`.
pub struct Module {
    /// The game version, as fixtures and traces name it.
    pub version: &'static str,
    /// The version this one changes, or `None` for the oldest module.
    pub base: Option<&'static str>,
    /// The version's blocks and states, generated from its data reports.
    pub registry: fn() -> Result<&'static Registry>,
    /// The behaviour of each block this module defines or changes, by name
    /// without the namespace. `None` defers to the base.
    pub block: fn(&str) -> Option<Kind>,
}

/// Every version the engine can simulate, newest first.
pub static MODULES: &[Module] = &[v26_3::MODULE];

/// The versions the engine can simulate, newest first.
pub fn versions() -> impl Iterator<Item = &'static str> {
    MODULES.iter().map(|m| m.version)
}

/// A version's rules, resolved from its chain of modules.
pub struct Rules {
    pub version: &'static str,
    pub reg: &'static Registry,
    /// Behaviour by block id. Blocks no module defines are [`Kind::Other`].
    kinds: Vec<Kind>,
}

impl Rules {
    pub fn for_version(version: &str) -> Result<Rules> {
        Rules::resolve(MODULES, version)
    }

    /// Resolve `version` against an explicit list of modules.
    pub fn resolve(modules: &[Module], version: &str) -> Result<Rules> {
        let find = |v: &str| modules.iter().find(|m| m.version == v);
        let Some(top) = find(version) else {
            let known: Vec<_> = modules.iter().map(|m| m.version).collect();
            bail!("no rules for Minecraft {version} (supported: {})", known.join(", "));
        };
        let mut chain = vec![top];
        while let Some(base) = chain.last().unwrap().base {
            let Some(m) = find(base) else {
                bail!("the {} rules build on {base}, which has no module", chain.last().unwrap().version);
            };
            if chain.iter().any(|c| c.version == base) {
                bail!("the rules for {version} build on themselves");
            }
            chain.push(m);
        }

        let reg = (top.registry)()?;
        let kinds = (0..reg.len() as u16)
            .map(|i| {
                let name = &reg.block(BlockId(i)).name;
                let name = name.strip_prefix("minecraft:").unwrap_or(name);
                chain.iter().find_map(|m| (m.block)(name)).unwrap_or(Kind::Other)
            })
            .collect();
        Ok(Rules { version: top.version, reg, kinds })
    }

    pub fn kind(&self, block: BlockId) -> Kind {
        self.kinds[block.0 as usize]
    }

    pub fn kind_of(&self, state: StateId) -> Kind {
        self.kind(self.reg.block_of(state))
    }

    /// The per-block table, for the kernel to own.
    pub(crate) fn into_kinds(self) -> Vec<Kind> {
        self.kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Result<&'static Registry> {
        Registry::get("26.3")
    }

    // A made-up later version where glass conducts like stone and every
    // other block keeps its 26.3 behaviour.
    static TEST_MODULES: &[Module] = &[
        Module {
            version: "test",
            base: Some("26.3"),
            registry,
            block: |name| (name == "glass").then_some(Kind::Solid),
        },
        v26_3::MODULE,
    ];

    #[test]
    fn a_module_changes_only_what_it_names() {
        let base = Rules::resolve(TEST_MODULES, "26.3").unwrap();
        let next = Rules::resolve(TEST_MODULES, "test").unwrap();
        let reg = next.reg;
        let id = |n: &str| reg.block_id(n).unwrap();
        assert_eq!(base.kind(id("glass")), Kind::Glass);
        assert_eq!(next.kind(id("glass")), Kind::Solid);
        assert_eq!(next.kind(id("repeater")), Kind::Repeater);
        assert_eq!(next.kind(id("furnace")), Kind::Other);
    }

    #[test]
    fn unknown_versions_and_broken_chains_fail_by_name() {
        let err = Rules::resolve(TEST_MODULES, "1.12.2").err().unwrap().to_string();
        assert!(err.contains("1.12.2") && err.contains("26.3"), "{err}");
        let orphan = [Module { version: "x", base: Some("y"), registry, block: |_| None }];
        let err = Rules::resolve(&orphan, "x").err().unwrap().to_string();
        assert!(err.contains("build on y"), "{err}");
    }

    #[test]
    fn plant_stems_are_not_logs() {
        let rules = Rules::for_version("26.3").unwrap();
        let kind = |n: &str| rules.kind(rules.reg.block_id(n).unwrap());
        for log in ["crimson_stem", "stripped_warped_stem", "warped_hyphae", "oak_log"] {
            assert_eq!(kind(log), Kind::Log, "{log}");
        }
        for plant in ["pumpkin_stem", "attached_melon_stem", "big_dripleaf_stem", "mushroom_stem"] {
            assert_eq!(kind(plant), Kind::Other, "{plant}");
        }
    }

    #[test]
    fn every_listed_version_resolves() {
        for v in versions() {
            Rules::for_version(v).unwrap();
        }
    }
}
