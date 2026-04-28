//! `repocontext model {pull,list,remove}` — manage cached GGUF models.

use std::path::Path;

use anyhow::{Context, Result};
use repocontext_core::config::Config;
use repocontext_enrich::model::{cache_dir, download, is_present, resolved_path, ModelDescriptor};
use tracing::info;

pub fn pull(_repo_root: &Path, config_path: &Path) -> Result<u8> {
    let cfg = load_config(config_path)?;
    let descriptor = ModelDescriptor::from_config(&cfg.enrich.model);
    let cache_override = cfg.enrich.model.cache_dir.as_deref();

    if is_present(&descriptor, cache_override)? {
        let path = resolved_path(&descriptor, cache_override)?;
        info!("Model already cached at {}", path.display());
        return Ok(0);
    }

    info!(
        "Downloading {} (~{:.1} GB). This is a one-time fetch.",
        descriptor.filename(),
        descriptor.approx_size_bytes as f64 / 1_000_000_000.0
    );
    let path = download(&descriptor, cache_override, true)?;
    info!("Model ready at {}", path.display());
    Ok(0)
}

pub fn list(_repo_root: &Path, config_path: &Path) -> Result<u8> {
    let cfg = load_config(config_path)?;
    let cache_override = cfg.enrich.model.cache_dir.as_deref();
    let dir = cache_dir(cache_override)?;

    if !dir.exists() {
        println!("No models cached.");
        println!("(Cache dir would be at {})", dir.display());
        return Ok(0);
    }

    println!("Cache: {}", dir.display());
    println!();
    let mut entries: Vec<(String, u64)> = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push((name, meta.len()));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    if entries.is_empty() {
        println!("(empty)");
    } else {
        let mut total: u64 = 0;
        for (name, size) in &entries {
            let size_mb = *size as f64 / (1024.0 * 1024.0);
            println!("  {name}  ({size_mb:.1} MB)");
            total += size;
        }
        println!();
        let total_gb = total as f64 / 1_000_000_000.0;
        println!("Total: {total_gb:.2} GB across {} files", entries.len());
    }
    Ok(0)
}

pub fn remove(_repo_root: &Path, config_path: &Path, name: Option<&str>) -> Result<u8> {
    let cfg = load_config(config_path)?;
    let cache_override = cfg.enrich.model.cache_dir.as_deref();
    let path = match name {
        Some(n) => cache_dir(cache_override)?.join(n),
        None => {
            let descriptor = ModelDescriptor::from_config(&cfg.enrich.model);
            resolved_path(&descriptor, cache_override)?
        }
    };

    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        return Ok(1);
    }
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    info!("Removed {}", path.display());
    Ok(0)
}

fn load_config(config_path: &Path) -> Result<Config> {
    let mut cfg = Config::load(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.apply_profile(None)?;
    Ok(cfg)
}
