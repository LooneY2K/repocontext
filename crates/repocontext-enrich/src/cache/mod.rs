//! Pluggable cache for enrichment outputs.
//!
//! Stage 2 caches every chunk's LLM output by SHA-256 of
//! `(prompt_version, model_id, chunk_input)` (see [`crate::cache_key_hash`]).
//! With a populated cache, re-running `--enrich` skips inference entirely —
//! that's what makes `repocontext check --enrich` work in CI without a model
//! runtime.
//!
//! Two backends:
//! - [`json::JsonFileCache`] — default, a flat JSON file at
//!   `.repocontext/enrich-cache.json`. Zero-config and committable to git.
//! - [`redis::RedisCache`] — opt-in, for shared team caches.
//!   (Implementation lands in phase 20.)

pub mod json;
pub mod redis;

use anyhow::Result;

use crate::types::CachedEntry;

/// The cache contract. Implementations MUST be Send + Sync — the orchestrator
/// passes `&dyn EnrichCache` around freely. Methods take `&self`; interior
/// mutability is the implementation's concern.
pub trait EnrichCache: Send + Sync {
    /// Look up a cached output by hex-encoded SHA-256 cache key.
    fn get(&self, key: &str) -> Result<Option<CachedEntry>>;

    /// Insert / overwrite a cached output. Implementations may buffer; call
    /// [`flush`](Self::flush) to commit to durable storage.
    fn put(&self, key: &str, entry: CachedEntry) -> Result<()>;

    /// Persist any in-memory state to durable storage. No-op if nothing changed.
    fn flush(&self) -> Result<()>;
}

pub use json::JsonFileCache;
pub use redis::RedisCache;
