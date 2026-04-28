//! `repocontext check` — synthesize Stage 1 in memory and compare to the file on disk.
//!
//! Returns exit code 0 if the on-disk file matches, 1 if stale or missing.

use std::path::Path;

use anyhow::{Context, Result};
use repocontext_core::config::Config;

use crate::orchestrator;

pub fn run(repo_root: &Path, config_path: &Path) -> Result<u8> {
    let mut cfg = Config::load(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.apply_profile(None)?;

    let stage1 = orchestrator::run_stage1(repo_root, &cfg)?;

    let temp_path = if cfg.output.temp_path.is_absolute() {
        cfg.output.temp_path.clone()
    } else {
        repo_root.join(&cfg.output.temp_path)
    };
    let on_disk = std::fs::read_to_string(&temp_path).ok();

    match on_disk {
        Some(actual) if actual == stage1.text => Ok(0),
        Some(_) => {
            eprintln!(
                "{} is stale. Run `repocontext generate` to update.",
                temp_path.display()
            );
            Ok(1)
        }
        None => {
            eprintln!(
                "{} does not exist. Run `repocontext generate` to create it.",
                temp_path.display()
            );
            Ok(1)
        }
    }
}
