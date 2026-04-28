//! `repocontext` CLI entry point.
//!
//! Subcommands:
//! - `init`     — write a default `.repocontext.toml` and (optional) gitignore entry
//! - `generate` — Stage 1 (and `--enrich` later): write `context_temp.md`
//! - `check`    — exit non-zero if `context_temp.md` is stale
//! - `extract`  — debug helper: dump the parsed index as JSON to stdout
//!
//! Exit codes follow the spec:
//! - 0 = success
//! - 1 = user-correctable failure (stale, missing files, etc.)
//! - 2 = tool error (config parse, IO failure, panic)

mod commands;
mod orchestrator;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use is_terminal::IsTerminal;

#[derive(Parser, Debug)]
#[command(
    name = "repocontext",
    version,
    about = "Two-stage codebase context generator",
    long_about = "Stage 1 produces a deterministic structural index of your codebase \
                  (context_temp.md). Stage 2 (--enrich) optionally runs a local GGUF \
                  model to produce a business-logic narrative (context.md)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Suppress non-error output.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Verbose logging (set RUST_LOG for finer control).
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Path to `.repocontext.toml`. Defaults to `<repo>/.repocontext.toml`.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Path to the repo root. Defaults to the current directory.
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write a default `.repocontext.toml` (and optionally a `.gitignore` entry).
    Init {
        /// Overwrite an existing `.repocontext.toml`.
        #[arg(long)]
        force: bool,
        /// Skip touching `.gitignore`.
        #[arg(long)]
        no_gitignore: bool,
    },

    /// Run Stage 1 (and Stage 2 if `--enrich`).
    Generate {
        /// Run Stage 2 enrichment after Stage 1 (not yet implemented in this build).
        #[arg(long)]
        enrich: bool,

        /// Override `[output].temp_path`.
        #[arg(long)]
        output_temp: Option<PathBuf>,
    },

    /// Re-synthesize Stage 1 in memory and compare against the file on disk.
    /// Exits 0 if matching, 1 if stale.
    Check,

    /// Dump the indexed file set as JSON to stdout. Hidden from default help.
    #[command(hide = true)]
    Extract,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.quiet, cli.verbose);

    let result = run(&cli);

    match result {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            // Anyhow's display chains causes via `{:#}`.
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli) -> Result<u8> {
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| cli.repo.join(".repocontext.toml"));

    match &cli.command {
        Command::Init {
            force,
            no_gitignore,
        } => commands::init::run(&cli.repo, &config_path, *force, *no_gitignore),
        Command::Generate {
            enrich,
            output_temp,
        } => commands::generate::run(&cli.repo, &config_path, *enrich, output_temp.as_deref()),
        Command::Check => commands::check::run(&cli.repo, &config_path),
        Command::Extract => commands::extract::run(&cli.repo, &config_path),
    }
}

fn init_logging(quiet: bool, verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    let default_filter = if quiet {
        "error"
    } else if verbose {
        "debug"
    } else {
        "info"
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let with_color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();

    let _ = fmt()
        .with_env_filter(filter)
        .with_ansi(with_color)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
