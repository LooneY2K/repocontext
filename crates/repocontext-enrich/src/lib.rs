//! repocontext-enrich
//!
//! Stage 2 of repocontext: reads `context_temp.md`, splits it into chunks,
//! runs an embedded GGUF model (`llama.cpp` via `llama-cpp-2`) chunk-by-chunk,
//! and assembles the responses into `context.md`. Caches every output by
//! content hash via a pluggable `EnrichCache` trait (`JsonFileCache` is the
//! default; `RedisCache` is opt-in for shared team caches).
//!
//! Stub for phase 1. Types + prompt loader + LlmBackend trait land in phase 9,
//! model download in phase 10, and embedded inference in phase 11.

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert_eq!(2 + 2, 4);
    }
}
