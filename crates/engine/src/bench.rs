//! A large, always-busy build for measuring simulation speed.
//!
//! `rings` loops of repeaters and dust side by side. Each loop is two rows
//! joined by dust corners, with one repeater placed powered so a pulse runs
//! round it for as long as the simulation lasts: every tick has scheduled
//! ticks, dust recomputing and neighbor updates.

use tickwright_trace::{Level as TraceLevel, ResolvedFixture};

fn wire(sides: &[&str]) -> String {
    let side = |d: &str| if sides.contains(&d) { "side" } else { "none" };
    format!(
        "minecraft:redstone_wire[east={},north={},power=0,south={},west={}]",
        side("east"),
        side("north"),
        side("south"),
        side("west")
    )
}

fn repeater(facing: &str, powered: bool) -> String {
    format!("minecraft:repeater[delay=1,facing={facing},locked=false,powered={powered}]")
}

/// `rings` loops, each `2 * (length + 2)` blocks round, run for `ticks`.
/// `length` is rounded up to an odd number.
pub fn redstone_rings(rings: i32, length: i32, ticks: u32) -> ResolvedFixture {
    let n = length | 1;
    let mut blocks = Vec::new();
    for k in 0..rings {
        let (a, b) = (3 * k, 3 * k + 1);
        for x in 0..=n + 1 {
            blocks.push((x, 0, a, "minecraft:stone".to_string()));
            blocks.push((x, 0, b, "minecraft:stone".to_string()));
        }
        blocks.push((0, 1, a, wire(&["east", "south"])));
        blocks.push((0, 1, b, wire(&["east", "north"])));
        blocks.push((n + 1, 1, a, wire(&["west", "south"])));
        blocks.push((n + 1, 1, b, wire(&["west", "north"])));
        for x in 1..=n {
            // Row a carries the pulse east, row b carries it back west; a
            // wire never sits beside a wire in the other row.
            let row_a = if x % 2 == 1 { repeater("west", x == 1) } else { wire(&["east", "west"]) };
            let row_b = if x % 2 == 0 { repeater("east", false) } else { wire(&["east", "west"]) };
            blocks.push((x, 1, a, row_a));
            blocks.push((x, 1, b, row_b));
        }
    }
    blocks.sort_by_key(|&(x, y, z, _)| (y, z, x));
    ResolvedFixture {
        format: 1,
        id: format!("bench_rings_{rings}x{n}@r0"),
        name: format!("bench_rings_{rings}x{n}"),
        transform: "r0".to_string(),
        ticks,
        levels: vec![TraceLevel::State],
        origin: [0, 100, 0],
        size: [n + 2, 2, 3 * rings - 1],
        margin: 3,
        blocks,
        stimuli: Vec::new(),
        containers: Vec::new(),
        comparators: Vec::new(),
        lecterns: Vec::new(),
        experiments: Vec::new(),
        day_time: None,
        max_chained_updates: None,
        entities: false,
    }
}

#[cfg(test)]
mod tests {
    use tickwright_trace::Kind;

    #[test]
    fn the_pulse_keeps_going_round() {
        let fixture = super::redstone_rings(1, 7, 200);
        let trace = crate::run::run(&fixture, "26.3").unwrap();
        // Every repeater in the loop (4 + 3 for length 7) switches on, and ticks keep coming late on.
        let late_ticks = trace.entries.iter().filter(|e| e.kind == Kind::Tick && e.tick > 150).count();
        assert!(late_ticks > 10, "only {late_ticks} block ticks after tick 150");
        let lit: std::collections::BTreeSet<_> = trace
            .entries
            .iter()
            .filter(|e| {
                e.kind == Kind::Set
                    && e.state.as_deref().is_some_and(|s| s.contains("repeater") && s.contains("powered=true"))
            })
            .map(|e| e.pos)
            .collect();
        assert_eq!(lit.len(), 7, "repeaters that powered: {lit:?}");
    }
}
