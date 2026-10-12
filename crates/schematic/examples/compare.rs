//! Compare a schematic's blocks with a fixture's.
//!
//!     cargo run -p tickwright-schematic --example compare -- <schematic> <fixture.json>

use tickwright_schematic::Schematic;
use tickwright_trace::{FixtureSource, Transform};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let s = Schematic::read(&std::fs::read(&args[1])?)?;
    let f = FixtureSource::from_json(&std::fs::read_to_string(&args[2])?)?.resolve(Transform::IDENTITY)?;
    let theirs: Vec<String> = f.blocks.iter().map(|(x, y, z, b)| format!("{x} {y} {z} {b}")).collect();
    // The fixture may hold the schematic with a border around it.
    let at = |d: [i32; 3]| -> Vec<String> {
        s.blocks.iter().map(|(p, b)| format!("{} {} {} {b}", p[0] + d[0], p[1] + d[1], p[2] + d[2])).collect()
    };
    let mut best = ([0; 3], 0);
    for dx in 0..4 {
        for dy in 0..4 {
            for dz in 0..4 {
                let n = at([dx, dy, dz]).iter().filter(|b| theirs.contains(b)).count();
                if n > best.1 {
                    best = ([dx, dy, dz], n);
                }
            }
        }
    }
    println!("best offset {:?}", best.0);
    let mine = at(best.0);
    println!("schematic {:?} {} blocks; fixture {:?} {} blocks", s.size, mine.len(), f.size, theirs.len());
    let only_s: Vec<_> = mine.iter().filter(|b| !theirs.contains(b)).collect();
    let only_f: Vec<_> = theirs.iter().filter(|b| !mine.contains(b)).collect();
    println!("{} only in the schematic, {} only in the fixture", only_s.len(), only_f.len());
    for b in only_s.iter().take(10) {
        println!("  schematic: {b}");
    }
    for b in only_f.iter().take(10) {
        println!("  fixture:   {b}");
    }
    println!("containers {:?}\ncomparators {:?}", s.containers(), s.comparators());
    println!("fixture containers {:?}\nfixture comparators {:?}", f.containers, f.comparators);
    Ok(())
}
