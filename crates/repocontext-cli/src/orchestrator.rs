//! Glue between the CLI and the core/extractor crates.
//!
//! Walks the repo, parses each TypeScript file via `repocontext-lang-ts`,
//! scores symbols, and synthesizes `context_temp.md` content. The CLI command
//! handlers ([`crate::commands`]) call this and decide what to do with the
//! produced string (write to disk, compare, etc.).

use std::path::Path;

use anyhow::{Context, Result};
use repocontext_core::config::Config;
use repocontext_core::salience::score_all;
use repocontext_core::symbol::{ExtractedSymbols, IndexedFile};
use repocontext_core::synth::{synthesize_stage1, SynthesisInput};
use repocontext_core::walker::{walk, WalkOptions};
use repocontext_core::{metadata, ScoredSymbol};
use tracing::{debug, warn};

/// Run Stage 1 end-to-end and return `(stage1_text, indexed_files, scored_symbols)`.
/// Symbol extraction errors per-file are logged and skipped — they never fail the run.
pub fn run_stage1(repo_root: &Path, cfg: &Config) -> Result<Stage1Output> {
    let walk_opts = WalkOptions::from_config(cfg);
    let discovered =
        walk(repo_root, &walk_opts).with_context(|| format!("walking {}", repo_root.display()))?;
    debug!("walker discovered {} files", discovered.len());

    let mut indexed = Vec::new();
    for f in &discovered {
        if !is_typescript_file(&f.relative_path) {
            continue;
        }
        let source = match std::fs::read_to_string(&f.absolute_path) {
            Ok(s) => s,
            Err(e) => {
                warn!("skip {}: {}", f.relative_path.display(), e);
                continue;
            }
        };
        let extracted = match repocontext_lang_ts::extract_file(&source, &f.relative_path) {
            Ok(e) => e,
            Err(e) => {
                warn!("extract failed for {}: {:#}", f.relative_path.display(), e);
                ExtractedSymbols::default()
            }
        };
        if extracted.had_parse_errors {
            warn!(
                "{} has tree-sitter parse errors; extracted what we could",
                f.relative_path.display()
            );
        }
        indexed.push(IndexedFile {
            relative_path: f.relative_path.clone(),
            source,
            extracted,
        });
    }
    debug!("indexed {} TypeScript files", indexed.len());

    let scored = score_all(&indexed);
    debug!("scored {} symbols", scored.len());

    let project_metadata = metadata::collect_metadata(repo_root);
    let readme = metadata::read_readme_excerpt(repo_root);

    let input = SynthesisInput {
        config: cfg,
        files: &indexed,
        scored: &scored,
        metadata: &project_metadata,
        readme_excerpt: readme.as_deref(),
        repocontext_version: env!("CARGO_PKG_VERSION"),
    };
    let stage1 = synthesize_stage1(&input).context("synthesizing context_temp.md")?;

    Ok(Stage1Output {
        text: stage1,
        indexed,
        scored,
    })
}

pub struct Stage1Output {
    pub text: String,
    pub indexed: Vec<IndexedFile>,
    pub scored: Vec<ScoredSymbol>,
}

fn is_typescript_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("tsx") | Some("mts") | Some("cts")
    )
}
