//! repocontext-enrich
//!
//! Stage 2 of repocontext: reads `context_temp.md`, splits it into chunks,
//! runs an embedded GGUF model (`llama.cpp` via `llama-cpp-2`) chunk-by-chunk,
//! and assembles the responses into `context.md`. Caches every output by
//! content hash via a pluggable [`cache::EnrichCache`] trait
//! (`JsonFileCache` is the default; `RedisCache` is opt-in).
//!
//! The hard guarantee: **the whole repo is covered**. Every section in
//! `context_temp.md` produces a corresponding section in `context.md`, with a
//! deterministic placeholder if the LLM fails. Sections that exceed the
//! model's context window are deterministically sub-split.

pub mod assembler;
pub mod backend;
pub mod cache;
pub mod chunker;
pub mod inference;
pub mod model;
pub mod orchestrator;
pub mod prompt;
pub mod prompts;
pub mod types;

pub use assembler::assemble;
pub use backend::{FailBackend, LlmBackend, MockBackend, PanicBackend, ScriptedBackend};
pub use cache::{EnrichCache, JsonFileCache, RedisCache};
pub use chunker::{chunk, ChunkerConfig};
pub use orchestrator::{
    assemble_basic, enrich, ChunkOutput, ChunkSource, EnrichConfig, EnrichResult,
    FALLBACK_PLACEHOLDER,
};
pub use prompt::PromptTemplate;
pub use prompts::template_for;
pub use types::{
    cache_key_hash, estimated_tokens, CachedEntry, Chunk, ChunkType, CompletionParams,
};
