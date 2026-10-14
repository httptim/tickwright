//! Write a schematic again in every format, saying each data version given,
//! as `<dir>/<version>.nbt`, `.schem` and `.litematic`. The harness loads
//! them in the game (`harness/schematics.sh`).
//!
//!     cargo run -p tickwright-schematic --example rewrite -- <file> <dir> <version>...

use tickwright_schematic::{Format, Schematic};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, dir, versions @ ..] = args.as_slice() else {
        anyhow::bail!("usage: rewrite <file> <dir> <version>...");
    };
    let mut s = Schematic::read(&std::fs::read(file)?)?;
    std::fs::create_dir_all(dir)?;
    for v in versions {
        s.data_version = Some(v.parse()?);
        for (ext, format) in [("nbt", Format::Structure), ("schem", Format::Sponge), ("litematic", Format::Litematic)] {
            std::fs::write(format!("{dir}/{v}.{ext}"), s.write(format))?;
        }
    }
    Ok(())
}
