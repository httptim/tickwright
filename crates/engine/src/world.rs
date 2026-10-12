//! Block storage: the world in 16×16×16 sections, as the game stores it.
//!
//! Each section keeps a palette of the states it holds and one small index
//! per block, so a big build costs about a byte per block, and space with
//! nothing in it costs nothing: a section is dropped once it is all air.

use std::collections::HashMap;

use crate::pos::Pos;
use crate::registry::StateId;

const SIDE: i32 = 16;
const VOLUME: usize = 16 * 16 * 16;

/// Palette indices fit in a byte until a section holds more than this many
/// different states; past that it stores states directly.
const SMALL_PALETTE: usize = 256;

pub struct World {
    air: StateId,
    sections: HashMap<(i32, i32, i32), Section>,
}

struct Section {
    /// Blocks that are not air, so empty sections can be dropped.
    non_air: u16,
    data: Data,
}

enum Data {
    /// Indices into `palette`, whose first entry is air.
    Paletted { palette: Vec<StateId>, index: Box<[u8; VOLUME]> },
    /// States stored directly.
    Direct(Box<[StateId; VOLUME]>),
}

fn split(pos: Pos) -> ((i32, i32, i32), usize) {
    let key = (pos.x >> 4, pos.y >> 4, pos.z >> 4);
    let i = (((pos.y & (SIDE - 1)) << 8) | ((pos.z & (SIDE - 1)) << 4) | (pos.x & (SIDE - 1))) as usize;
    (key, i)
}

fn join(key: (i32, i32, i32), i: usize) -> Pos {
    let i = i as i32;
    Pos::new(key.0 * SIDE + (i & 15), key.1 * SIDE + (i >> 8), key.2 * SIDE + ((i >> 4) & 15))
}

impl Section {
    fn new(air: StateId) -> Section {
        Section { non_air: 0, data: Data::Paletted { palette: vec![air], index: Box::new([0; VOLUME]) } }
    }

    fn get(&self, i: usize) -> StateId {
        match &self.data {
            Data::Paletted { palette, index } => palette[index[i] as usize],
            Data::Direct(states) => states[i],
        }
    }

    fn set(&mut self, i: usize, state: StateId, air: StateId) {
        let old = self.get(i);
        if old == state {
            return;
        }
        match (old == air, state == air) {
            (true, false) => self.non_air += 1,
            (false, true) => self.non_air -= 1,
            _ => {}
        }
        if let Data::Paletted { palette, index } = &mut self.data {
            let slot = match palette.iter().position(|&s| s == state) {
                Some(slot) => slot,
                None => {
                    if palette.len() == SMALL_PALETTE {
                        Self::compact(palette, index);
                    }
                    palette.push(state);
                    palette.len() - 1
                }
            };
            if slot < SMALL_PALETTE {
                index[i] = slot as u8;
                return;
            }
            // Still too many states after dropping unused ones.
            palette.pop();
            let mut states = Box::new([air; VOLUME]);
            for (s, &k) in states.iter_mut().zip(index.iter()) {
                *s = palette[k as usize];
            }
            self.data = Data::Direct(states);
        }
        if let Data::Direct(states) = &mut self.data {
            states[i] = state;
        }
    }

    /// Drop palette entries no block uses any more, keeping air first.
    fn compact(palette: &mut Vec<StateId>, index: &mut [u8; VOLUME]) {
        let mut used = [false; SMALL_PALETTE];
        used[0] = true;
        for &k in index.iter() {
            used[k as usize] = true;
        }
        let mut remap = [0u8; SMALL_PALETTE];
        let mut kept = Vec::with_capacity(palette.len());
        for (k, &s) in palette.iter().enumerate() {
            if used[k] {
                remap[k] = kept.len() as u8;
                kept.push(s);
            }
        }
        for k in index.iter_mut() {
            *k = remap[*k as usize];
        }
        *palette = kept;
    }
}

impl World {
    pub fn new(air: StateId) -> World {
        World { air, sections: HashMap::new() }
    }

    pub fn get(&self, pos: Pos) -> StateId {
        let (key, i) = split(pos);
        self.sections.get(&key).map_or(self.air, |s| s.get(i))
    }

    pub fn set(&mut self, pos: Pos, state: StateId) {
        let (key, i) = split(pos);
        if state == self.air {
            if let Some(section) = self.sections.get_mut(&key) {
                section.set(i, state, self.air);
                if section.non_air == 0 {
                    self.sections.remove(&key);
                }
            }
            return;
        }
        let air = self.air;
        self.sections.entry(key).or_insert_with(|| Section::new(air)).set(i, state, air);
    }

    /// Every block that is not air, in no particular order.
    pub fn non_air(&self) -> impl Iterator<Item = (Pos, StateId)> + '_ {
        self.sections.iter().flat_map(move |(&key, section)| {
            (0..VOLUME).filter_map(move |i| {
                let s = section.get(i);
                (s != self.air).then(|| (join(key, i), s))
            })
        })
    }

    pub fn sections(&self) -> usize {
        self.sections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: StateId = StateId(0);

    #[test]
    fn blocks_read_back_across_section_borders_and_negative_coordinates() {
        let mut w = World::new(AIR);
        let spots = [Pos::new(0, 0, 0), Pos::new(-1, -1, -1), Pos::new(15, 16, -16), Pos::new(-17, 100, 31)];
        for (n, &p) in spots.iter().enumerate() {
            w.set(p, StateId(n as u32 + 1));
        }
        for (n, &p) in spots.iter().enumerate() {
            assert_eq!(w.get(p), StateId(n as u32 + 1));
        }
        assert_eq!(w.get(Pos::new(1, 0, 0)), AIR);
        let mut all: Vec<_> = w.non_air().collect();
        all.sort();
        let mut expected: Vec<_> = spots.iter().enumerate().map(|(n, &p)| (p, StateId(n as u32 + 1))).collect();
        expected.sort();
        assert_eq!(all, expected);
    }

    #[test]
    fn empty_sections_are_dropped() {
        let mut w = World::new(AIR);
        w.set(Pos::new(3, 3, 3), StateId(5));
        w.set(Pos::new(4, 3, 3), StateId(5));
        assert_eq!(w.sections(), 1);
        w.set(Pos::new(3, 3, 3), AIR);
        assert_eq!(w.sections(), 1);
        w.set(Pos::new(4, 3, 3), AIR);
        assert_eq!(w.sections(), 0);
        w.set(Pos::new(4, 3, 3), AIR);
        assert_eq!(w.sections(), 0);
    }

    #[test]
    fn a_section_with_many_states_keeps_them_all() {
        let mut w = World::new(AIR);
        // More states than a byte can index, then churn through more so the
        // palette has to drop unused entries.
        for i in 0..VOLUME {
            w.set(join((0, 0, 0), i), StateId(1 + (i % 300) as u32));
        }
        for i in 0..VOLUME {
            assert_eq!(w.get(join((0, 0, 0), i)), StateId(1 + (i % 300) as u32));
        }
        let mut w = World::new(AIR);
        for round in 0..10u32 {
            for i in 0..100 {
                w.set(join((0, 0, 0), i), StateId(1 + round * 100 + i as u32));
            }
        }
        for i in 0..100 {
            assert_eq!(w.get(join((0, 0, 0), i)), StateId(901 + i as u32));
        }
        assert!(matches!(w.sections[&(0, 0, 0)].data, Data::Paletted { .. }));
    }
}
