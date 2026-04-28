//! `repocontext extract` — debug command. Walks the repo, parses, and dumps
//! the indexed file set + scored symbols as JSON to stdout. Hidden from `--help`.

use std::path::Path;

use anyhow::{Context, Result};
use repocontext_core::config::Config;
use serde::Serialize;

use crate::orchestrator;

#[derive(Serialize)]
struct ExtractOutput<'a> {
    indexed: &'a [repocontext_core::symbol::IndexedFile],
    scored: &'a [repocontext_core::salience::ScoredSymbol],
}

pub fn run(repo_root: &Path, config_path: &Path) -> Result<u8> {
    let mut cfg = Config::load(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.apply_profile(None)?;

    let stage1 = orchestrator::run_stage1(repo_root, &cfg)?;
    let payload = ExtractOutput {
        indexed: &stage1.indexed,
        scored: &stage1.scored,
    };
    let json =
        serde_json::to_string_pretty(&payload).context("serializing extract payload to JSON")?;
    println!("{json}");
    Ok(0)
}
