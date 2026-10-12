//! `tickwright`: tools for fixtures and traces.
//!
//! - `resolve` turns hand-written fixtures into one explicit fixture per
//!   orientation, which is what the capture harness and the engine load.
//! - `diff` compares two traces and reports the first divergence.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tickwright_trace::{FixtureSource, Level, Trace, compare};

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
