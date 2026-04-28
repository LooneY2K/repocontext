//! Core types for Stage 2 enrichment.
//!
//! These flow from chunker → cache → backend → assembler. Kept deliberately
//! flat (no enums-of-structs) so they're easy to serialise into the cache
//! and easy to inspect with `repocontext extract` / debugging.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What kind of LLM task this chunk represents. Each chunk type has its own
/// prompt template and produces a specific section in `context.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkType {
    /// Project-level overview produced from the metadata + readme sections.
    Overview,
    /// Architecture narrative produced from the directory tree.
    Architecture,
    /// Per-module business-purpose paragraph.
    Module,
    /// Domain-model narrative produced from the data_models section.
    DataModels,
    /// "What this implementation does and why" prose for one entry from
    /// `key_implementations`. Source body is included in the prompt input.
    KeyImplementation,
    /// Same as `KeyImplementation` but the source body is too large to fit
    /// the per-chunk budget — the prompt is given the signature only and the
    /// rendered output notes that the body was elided.
    KeyImplementationElided,
}

impl ChunkType {
    /// Stable string id used in cache keys + diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Architecture => "architecture",
            Self::Module => "module",
            Self::DataModels => "data_models",
            Self::KeyImplementation => "key_implementation",
            Self::KeyImplementationElided => "key_implementation_elided",
        }
    }
}

/// A single unit of work for the LLM. Produced by the chunker, consumed by
/// the orchestrator, cached by content hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique within a single Stage 2 run. For sub-chunked sections this
    /// includes a `:part-N` suffix (e.g. `module:src/services:part-1`).
    pub chunk_id: String,
    /// What kind of task this is.
    pub chunk_type: ChunkType,
    /// Logical section name — e.g. `module:src/services`, `key_impl:src/auth.ts:validateSession`.
    /// Multiple sub-chunks can share the same `section_name`; the assembler
    /// groups by it and concatenates outputs in `part_index` order.
    pub section_name: String,
    /// `Some(N)` for the Nth sub-chunk (0-indexed), `None` for non-split sections.
    pub part_index: Option<usize>,
    /// Total parts for this `section_name`, when sub-chunked. `None` otherwise.
    pub total_parts: Option<usize>,
    /// The raw content from `context_temp.md` that this chunk covers.
    /// Becomes the bulk of the prompt input.
    pub content: String,
    /// Sibling section names in the same repo — fed to the prompt as context
    /// so the LLM can reference adjacent modules. Always empty for non-module
    /// chunk types.
    pub cross_references: Vec<String>,
}

/// One persisted output for a chunk, keyed by the SHA256 of (prompt version,
/// model id, serialized chunk input). Stored in the cache backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedEntry {
    /// Echoed for diagnostics — never used for invalidation.
    pub chunk_type: ChunkType,
    /// Echoed for diagnostics.
    pub section_name: String,
    /// First ~120 chars of the chunk content for human inspection of the cache file.
    pub input_preview: String,
    /// The LLM's response, trimmed of leading/trailing whitespace.
    pub output: String,
    /// Cache key composition — the `# version:` from the prompt template.
    pub prompt_version: u32,
    /// Cache key composition — the model identifier used to produce this output.
    pub model_id: String,
}

/// Sampling parameters threaded through to the inference backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionParams {
    pub temperature: f32,
    pub seed: u64,
    pub max_tokens: u32,
    /// Model context window. Used by the chunker to compute per-chunk budgets;
    /// `LlamaCppBackend` also passes this to llama.cpp directly.
    pub n_ctx: u32,
}

impl Default for CompletionParams {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            seed: 42,
            max_tokens: 400,
            n_ctx: 4096,
        }
    }
}

/// Compose a SHA-256 cache key from `(prompt_version, model_id, chunk_input)`.
/// The cache key MUST include the prompt version and model id so a prompt or
/// model bump invalidates all prior outputs automatically.
pub fn cache_key_hash(prompt_version: u32, model_id: &str, chunk_input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prompt_version.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(model_id.as_bytes());
    hasher.update(b":");
    hasher.update(chunk_input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Estimate token count from byte length using `chars / 4`. Matches the
/// estimator used by the Stage 1 token-budget enforcement so chunker and
/// synthesizer agree on what "fits".
pub fn estimated_tokens(s: &str) -> usize {
    s.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_when_inputs_change() {
        let a = cache_key_hash(1, "qwen-7b", "hello");
        let b = cache_key_hash(2, "qwen-7b", "hello");
        let c = cache_key_hash(1, "qwen-13b", "hello");
        let d = cache_key_hash(1, "qwen-7b", "world");
        assert_ne!(a, b, "version change should bust cache");
        assert_ne!(a, c, "model change should bust cache");
        assert_ne!(a, d, "input change should bust cache");
    }

    #[test]
    fn cache_key_is_deterministic() {
        let a = cache_key_hash(1, "qwen-7b", "hello");
        let b = cache_key_hash(1, "qwen-7b", "hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // sha256 hex length
    }

    #[test]
    fn chunk_type_str_is_stable() {
        assert_eq!(ChunkType::Overview.as_str(), "overview");
        assert_eq!(ChunkType::Module.as_str(), "module");
        assert_eq!(
            ChunkType::KeyImplementationElided.as_str(),
            "key_implementation_elided"
        );
    }

    #[test]
    fn estimator_matches_synth() {
        assert_eq!(estimated_tokens(""), 0);
        assert_eq!(estimated_tokens("abcd"), 1);
        assert_eq!(estimated_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn completion_params_defaults_match_spec() {
        let p = CompletionParams::default();
        assert!((p.temperature - 0.2).abs() < 1e-6);
        assert_eq!(p.seed, 42);
        assert_eq!(p.max_tokens, 400);
        assert_eq!(p.n_ctx, 4096);
    }

    #[test]
    fn types_round_trip_via_serde() {
        let chunk = Chunk {
            chunk_id: "module:src/services:part-1".to_string(),
            chunk_type: ChunkType::Module,
            section_name: "module:src/services".to_string(),
            part_index: Some(1),
            total_parts: Some(3),
            content: "exports go here".to_string(),
            cross_references: vec!["module:src/routes".to_string()],
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let back: Chunk = serde_json::from_str(&json).unwrap();
        assert_eq!(chunk, back);
    }
}
