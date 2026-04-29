//! Embedded GGUF inference via `llama-cpp-2`.
//!
//! Gated behind the `inference` Cargo feature. Without that feature,
//! [`LlamaCppBackend::load`] returns an actionable error explaining how to
//! enable it. This keeps the default `cargo install --path crates/repocontext-cli`
//! build fast (no C++ compile) for users who only need Stage 1 or who want
//! to wire in a different backend.
//!
//! # Enabling real inference
//!
//! ```sh
//! cargo install --path crates/repocontext-cli --features inference
//! # On Apple Silicon, prefer the Metal-accelerated build:
//! cargo install --path crates/repocontext-cli --features inference-metal
//! # On NVIDIA hardware (requires CUDA toolkit):
//! cargo install --path crates/repocontext-cli --features inference-cuda
//! ```
//!
//! The first build compiles `llama.cpp` from C++ source — expect 5–15 minutes
//! depending on hardware. Subsequent rebuilds are incremental.
//!
//! # Chat template
//!
//! Qwen2.5-Coder uses the ChatML format:
//!
//! ```text
//! <|im_start|>system
//! {system}<|im_end|>
//! <|im_start|>user
//! {user}<|im_end|>
//! <|im_start|>assistant
//! ```
//!
//! Generation stops on the `<|im_end|>` token.

use std::path::Path;

use anyhow::Result;

use crate::backend::LlmBackend;
use crate::types::CompletionParams;

/// Wraps the configured GGUF model + a llama.cpp inference context.
///
/// With the `inference` feature off, this is a placeholder type — calls to
/// [`LlamaCppBackend::load`] return an error explaining how to enable it.
pub struct LlamaCppBackend {
    #[cfg(feature = "inference")]
    inner: imp::Inner,
    #[cfg(not(feature = "inference"))]
    _phantom: std::marker::PhantomData<()>,
}

impl std::fmt::Debug for LlamaCppBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hide implementation details — `llama-cpp-2`'s types don't all
        // implement Debug, and exposing internals isn't useful anyway.
        f.debug_struct("LlamaCppBackend").finish_non_exhaustive()
    }
}

impl LlamaCppBackend {
    /// Load a GGUF model from disk and prepare an inference backend. Cheap to
    /// call repeatedly within a process — the underlying model is reused.
    pub fn load(model_path: &Path) -> Result<Self> {
        #[cfg(feature = "inference")]
        {
            let inner = imp::Inner::load(model_path)?;
            Ok(Self { inner })
        }
        #[cfg(not(feature = "inference"))]
        {
            let _ = model_path;
            anyhow::bail!(
                "llama-cpp-2 inference is not compiled in. \
                 Rebuild with `--features inference` (or `--features inference-metal` on Apple Silicon). \
                 The first build is slow (~5-15 min) because llama.cpp compiles from C++ source. \
                 Without this feature, --enrich falls back to MockBackend (deterministic placeholders)."
            )
        }
    }
}

impl LlmBackend for LlamaCppBackend {
    fn complete(&mut self, system: &str, user: &str, params: &CompletionParams) -> Result<String> {
        #[cfg(feature = "inference")]
        {
            self.inner.complete(system, user, params)
        }
        #[cfg(not(feature = "inference"))]
        {
            let _ = (system, user, params);
            anyhow::bail!(
                "LlamaCppBackend::complete called without `inference` feature \
                 — load() should have errored. This is a bug."
            )
        }
    }
}

#[cfg(feature = "inference")]
mod imp {
    //! Real implementation (only compiled when `inference` is enabled).

    use std::num::NonZeroU32;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::{anyhow, Context, Result};
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::AddBos;
    use llama_cpp_2::model::LlamaModel;
    #[allow(deprecated)]
    use llama_cpp_2::model::Special;
    use llama_cpp_2::sampling::LlamaSampler;

    use crate::types::CompletionParams;

    const QWEN_END_TOKEN: &str = "<|im_end|>";

    /// Replace Qwen ChatML control tokens that appear inside user-provided text
    /// before they reach the prompt template. Without this, a source file that
    /// contains `<|im_end|>\n<|im_start|>system\n...` could escape the user
    /// envelope and inject new system instructions. The replacement keeps the
    /// rendered prompt human-readable (so dry-run-llm logs are still useful)
    /// without giving the tokenizer any genuine control sequence to anchor on.
    fn sanitize_chatml(input: &str) -> std::borrow::Cow<'_, str> {
        if !input.contains("<|") {
            // Hot path: no candidate prefix at all.
            return std::borrow::Cow::Borrowed(input);
        }
        let cleaned = input
            .replace("<|im_start|>", "<|_im_start_|>")
            .replace("<|im_end|>", "<|_im_end_|>")
            .replace("<|endoftext|>", "<|_endoftext_|>");
        std::borrow::Cow::Owned(cleaned)
    }

    pub(super) struct Inner {
        backend: Arc<LlamaBackend>,
        model: LlamaModel,
    }

    impl Inner {
        pub(super) fn load(model_path: &Path) -> Result<Self> {
            let backend = Arc::new(LlamaBackend::init().context("initialising llama.cpp backend")?);
            // `n_gpu_layers` defaults to 0 on stock llama.cpp; with `metal`
            // enabled it auto-detects. We default to "all on GPU" by setting
            // a large number — llama.cpp clamps to actual layer count.
            let model_params = LlamaModelParams::default().with_n_gpu_layers(999);
            let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
                .with_context(|| format!("loading GGUF model from {}", model_path.display()))?;
            Ok(Self { backend, model })
        }

        pub(super) fn complete(
            &mut self,
            system: &str,
            user: &str,
            params: &CompletionParams,
        ) -> Result<String> {
            // Qwen2.5 ChatML template. Body uses Unix newlines because that's
            // what the tokenizer expects. Sanitize control tokens out of the
            // user/system content first — otherwise a source file containing
            // `<|im_end|>...<|im_start|>system\nNew instructions:` could break
            // out of the user envelope.
            let safe_system = sanitize_chatml(system.trim());
            let safe_user = sanitize_chatml(user.trim());
            let prompt = format!(
                "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
                safe_system, safe_user,
            );

            let n_ctx =
                NonZeroU32::new(params.n_ctx).ok_or_else(|| anyhow!("n_ctx must be > 0"))?;
            // `n_batch` and `n_ubatch` default to 512 in llama-cpp-2. When a
            // single prompt exceeds 512 tokens, decoding aborts with
            // `GGML_ASSERT(n_tokens_all <= cparams.n_batch)`. Bump both to
            // the full context size so any prompt that fits in n_ctx can be
            // decoded in one shot.
            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(Some(n_ctx))
                .with_n_batch(params.n_ctx)
                .with_n_ubatch(params.n_ctx);
            // The seed lives on the sampler in this version of llama-cpp-2 —
            // see `LlamaSampler::dist(seed)` below.

            let mut ctx = self
                .model
                .new_context(&self.backend, ctx_params)
                .context("creating llama context")?;

            let tokens = self
                .model
                .str_to_token(&prompt, AddBos::Always)
                .context("tokenising prompt")?;

            let prompt_len = tokens.len();
            if prompt_len as u32 >= params.n_ctx {
                anyhow::bail!(
                    "prompt is {} tokens but n_ctx is only {} — the chunker should have prevented this",
                    prompt_len,
                    params.n_ctx,
                );
            }

            let max_total = (prompt_len as u32 + params.max_tokens).min(params.n_ctx);

            // Initial decode: feed the prompt tokens. The batch capacity must
            // accommodate the largest single decode we'll perform — that's
            // the prompt length on the first call. A 512-token cap was too
            // small for chunks near the n_ctx limit; size to n_ctx instead.
            let batch_cap = params.n_ctx as usize;
            let mut batch = LlamaBatch::new(batch_cap, 1);
            let last_idx = (prompt_len - 1) as i32;
            for (i, token) in (0_i32..).zip(tokens.into_iter()) {
                let is_last = i == last_idx;
                batch
                    .add(token, i, &[0], is_last)
                    .context("adding prompt token to batch")?;
            }
            ctx.decode(&mut batch).context("decoding prompt")?;

            // Sampler: low temperature to match the spec's deterministic-style
            // output. We could expose more knobs (top_k, top_p) but for now
            // greedy + temperature is enough.
            let mut sampler = if params.temperature > 0.0 {
                LlamaSampler::chain_simple([
                    LlamaSampler::temp(params.temperature),
                    LlamaSampler::dist(params.seed as u32),
                ])
            } else {
                LlamaSampler::greedy()
            };

            // Track stop sequence "<|im_end|>". We accumulate decoded text
            // and break when it appears (or when EOS / max-tokens hit).
            let mut output_tokens = Vec::new();
            let mut decoded = String::new();
            let mut n_cur = batch.n_tokens();
            let eos_token = self.model.token_eos();

            while (n_cur as u32) < max_total {
                let next = sampler.sample(&ctx, batch.n_tokens() - 1);
                sampler.accept(next);

                if next == eos_token {
                    break;
                }

                // `token_to_str` is deprecated in this llama-cpp-2 version;
                // its replacement (`token_to_piece`) takes a streaming
                // `encoding_rs::Decoder` plus a max-size argument. For our
                // streaming-decode-with-stop-sequence-detection use case the
                // deprecated single-shot API is simpler. Migrate when it's
                // actually removed.
                #[allow(deprecated)]
                let piece = self
                    .model
                    .token_to_str(next, Special::Tokenize)
                    .unwrap_or_default();
                decoded.push_str(&piece);
                output_tokens.push(next);

                if decoded.contains(QWEN_END_TOKEN) {
                    // Strip the stop sequence from the tail.
                    if let Some(idx) = decoded.find(QWEN_END_TOKEN) {
                        decoded.truncate(idx);
                    }
                    break;
                }

                batch.clear();
                batch
                    .add(next, n_cur, &[0], true)
                    .context("adding sampled token")?;
                ctx.decode(&mut batch).context("decoding sampled token")?;
                n_cur += 1;
            }

            Ok(decoded.trim().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[cfg(not(feature = "inference"))]
    fn load_without_feature_returns_actionable_error() {
        let err = LlamaCppBackend::load(&PathBuf::from("/nope.gguf")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("inference"), "got: {msg}");
        assert!(msg.contains("--features inference"), "got: {msg}");
    }

    // Real-inference smoke test gated by REPOCONTEXT_TEST_LLAMA=1 and the
    // `inference` feature. Requires a GGUF file at the path in
    // `REPOCONTEXT_TEST_LLAMA_MODEL` (defaults to the standard Qwen cache
    // location).
    #[test]
    #[cfg(feature = "inference")]
    #[ignore = "real LLM inference; gate manually with REPOCONTEXT_TEST_LLAMA=1"]
    fn smoke_inference_against_real_model() {
        if std::env::var("REPOCONTEXT_TEST_LLAMA").is_err() {
            eprintln!("REPOCONTEXT_TEST_LLAMA not set; skipping");
            return;
        }
        let model_path = std::env::var("REPOCONTEXT_TEST_LLAMA_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::cache_dir()
                    .unwrap()
                    .join("repocontext/models/qwen2.5-coder-7b-instruct-q4_k_m.gguf")
            });
        let mut backend = LlamaCppBackend::load(&model_path).unwrap();
        let response = backend
            .complete(
                "You are a concise assistant.",
                "Say the word 'hello' and nothing else.",
                &CompletionParams::default(),
            )
            .unwrap();
        assert!(!response.is_empty(), "expected non-empty response");
        eprintln!("real-LLM response: {response:?}");
    }
}
