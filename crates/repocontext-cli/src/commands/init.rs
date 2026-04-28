//! `repocontext init` — write a default `.repocontext.toml` and (optional)
//! `.gitignore` entry. Idempotent unless `--force`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use tracing::info;

const DEFAULT_TOML: &str = r#"# repocontext configuration
# See README for the full schema.

[output]
temp_path = "context_temp.md"
final_path = "context.md"
profile = "full"
max_tokens = 8000

[include]
paths = ["."]
languages = ["typescript"]

[exclude]
paths = [
    "**/test/**",
    "**/tests/**",
    "**/__tests__/**",
    "**/*.test.ts",
    "**/*.test.tsx",
    "**/*.spec.ts",
    "**/*.spec.tsx",
    "**/*.generated.*",
]

[synthesis]
include_doc_comments = true
include_implementation_for_top_n = 10
deterministic_ordering = true

[enrich]
enabled = false
max_tokens_per_request = 400
temperature = 0.2
seed = 42
timeout_seconds = 120
max_concurrent_requests = 1
chunk_strategy = "by_section"

[enrich.model]
# Default: qwen2.5-coder:7b Q4_K_M (~4.5 GB, downloaded on first --enrich run).
name = "qwen2.5-coder-7b-instruct"
quantization = "q4_k_m"
context_size = 4096
gpu_layers = -1   # -1 = auto (Metal on macOS, CPU otherwise)

[enrich.cache]
# "json" = local file (zero-config, committable for CI).
# "redis" = local Redis (great for shared team caches).
backend = "json"
path = ".repocontext/enrich-cache.json"
url = "redis://localhost:6379"
key_prefix = "repocontext:"
"#;

const GITIGNORE_BLOCK: &str = "\n# repocontext\n.repocontext/\ncontext_temp.md\ncontext.md\n";

pub fn run(repo_root: &Path, config_path: &Path, force: bool, no_gitignore: bool) -> Result<u8> {
    if config_path.exists() && !force {
        bail!(
            "{} already exists. Re-run with --force to overwrite.",
            config_path.display()
        );
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(config_path, DEFAULT_TOML)
        .with_context(|| format!("writing {}", config_path.display()))?;
    info!("wrote {}", config_path.display());

    if !no_gitignore {
        update_gitignore(repo_root)?;
    }

    Ok(0)
}

fn update_gitignore(repo_root: &Path) -> Result<()> {
    let path = repo_root.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.contains("# repocontext") {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.ends_with('\n') && !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str(GITIGNORE_BLOCK);
    std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
    info!("updated {}", path.display());
    Ok(())
}
