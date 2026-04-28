//! Pluggable LLM backend abstraction.
//!
//! [`LlmBackend`] is the only thing the Stage 2 orchestrator knows about.
//! Real inference (`LlamaCppBackend`, phase 19) and the test backends
//! ([`MockBackend`], [`FailBackend`], [`PanicBackend`], [`ScriptedBackend`])
//! all implement the same trait, so the rest of the pipeline can be tested
//! without dragging in a 4.5 GB GGUF model.

use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::types::CompletionParams;

/// The single hook for "given a system + user prompt, return text". Sync —
/// llama.cpp inference is single-threaded anyway, and we currently cap
/// `max_concurrent_requests` at 1 in config.
pub trait LlmBackend: Send {
    fn complete(&mut self, system: &str, user: &str, params: &CompletionParams) -> Result<String>;
}

/// Returns deterministic synthetic responses derived from the input hash.
/// Same input produces the same output so cache-replay tests are stable.
/// Different inputs produce different outputs so tests can verify the backend
/// is actually consulted.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockBackend;

impl LlmBackend for MockBackend {
    fn complete(
        &mut self,
        _system: &str,
        user: &str,
        _params: &CompletionParams,
    ) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(user.as_bytes());
        let hex = format!("{:x}", hasher.finalize());
        Ok(format!(
            "[Mock summary {}: this is a synthetic response.]",
            &hex[..8]
        ))
    }
}

/// Returns an error on every call. Used by the coverage integration test to
/// prove that every input section still produces a (placeholder) output
/// section, even when the LLM is completely broken.
#[derive(Debug, Default, Clone, Copy)]
pub struct FailBackend;

impl LlmBackend for FailBackend {
    fn complete(
        &mut self,
        _system: &str,
        _user: &str,
        _params: &CompletionParams,
    ) -> Result<String> {
        Err(anyhow!("FailBackend: simulated LLM failure"))
    }
}

/// Panics on any call. Used by the cache-replay test to prove that a fully
/// populated cache satisfies every chunk without ever consulting the LLM —
/// this is what makes `repocontext check --enrich` work in CI without any
/// inference runtime.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanicBackend;

impl LlmBackend for PanicBackend {
    fn complete(
        &mut self,
        _system: &str,
        user: &str,
        _params: &CompletionParams,
    ) -> Result<String> {
        panic!(
            "PanicBackend was called — the cache should have satisfied this chunk. \
             User prompt prefix: {:?}",
            &user[..user.len().min(120)]
        );
    }
}

/// Returns pre-set responses in FIFO order. Useful for snapshot tests where
/// you want fully deterministic, hand-crafted output text.
#[derive(Debug, Clone)]
pub struct ScriptedBackend {
    pub responses: VecDeque<String>,
}

impl ScriptedBackend {
    pub fn new<I, S>(responses: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            responses: responses.into_iter().map(Into::into).collect(),
        }
    }
}

impl LlmBackend for ScriptedBackend {
    fn complete(
        &mut self,
        _system: &str,
        _user: &str,
        _params: &CompletionParams,
    ) -> Result<String> {
        self.responses
            .pop_front()
            .ok_or_else(|| anyhow!("ScriptedBackend ran out of pre-set responses"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> CompletionParams {
        CompletionParams::default()
    }

    #[test]
    fn mock_is_deterministic() {
        let mut a = MockBackend;
        let mut b = MockBackend;
        let r1 = a.complete("sys", "user", &params()).unwrap();
        let r2 = b.complete("sys", "user", &params()).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn mock_distinguishes_inputs() {
        let mut m = MockBackend;
        let r1 = m.complete("sys", "alpha", &params()).unwrap();
        let r2 = m.complete("sys", "beta", &params()).unwrap();
        assert_ne!(r1, r2);
    }

    #[test]
    fn fail_returns_error() {
        let mut f = FailBackend;
        let err = f.complete("sys", "user", &params()).unwrap_err();
        assert!(err.to_string().contains("FailBackend"));
    }

    #[test]
    #[should_panic(expected = "PanicBackend was called")]
    fn panic_panics() {
        let mut p = PanicBackend;
        let _ = p.complete("sys", "user", &params());
    }

    #[test]
    fn scripted_replays_in_order() {
        let mut s = ScriptedBackend::new(["first", "second", "third"]);
        assert_eq!(s.complete("", "", &params()).unwrap(), "first");
        assert_eq!(s.complete("", "", &params()).unwrap(), "second");
        assert_eq!(s.complete("", "", &params()).unwrap(), "third");
        let err = s.complete("", "", &params()).unwrap_err();
        assert!(err.to_string().contains("ran out"));
    }

    #[test]
    fn trait_object_is_usable() {
        // Sanity: backends can be used through a trait object.
        let mut backends: Vec<Box<dyn LlmBackend>> =
            vec![Box::new(MockBackend), Box::new(FailBackend)];
        let r0 = backends[0].complete("", "x", &params()).unwrap();
        assert!(r0.starts_with("[Mock summary"));
        let r1 = backends[1].complete("", "x", &params());
        assert!(r1.is_err());
    }
}
