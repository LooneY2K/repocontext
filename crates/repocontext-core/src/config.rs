//! Configuration types and TOML loading for repocontext.
//!
//! The config is read from `.repocontext.toml` (or whatever path is passed to
//! [`Config::load`]). Every field has a sensible default — a totally absent
//! config file is fine. Zero-config use returns [`Config::default`].
//!
//! Schema overview (each section maps to a struct in this module):
//!
//! ```toml
//! [output]              # OutputConfig: paths + token budget
//! [include]             # IncludeConfig: scan roots + languages
//! [exclude]             # ExcludeConfig: glob patterns to skip
//! [synthesis]           # SynthesisConfig: Stage 1 knobs
//! [enrich]              # EnrichConfig: Stage 2 toggles + sampling
//! [enrich.model]        # ModelConfig: which GGUF + how to load it
//! [enrich.cache]        # CacheConfig: JSON path or Redis URL + key prefix
//! [profiles.<name>]     # ProfileConfig: per-profile overrides of output knobs
//! ```

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level repocontext configuration. All fields default if absent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub output: OutputConfig,
    pub include: IncludeConfig,
    pub exclude: ExcludeConfig,
    pub synthesis: SynthesisConfig,
    pub enrich: EnrichConfig,
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub temp_path: PathBuf,
    pub final_path: PathBuf,
    pub profile: String,
    pub max_tokens: usize,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            temp_path: PathBuf::from("context_temp.md"),
            final_path: PathBuf::from("context.md"),
            profile: "full".to_string(),
            max_tokens: 8000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IncludeConfig {
    pub paths: Vec<PathBuf>,
    pub languages: Vec<String>,
}

impl Default for IncludeConfig {
    fn default() -> Self {
        Self {
            paths: vec![PathBuf::from(".")],
            languages: vec!["typescript".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludeConfig {
    pub paths: Vec<String>,
}

impl Default for ExcludeConfig {
    fn default() -> Self {
        // Globs in addition to the always-on hardcoded excludes
        // (`node_modules`, `dist`, `build`, `target`, `.git`, `vendor`)
        // applied by the walker in phase 3.
        Self {
            paths: vec![
                "**/test/**".to_string(),
                "**/tests/**".to_string(),
                "**/*.generated.*".to_string(),
                "**/*.test.ts".to_string(),
                "**/*.test.tsx".to_string(),
                "**/*.spec.ts".to_string(),
                "**/*.spec.tsx".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisConfig {
    pub include_doc_comments: bool,
    pub include_implementation_for_top_n: usize,
    pub deterministic_ordering: bool,
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            include_doc_comments: true,
            include_implementation_for_top_n: 10,
            deterministic_ordering: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnrichConfig {
    pub enabled: bool,
    pub max_tokens_per_request: u32,
    pub temperature: f32,
    pub seed: u64,
    pub timeout_seconds: u64,
    pub max_concurrent_requests: u32,
    pub chunk_strategy: ChunkStrategy,
    pub model: ModelConfig,
    pub cache: CacheConfig,
}

impl Default for EnrichConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_tokens_per_request: 400,
            temperature: 0.2,
            seed: 42,
            timeout_seconds: 120,
            max_concurrent_requests: 1,
            chunk_strategy: ChunkStrategy::BySection,
            model: ModelConfig::default(),
            cache: CacheConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStrategy {
    #[default]
    BySection,
    ByModule,
    FixedSize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// Logical model identifier, e.g. `qwen2.5-coder-7b-instruct`. The
    /// quantization suffix is stored separately so we can construct download
    /// URLs and cache filenames consistently.
    pub name: String,
    /// e.g. `q4_k_m`, `q5_k_m`, `q8_0`. Combined with `name` to form the GGUF
    /// filename: `{name}-{quantization}.gguf`.
    pub quantization: String,
    /// llama.cpp `n_ctx`. Must be ≤ the model's training context window.
    pub context_size: u32,
    /// llama.cpp `n_gpu_layers`. -1 means "all layers on GPU if available".
    pub gpu_layers: i32,
    /// Override the resolved model file path entirely. Used for tests and for
    /// users running custom GGUFs.
    pub path_override: Option<PathBuf>,
    /// Override the cache dir where models are downloaded. Defaults to
    /// `dirs::cache_dir()/repocontext/models/`.
    pub cache_dir: Option<PathBuf>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            name: "qwen2.5-coder-7b-instruct".to_string(),
            quantization: "q4_k_m".to_string(),
            context_size: 4096,
            gpu_layers: -1,
            path_override: None,
            cache_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub backend: CacheBackend,
    /// Used when `backend = "json"`.
    pub path: PathBuf,
    /// Used when `backend = "redis"`.
    pub url: String,
    /// Used when `backend = "redis"`.
    pub key_prefix: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            backend: CacheBackend::Json,
            path: PathBuf::from(".repocontext/enrich-cache.json"),
            url: "redis://localhost:6379".to_string(),
            key_prefix: "repocontext:".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CacheBackend {
    #[default]
    Json,
    Redis,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileConfig {
    pub max_tokens: Option<usize>,
    pub sections: Option<Vec<String>>,
}

impl Config {
    /// Load a config from a `.repocontext.toml` file.
    ///
    /// If the file doesn't exist, returns [`Config::default`] — this is the
    /// zero-config path. Other I/O or parse errors propagate. The returned
    /// config has been [validated](Self::validate); if it contains forbidden
    /// path patterns (e.g. `..` parent-directory traversal), this errors.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("reading config from {}", path.display()))?;
        let cfg: Self = s.parse()?;
        cfg.validate()
            .with_context(|| format!("validating config at {}", path.display()))?;
        Ok(cfg)
    }

    /// Verify path-typed config fields don't contain `..` parent-directory
    /// components. A hostile `.repocontext.toml` (e.g. one shipped with a
    /// repository you just cloned) could otherwise point `[output] temp_path`
    /// at `../../etc/foo` and cause repocontext to write outside the project
    /// when invoked. Absolute paths are allowed — they're visible in the toml
    /// and easy for a reviewer to spot.
    pub fn validate(&self) -> Result<()> {
        check_no_parent_traversal(&self.output.temp_path, "[output] temp_path")?;
        check_no_parent_traversal(&self.output.final_path, "[output] final_path")?;
        check_no_parent_traversal(&self.enrich.cache.path, "[enrich.cache] path")?;
        if let Some(p) = &self.enrich.model.cache_dir {
            check_no_parent_traversal(p, "[enrich.model] cache_dir")?;
        }
        if let Some(p) = &self.enrich.model.path_override {
            check_no_parent_traversal(p, "[enrich.model] path_override")?;
        }
        for include in &self.include.paths {
            check_no_parent_traversal(include, "[include] paths entry")?;
        }
        Ok(())
    }

    /// Resolve the profile named in `output.profile` (or `name` if provided),
    /// applying its overrides to this config in-place.
    ///
    /// `"full"` is the implicit default and a no-op. Unknown profiles return
    /// an error listing the known profile names.
    pub fn apply_profile(&mut self, name: Option<&str>) -> Result<()> {
        let chosen = name.unwrap_or(&self.output.profile).to_string();
        if chosen == "full" {
            return Ok(());
        }
        let profile = self.profiles.get(&chosen).cloned().with_context(|| {
            let known = self.profiles.keys().cloned().collect::<Vec<_>>().join(", ");
            let known = if known.is_empty() {
                "(none)".to_string()
            } else {
                known
            };
            format!(
                "profile `{}` not found in [profiles]; known profiles: {}",
                chosen, known
            )
        })?;
        if let Some(mt) = profile.max_tokens {
            self.output.max_tokens = mt;
        }
        // `sections` is consumed by phase 6 synthesis; it stays accessible
        // via `self.profiles[&chosen].sections`.
        Ok(())
    }
}

impl FromStr for Config {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).context("parsing repocontext config TOML")
    }
}

fn check_no_parent_traversal(path: &Path, source: &str) -> Result<()> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!(
            "{source} contains `..` parent-directory components ({}). \
             Refusing to escape the project root.",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_parent_traversal_in_temp_path() {
        let toml = r#"
[output]
temp_path = "../../etc/passwd"
"#;
        let cfg: Config = toml.parse().expect("parse");
        let err = cfg.validate().expect_err("must reject ..");
        let msg = format!("{err:#}");
        assert!(msg.contains("[output] temp_path"), "got: {msg}");
        assert!(msg.contains(".."), "got: {msg}");
    }

    #[test]
    fn validate_rejects_parent_traversal_in_cache_path() {
        let toml = r#"
[enrich.cache]
path = "../../tmp/leak.json"
"#;
        let cfg: Config = toml.parse().expect("parse");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_allows_absolute_paths() {
        let toml = r#"
[output]
temp_path = "/tmp/repocontext_temp.md"
final_path = "/tmp/repocontext.md"
"#;
        let cfg: Config = toml.parse().expect("parse");
        cfg.validate().expect("absolute paths allowed");
    }

    #[test]
    fn validate_allows_normal_relative_paths() {
        let cfg = Config::default();
        cfg.validate().expect("defaults must validate");
    }

    #[test]
    fn empty_toml_uses_defaults() {
        let cfg: Config = "".parse().expect("parse empty");
        assert_eq!(cfg.output.temp_path, PathBuf::from("context_temp.md"));
        assert_eq!(cfg.output.final_path, PathBuf::from("context.md"));
        assert_eq!(cfg.output.max_tokens, 8000);
        assert_eq!(cfg.output.profile, "full");
        assert_eq!(cfg.synthesis.include_implementation_for_top_n, 10);
        assert!(cfg.synthesis.include_doc_comments);
        assert!(!cfg.enrich.enabled);
        assert_eq!(cfg.enrich.model.name, "qwen2.5-coder-7b-instruct");
        assert_eq!(cfg.enrich.model.quantization, "q4_k_m");
        assert_eq!(cfg.enrich.model.gpu_layers, -1);
        assert_eq!(cfg.enrich.cache.backend, CacheBackend::Json);
        assert_eq!(
            cfg.enrich.cache.path,
            PathBuf::from(".repocontext/enrich-cache.json")
        );
    }

    #[test]
    fn full_toml_parses() {
        let toml_str = r#"
            [output]
            temp_path = "out.md"
            final_path = "ctx.md"
            profile = "minimal"
            max_tokens = 4000

            [include]
            paths = ["src/"]
            languages = ["typescript"]

            [exclude]
            paths = ["**/*.test.ts"]

            [synthesis]
            include_doc_comments = false
            include_implementation_for_top_n = 5
            deterministic_ordering = true

            [enrich]
            enabled = true
            temperature = 0.4
            seed = 7
            chunk_strategy = "by_module"

            [enrich.model]
            name = "custom-model"
            quantization = "q5_k_m"
            context_size = 8192
            gpu_layers = 32

            [enrich.cache]
            backend = "redis"
            url = "redis://example:6379"
            key_prefix = "myproj:"

            [profiles.minimal]
            max_tokens = 1500
            sections = ["overview", "architecture"]
        "#;
        let cfg: Config = toml_str.parse().expect("parse full");
        assert_eq!(cfg.output.temp_path, PathBuf::from("out.md"));
        assert_eq!(cfg.output.profile, "minimal");
        assert_eq!(cfg.output.max_tokens, 4000);
        assert_eq!(cfg.exclude.paths, vec!["**/*.test.ts".to_string()]);
        assert!(!cfg.synthesis.include_doc_comments);
        assert_eq!(cfg.synthesis.include_implementation_for_top_n, 5);
        assert!(cfg.enrich.enabled);
        assert!((cfg.enrich.temperature - 0.4).abs() < 1e-6);
        assert_eq!(cfg.enrich.seed, 7);
        assert_eq!(cfg.enrich.chunk_strategy, ChunkStrategy::ByModule);
        assert_eq!(cfg.enrich.model.name, "custom-model");
        assert_eq!(cfg.enrich.model.gpu_layers, 32);
        assert_eq!(cfg.enrich.cache.backend, CacheBackend::Redis);
        assert_eq!(cfg.enrich.cache.url, "redis://example:6379");
        assert_eq!(cfg.enrich.cache.key_prefix, "myproj:");
        let profile = cfg.profiles.get("minimal").expect("profile present");
        assert_eq!(profile.max_tokens, Some(1500));
        assert_eq!(
            profile.sections.as_ref().unwrap(),
            &vec!["overview".to_string(), "architecture".to_string()]
        );
    }

    #[test]
    fn partial_section_uses_field_defaults() {
        let toml_str = r#"
            [output]
            max_tokens = 1234
        "#;
        let cfg: Config = toml_str.parse().expect("parse partial");
        // max_tokens overridden, other fields use defaults
        assert_eq!(cfg.output.max_tokens, 1234);
        assert_eq!(cfg.output.profile, "full");
        assert_eq!(cfg.output.temp_path, PathBuf::from("context_temp.md"));
    }

    #[test]
    fn profile_override_applies() {
        let toml_str = r#"
            [output]
            profile = "minimal"
            max_tokens = 8000

            [profiles.minimal]
            max_tokens = 1500
        "#;
        let mut cfg: Config = toml_str.parse().expect("parse");
        cfg.apply_profile(None).expect("apply");
        assert_eq!(cfg.output.max_tokens, 1500);
    }

    #[test]
    fn explicit_profile_arg_overrides_output_profile_field() {
        let toml_str = r#"
            [output]
            profile = "minimal"
            max_tokens = 8000

            [profiles.minimal]
            max_tokens = 1500

            [profiles.tiny]
            max_tokens = 500
        "#;
        let mut cfg: Config = toml_str.parse().expect("parse");
        cfg.apply_profile(Some("tiny")).expect("apply tiny");
        assert_eq!(cfg.output.max_tokens, 500);
    }

    #[test]
    fn full_profile_is_no_op() {
        let mut cfg = Config::default();
        cfg.output.max_tokens = 9999;
        cfg.apply_profile(Some("full")).expect("apply full");
        assert_eq!(cfg.output.max_tokens, 9999);
    }

    #[test]
    fn unknown_profile_errors() {
        let mut cfg = Config::default();
        let err = cfg.apply_profile(Some("bogus")).expect_err("bogus profile");
        let msg = err.to_string();
        assert!(msg.contains("profile `bogus` not found"), "got: {}", msg);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let p = std::path::PathBuf::from("/this/does/not/exist/.repocontext.toml");
        let cfg = Config::load(&p).expect("load missing returns default");
        assert_eq!(cfg.output.temp_path, PathBuf::from("context_temp.md"));
    }

    #[test]
    fn load_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".repocontext.toml");
        std::fs::write(&p, "[output]\nmax_tokens = 1234\n").unwrap();
        let cfg = Config::load(&p).expect("load");
        assert_eq!(cfg.output.max_tokens, 1234);
    }

    #[test]
    fn parse_error_is_actionable() {
        let toml_str = "[output\nbroken";
        let err = toml_str
            .parse::<Config>()
            .expect_err("malformed toml should error");
        let msg = err.to_string();
        assert!(
            msg.contains("parsing repocontext config TOML"),
            "got: {}",
            msg
        );
    }
}
