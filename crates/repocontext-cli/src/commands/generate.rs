//! `repocontext generate` — Stage 1 (and Stage 2 once `--enrich`).
//!
//! Backend selection: when the `inference` feature is compiled in, `--enrich`
//! uses [`LlamaCppBackend`] backed by the local GGUF model. Without it, the
//! command falls back to [`MockBackend`] (deterministic placeholders) and
//! warns the user — useful for verifying the pipeline without paying the
//! 5–15 minute llama.cpp C++ compile cost.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use repocontext_core::config::{CacheBackend, Config};
use repocontext_core::metadata;
use repocontext_enrich::model::{resolved_path, ModelDescriptor};
use repocontext_enrich::{
    assemble, enrich, ChunkerConfig, CompletionParams, EnrichCache, EnrichConfig, JsonFileCache,
    LlmBackend, RedisCache,
};
use tracing::{info, warn};

use crate::orchestrator;

#[allow(clippy::too_many_arguments)]
pub fn run(
    repo_root: &Path,
    config_path: &Path,
    enrich_flag: bool,
    output_temp_override: Option<&Path>,
    output_override: Option<&Path>,
    no_cache: bool,
    dry_run_llm: bool,
    model_path_override: Option<&Path>,
) -> Result<u8> {
    let mut cfg = Config::load(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.apply_profile(None)?;

    let stage1 = orchestrator::run_stage1(repo_root, &cfg)?;

    let temp_path = resolve_path(repo_root, &cfg.output.temp_path, output_temp_override);
    write_with_parents(&temp_path, &stage1.text)?;
    info!("wrote {}", temp_path.display());

    if !enrich_flag {
        return Ok(0);
    }

    // ─────────────────────────────────────── Stage 2 ───────────────────────────────────────

    let cache: Box<dyn EnrichCache> = match cfg.enrich.cache.backend {
        CacheBackend::Json => {
            let cache_path = resolve_path(repo_root, &cfg.enrich.cache.path, None);
            let json_cache = JsonFileCache::load(cache_path.clone())
                .with_context(|| format!("loading cache from {}", cache_path.display()))?;
            info!("Cache: JSON at {}", cache_path.display());
            Box::new(json_cache)
        }
        CacheBackend::Redis => {
            info!(
                "Cache: Redis at {} (prefix `{}`)",
                cfg.enrich.cache.url, cfg.enrich.cache.key_prefix
            );
            let redis_cache = RedisCache::new(
                cfg.enrich.cache.url.clone(),
                cfg.enrich.cache.key_prefix.clone(),
            )
            .with_context(|| format!("opening Redis cache at {}", cfg.enrich.cache.url))?;
            Box::new(redis_cache)
        }
    };

    // Tag the model id with the backend kind so Mock outputs don't collide
    // with real-LLM outputs in the cache. Without this, a `--features inference`
    // run would serve Mock entries from a previous default-features run, and
    // vice-versa.
    let backend_tag = if cfg!(feature = "inference") && !dry_run_llm {
        ""
    } else {
        "-mock"
    };
    let model_id = format!(
        "{}-{}{}",
        cfg.enrich.model.name, cfg.enrich.model.quantization, backend_tag
    );
    let completion_params = CompletionParams {
        temperature: cfg.enrich.temperature,
        seed: cfg.enrich.seed,
        max_tokens: cfg.enrich.max_tokens_per_request,
        n_ctx: cfg.enrich.model.context_size,
    };
    let enrich_cfg = EnrichConfig {
        model_id,
        chunker_config: ChunkerConfig::from_params(&completion_params),
        completion_params,
        dry_run: dry_run_llm,
        no_cache,
    };

    info!(
        "Stage 2: chunk budget = {} chars (n_ctx={}, prompt overhead 600t, response={}t)",
        enrich_cfg.chunker_config.chunk_budget_chars,
        enrich_cfg.completion_params.n_ctx,
        enrich_cfg.completion_params.max_tokens
    );

    // Resolve the model path (consulted only when `inference` feature is on).
    let model_path = resolve_model_path(repo_root, &cfg, model_path_override)?;

    let mut backend = build_backend(&model_path, dry_run_llm)?;

    let result = enrich(&stage1.text, &enrich_cfg, cache.as_ref(), backend.as_mut())?;

    if dry_run_llm {
        info!(
            "--dry-run-llm: {} chunks logged to stdout, nothing written.",
            result.chunks.len()
        );
        return Ok(0);
    }

    let final_path = resolve_path(repo_root, &cfg.output.final_path, output_override);
    let project_metadata = metadata::collect_metadata(repo_root);
    let context_md = assemble(&result, project_metadata.name.as_deref());
    write_with_parents(&final_path, &context_md)?;
    info!(
        "wrote {} ({} chunks: {} cache, {} LLM, {} fallback)",
        final_path.display(),
        result.chunks.len(),
        result.cache_hits,
        result.cache_misses - result.failures,
        result.failures
    );

    if result.failures > 0 {
        warn!(
            "{}/{} chunks fell back to placeholder content (LLM errors). \
             context.md is structurally complete but missing narrative for those sections.",
            result.failures,
            result.chunks.len()
        );
        return Ok(1);
    }

    Ok(0)
}

fn resolve_path(repo_root: &Path, configured: &Path, override_path: Option<&Path>) -> PathBuf {
    let chosen = override_path.unwrap_or(configured);
    if chosen.is_absolute() {
        chosen.to_path_buf()
    } else {
        repo_root.join(chosen)
    }
}

fn write_with_parents(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn resolve_model_path(
    repo_root: &Path,
    cfg: &Config,
    override_path: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(p) = override_path {
        let path = if p.is_absolute() {
            p.to_path_buf()
        } else {
            repo_root.join(p)
        };
        return Ok(path);
    }
    if let Some(p) = &cfg.enrich.model.path_override {
        return Ok(p.clone());
    }
    let descriptor = ModelDescriptor::from_config(&cfg.enrich.model);
    let cache_override = cfg.enrich.model.cache_dir.as_deref();
    resolved_path(&descriptor, cache_override)
}

#[cfg(feature = "inference")]
fn build_backend(model_path: &Path, dry_run_llm: bool) -> Result<Box<dyn LlmBackend>> {
    use repocontext_enrich::inference::LlamaCppBackend;

    if dry_run_llm {
        // No backend call will happen anyway; skip the model-load cost.
        info!("--dry-run-llm: skipping model load");
        return Ok(Box::new(repocontext_enrich::MockBackend));
    }
    if !model_path.exists() {
        anyhow::bail!(
            "Model not found at {}. Run `repocontext model pull` to download it (~4.5 GB), \
             or pass --model-path to point at an existing GGUF.",
            model_path.display()
        );
    }
    info!("Loading GGUF model from {}", model_path.display());
    let backend = LlamaCppBackend::load(model_path)?;
    Ok(Box::new(backend))
}

#[cfg(not(feature = "inference"))]
fn build_backend(model_path: &Path, _dry_run_llm: bool) -> Result<Box<dyn LlmBackend>> {
    let _ = model_path;
    warn!(
        "--enrich is using MockBackend (deterministic placeholders). \
         For real LLM output, rebuild with `cargo install --path crates/repocontext-cli --features inference` \
         (or --features inference-metal on Apple Silicon)."
    );
    Ok(Box::new(repocontext_enrich::MockBackend))
}
