//! `tickwright`: tools for fixtures and traces.
//!
//! - `resolve` turns hand-written fixtures into one explicit fixture per
//!   orientation, which is what the capture harness and the engine load.
//! - `run` simulates resolved fixtures with the engine and writes traces.
//! - `diff` compares two traces and reports the first divergence.
//! - `import` turns a `.litematic`, `.schem` or structure `.nbt` into a
//!   fixture; `export` writes a fixture's build as one.
//! - `bench` times the engine on a large dust-and-repeater build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tickwright_schematic::{Format, Schematic};
use tickwright_trace::{FixtureSource, Level, ResolvedFixture, Trace, Transform, compare};

#[derive(Parser)]
#[command(name = "tickwright", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate fixtures and write one resolved fixture per orientation.
    Resolve {
        /// Fixture files or directories to search for `*.json`.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Directory for the resolved fixtures.
        #[arg(long)]
        out: PathBuf,
    },
    /// Run resolved fixtures through the engine and write one trace each.
    Run {
        /// Resolved fixture files or directories to search for `*.json`.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Directory for the traces.
        #[arg(long)]
        out: PathBuf,
        /// Minecraft version whose rules to simulate.
        #[arg(long, default_value = "26.3")]
        mc_version: String,
    },
    /// Turn a `.litematic`, `.schem` or structure `.nbt` into a fixture.
    Import {
        schematic: PathBuf,
        /// The fixture file to write.
        #[arg(long)]
        out: PathBuf,
        /// Fixture name; defaults to the schematic's file stem.
        #[arg(long)]
        name: Option<String>,
        /// Game ticks the fixture records.
        #[arg(long, default_value_t = 100)]
        ticks: u32,
    },
    /// Write a fixture's build as a schematic; the extension of `out`
    /// (`.litematic`, `.schem` or `.nbt`) picks the format.
    Export {
        fixture: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Time the engine on a large dust-and-repeater build.
    Bench {
        /// Number of repeater loops.
        #[arg(long, default_value_t = 32)]
        rings: i32,
        /// Components along each side of a loop.
        #[arg(long, default_value_t = 63)]
        length: i32,
        #[arg(long, default_value_t = 2000)]
        ticks: u32,
        /// Also write the build as a resolved fixture, for the browser run.
        #[arg(long)]
        write: Option<PathBuf>,
    },
    /// Compare an expected trace with an actual one.
    Diff {
        expected: PathBuf,
        actual: PathBuf,
        #[arg(long, value_enum, default_value = "order")]
        level: LevelArg,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum LevelArg {
    State,
    Phase,
    Order,
}

impl From<LevelArg> for Level {
    fn from(l: LevelArg) -> Self {
        match l {
            LevelArg::State => Level::State,
            LevelArg::Phase => Level::Phase,
            LevelArg::Order => Level::Order,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Resolve { inputs, out } => {
            let files = collect_json(&inputs)?;
            fs::create_dir_all(&out)?;
            let mut written = 0;
            for file in &files {
                let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
                let fixture = FixtureSource::from_json(&text).with_context(|| format!("in {}", file.display()))?;
                for resolved in fixture.resolve_all()? {
                    let path = out.join(format!("{}.json", resolved.id));
                    fs::write(&path, serde_json::to_string_pretty(&resolved)? + "\n")?;
                    written += 1;
                }
            }
            println!("resolved {} fixture(s) into {written} file(s) in {}", files.len(), out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Run { inputs, out, mc_version } => {
            let files = collect_json(&inputs)?;
            fs::create_dir_all(&out)?;
            let mut refused = 0;
            for file in &files {
                let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
                let fixture: ResolvedFixture =
                    serde_json::from_str(&text).with_context(|| format!("in {}", file.display()))?;
                // A build with blocks the engine doesn't simulate yet gets no
                // trace; the browser check expects the same refusal there.
                match tickwright_engine::run::run(&fixture, &mc_version) {
                    Ok(trace) => {
                        fs::write(out.join(format!("{}.json", fixture.id)), serde_json::to_string(&trace)? + "\n")?
                    }
                    Err(e) => {
                        eprintln!("skipped {}: {e:#}", fixture.id);
                        refused += 1;
                    }
                }
            }
            println!("ran {} fixture(s) into {}, {refused} refused", files.len() - refused, out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Import { schematic, out, name, ticks } => {
            let bytes = fs::read(&schematic).with_context(|| format!("reading {}", schematic.display()))?;
            let s = Schematic::read(&bytes).with_context(|| format!("in {}", schematic.display()))?;
            let name = name.unwrap_or_else(|| {
                schematic.file_stem().map_or("imported".into(), |n| n.to_string_lossy().into_owned())
            });
            let fixture = s.to_fixture(&name, ticks)?;
            fs::write(&out, serde_json::to_string_pretty(&fixture)? + "\n")?;
            println!(
                "{}: {}x{}x{}, {} blocks, {} container(s), {} comparator output(s) -> {}",
                schematic.display(),
                s.size[0],
                s.size[1],
                s.size[2],
                s.blocks.len(),
                fixture.containers.len(),
                fixture.comparators.len(),
                out.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Export { fixture, out } => {
            let format = Format::from_name(&out.to_string_lossy())?;
            let text = fs::read_to_string(&fixture).with_context(|| format!("reading {}", fixture.display()))?;
            // Either a hand-written fixture, exported unrotated, or a resolved one.
            let resolved = match serde_json::from_str::<ResolvedFixture>(&text) {
                Ok(r) => r,
                Err(_) => FixtureSource::from_json(&text)
                    .with_context(|| format!("in {}", fixture.display()))?
                    .resolve(Transform::IDENTITY)?,
            };
            let s = Schematic::from_fixture(&resolved)?;
            fs::write(&out, s.write(format))?;
            println!("{}: {} blocks -> {}", resolved.id, s.blocks.len(), out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Bench { rings, length, ticks, write } => {
            let fixture = tickwright_engine::bench::redstone_rings(rings, length, ticks);
            if let Some(path) = write {
                fs::write(&path, serde_json::to_string(&fixture)? + "\n")?;
            }
            let start = std::time::Instant::now();
            tickwright_engine::run::run_unrecorded(&fixture, "26.3")?;
            let secs = start.elapsed().as_secs_f64();
            println!(
                "{}: {} blocks, {ticks} ticks in {secs:.3} s, {:.0} ticks/s",
                fixture.name,
                fixture.blocks.len(),
                f64::from(ticks) / secs
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Diff { expected, actual, level } => {
            let load = |p: &Path| -> Result<Trace> {
                Trace::from_json(&fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)
                    .with_context(|| format!("in {}", p.display()))
            };
            let (a, b) = (load(&expected)?, load(&actual)?);
            let level = Level::from(level);
            for (name, t) in [("expected", &a), ("actual", &b)] {
                anyhow::ensure!(t.has_level(level), "the {name} trace does not record the {level:?} level");
            }
            match compare(&a, &b, level) {
                Ok(()) => {
                    println!("match ({level:?} level, {} ticks)", a.ticks);
                    Ok(ExitCode::SUCCESS)
                }
                Err(d) => {
                    print!("{d}");
                    Ok(ExitCode::from(1))
                }
            }
        }
    }
}

fn collect_json(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let mut stack = vec![input.clone()];
            while let Some(dir) = stack.pop() {
                for entry in fs::read_dir(&dir)? {
                    let p = entry?.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|e| e == "json") {
                        out.push(p);
                    }
                }
            }
        } else {
            out.push(input.clone());
        }
    }
    out.sort();
    Ok(out)
}
