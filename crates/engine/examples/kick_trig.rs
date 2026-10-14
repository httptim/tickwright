//! Compares the JVM's `Math.sin` and `Math.cos` of TNT kick angles, as
//! `harness/kick-trig.sh` records them, with the engine's (`crate::math`).
//!
//! ```sh
//! cargo run --release -p tickwright-engine --example kick_trig -- [--diffs <tsv>] <file>...
//! ```
//!
//! Checks that each recorded angle is the one the engine draws from the same
//! seed, then compares every sine and cosine bit for bit. Prints the first
//! few differences of each file, writes all of them to `--diffs`, and fails
//! if there are any.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use tickwright_engine::math;
use tickwright_engine::random::JavaRandom;

/// Differences printed per file; `--diffs` gets them all.
const SHOWN: usize = 10;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    let mut diffs_path = None;
    if args.peek().map(String::as_str) == Some("--diffs") {
        args.next();
        diffs_path = Some(args.next().expect("--diffs needs a path"));
    }
    let files: Vec<String> = args.collect();
    if files.is_empty() {
        eprintln!("usage: kick_trig [--diffs <tsv>] <file>...");
        return ExitCode::FAILURE;
    }

    let mut diffs = String::from("file\tdraw\tfunction\tangle\tjvm\tengine\n");
    let mut total = 0;
    for path in &files {
        let data = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let word = |i: usize| u64::from_le_bytes(data[8 * i..8 * i + 8].try_into().unwrap());
        let seed = word(0) as i64;
        let count = word(1) as usize;
        assert_eq!(data.len(), 16 + 24 * count, "{path}: expected {count} records");
        let name = Path::new(path).file_stem().unwrap().to_string_lossy();

        let mut random = JavaRandom::new(seed);
        let (mut sin_off, mut cos_off) = (0, 0);
        for i in 0..count {
            let angle = f64::from_bits(word(2 + 3 * i));
            // As in entity::tnt::new_primed.
            let drawn = random.next_double() * 6.2831854820251465;
            assert_eq!(drawn.to_bits(), angle.to_bits(), "{path}: draw {i} is {angle:?} in the JVM but {drawn:?} here");
            let jvm = [f64::from_bits(word(3 + 3 * i)), f64::from_bits(word(4 + 3 * i))];
            let engine = [math::sin(angle), math::cos(angle)];
            for (f, function) in ["sin", "cos"].into_iter().enumerate() {
                if jvm[f].to_bits() == engine[f].to_bits() {
                    continue;
                }
                let n = if f == 0 { &mut sin_off } else { &mut cos_off };
                *n += 1;
                let line = format!("{name}\t{i}\t{function}\t{angle:?}\t{:?}\t{:?}", jvm[f], engine[f]);
                if sin_off + cos_off <= SHOWN {
                    println!("  {line}");
                }
                writeln!(diffs, "{line}").unwrap();
            }
        }
        println!("{name}: {count} angles from seed {seed}; sin differs on {sin_off}, cos on {cos_off}");
        total += sin_off + cos_off;
    }
    if let Some(p) = diffs_path {
        std::fs::write(&p, diffs).unwrap_or_else(|e| panic!("{p}: {e}"));
    }
    if total == 0 {
        println!("every sine and cosine matches the engine's bit for bit");
        ExitCode::SUCCESS
    } else {
        println!("{total} results differ from the engine's");
        ExitCode::FAILURE
    }
}
