//! Print a schematic's size, block count and block entities.
//!
//!     cargo run -p tickwright-schematic --example inspect -- <file>...

use tickwright_schematic::Schematic;

fn main() -> anyhow::Result<()> {
    for path in std::env::args().skip(1) {
        let s = Schematic::read(&std::fs::read(&path)?).map_err(|e| e.context(path.clone()))?;
        println!(
            "{path}: {:?}, {} blocks, {} block entities, {} containers, {} comparator outputs, data version {:?}",
            s.size,
            s.blocks.len(),
            s.block_entities.len(),
            s.containers().len(),
            s.comparators().len(),
            s.data_version
        );
    }
    Ok(())
}
