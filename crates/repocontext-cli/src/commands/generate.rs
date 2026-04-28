//! `repocontext generate` — Stage 1 (and Stage 2 once enrichment lands).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use repocontext_core::config::Config;
use tracing::info;

use crate::orchestrator;

pub fn run(
    repo_root: &Path,
    config_path: &Path,
    enrich: bool,
    output_temp_override: Option<&Path>,
) -> Result<u8> {
    let mut cfg = Config::load(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.apply_profile(None)?;

    let stage1 = orchestrator::run_stage1(repo_root, &cfg)?;

    let temp_path = resolve_temp_path(repo_root, &cfg, output_temp_override);
    if let Some(parent) = temp_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&temp_path, &stage1.text)
        .with_context(|| format!("writing {}", temp_path.display()))?;
    info!("wrote {}", temp_path.display());

    if enrich {
        bail!(
            "Stage 2 (--enrich) is not yet implemented in this build. \
             Stage 1 succeeded — `context_temp.md` is at {}.",
            temp_path.display()
        );
    }

    Ok(0)
}

fn resolve_temp_path(repo_root: &Path, cfg: &Config, override_path: Option<&Path>) -> PathBuf {
    if let Some(p) = override_path {
        if p.is_absolute() {
            return p.to_path_buf();
        }
        return repo_root.join(p);
    }
    if cfg.output.temp_path.is_absolute() {
        cfg.output.temp_path.clone()
    } else {
        repo_root.join(&cfg.output.temp_path)
    }
}
