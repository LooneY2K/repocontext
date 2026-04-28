# repocontext

A two-stage CLI that produces structured Markdown context for codebases — built for use as input to AI tools (Claude, ChatGPT, Cursor, etc.).

- **Stage 1 (deterministic):** parses the codebase with `tree-sitter`, extracts symbols and signatures, and writes `context_temp.md`. Fast, reproducible, no LLM involved.
- **Stage 2 (embedded LLM, opt-in):** loads `qwen2.5-coder:7b` (Q4_K_M GGUF) in-process via `llama.cpp`, reads `context_temp.md`, and produces `context.md` describing the **description, flow, and business logic** of the codebase. Code blocks are copied verbatim — the model never reproduces code.

Language support: TypeScript (`.ts`, `.tsx`, `.mts`, `.cts`) and Go (`.go`). Adding more is a drop-in via a new `repocontext-lang-*` crate.

## Quick start (Stage 1 only — no LLM required)

```sh
# from this repo
cargo install --path crates/repocontext-cli

# in any TypeScript or Go project
repocontext init           # writes .repocontext.toml + appends .gitignore
repocontext generate       # walks, parses, writes context_temp.md
repocontext check          # exit 0 if context_temp.md is up to date, 1 if stale
repocontext extract        # debug: dump the indexed file set + scored symbols as JSON
```

Stage 1 alone is enough for many use cases — `context_temp.md` is a structured, deterministic snapshot of every exported symbol, the directory layout, the data models, and the highest-salience implementations (ranked by cross-file reference count).

For large repos, raise the token budget in `.repocontext.toml`:

```toml
[output]
max_tokens = 80000
```

## Enabling Stage 2 (real LLM narrative)

Stage 2 produces `context.md` — a polished narrative describing the project's purpose, architecture, modules, domain model, and key behaviours. It runs `qwen2.5-coder:7b` locally via embedded `llama.cpp`. Nothing leaves your machine except the one-time model download from Hugging Face.

### 1. Build with the inference feature

```sh
# Apple Silicon (Metal-accelerated, recommended)
cargo install --path crates/repocontext-cli --features inference-metal

# NVIDIA GPUs (requires CUDA toolkit at build time)
cargo install --path crates/repocontext-cli --features inference-cuda

# CPU-only
cargo install --path crates/repocontext-cli --features inference
```

The first build is slow (~5–15 minutes) because `llama.cpp` compiles from C++ source. Subsequent rebuilds are incremental.

### 2. Pull the model

```sh
repocontext model pull
# downloads ~4.5 GB to ~/.cache/repocontext/models/qwen2.5-coder-7b-instruct-q4_k_m.gguf
```

Other model commands:

```sh
repocontext model list      # show what's cached + sizes
repocontext model remove    # delete the configured model
```

### 3. Generate `context.md`

```sh
repocontext generate --enrich
```

What happens:

1. Stage 1 runs → `context_temp.md`.
2. Chunker splits `context_temp.md` by section markers (`<!-- repocontext:section=… -->`). Modules that exceed the model's context window are sub-split deterministically.
3. Each chunk's prompt is rendered (templates in `prompts/`) and sent to the local Qwen.
4. Outputs are cached by SHA-256 of `(prompt_version, model_id, chunk_input)` to `.repocontext/enrich-cache.json`. Re-running with the same source code is a no-op (every chunk hits the cache).
5. Outputs are stitched into `context.md` with sections: `# Project`, `## Architecture`, `## Modules`, `## Domain Model`, `## Key Behaviors`. Code blocks are copied verbatim from `context_temp.md`.

### Coverage guarantee

Every section in `context_temp.md` produces a corresponding section in `context.md`, even when the LLM fails. On any error path (timeout, malformed response, panic, missing model), the affected section gets a deterministic placeholder — never a silent drop. This is enforced by an integration test (`coverage_under_total_failure`).

### Prompt iteration without firing the LLM

```sh
repocontext generate --enrich --dry-run-llm
```

Logs every rendered prompt to stdout instead of calling the model. Paste a prompt into LM Studio / Jan / koboldcpp to test it manually, edit the corresponding `prompts/chunk_*.md` file, bump its `# version:` (which invalidates that task's cache entries), re-run.

## Hardware

- **CPU-only**: 16 GB RAM minimum for Q4_K_M. Expect ~10–30 tokens/s.
- **Apple Silicon (M1+)**: Metal kicks in automatically with `--features inference-metal`. ~30–60 tokens/s on M2 Pro.
- **NVIDIA RTX 4060 / 8 GB VRAM**: comfortable; needs `--features inference-cuda` and the CUDA toolkit at build time.
- **Disk**: ~5 GB free for the model cache (`~/.cache/repocontext/models/`).
- **Build toolchain**: a C++ compiler (Xcode CLT on macOS, `build-essential` on Linux, Visual Studio build tools on Windows) is required because `llama.cpp` compiles from source.

A typical Stage 2 run on a medium repo is ~20 chunks × ~150 tokens ≈ 5 minutes on CPU-only, sub-minute on Metal. The cache makes second runs trivial.

## Cache backends

`.repocontext.toml` controls the cache:

```toml
[enrich.cache]
backend = "json"                              # "json" | "redis"
path = ".repocontext/enrich-cache.json"       # used when backend = "json"
url = "redis://localhost:6379"                # used when backend = "redis"
key_prefix = "repocontext:"                   # used when backend = "redis"
```

- **`json`** (default): a flat JSON file under `.repocontext/`. Zero-config, human-readable, and committable to git so `repocontext check --enrich` works in CI without an LLM runtime.
- **`redis`**: opt-in. Useful for **shared team caches** — point everyone's `redis://` URL at the same instance and re-runs across the team hit the same cache. Connect lazily; the first failed connection surfaces an actionable error (`brew services start redis`).

To use Redis:

```sh
brew services start redis      # or `redis-server`
# edit .repocontext.toml: backend = "redis"
repocontext generate --enrich
redis-cli keys "repocontext:*" # cached entries visible
```

## Configuration reference

```toml
[output]
temp_path = "context_temp.md"
final_path = "context.md"
profile = "full"
max_tokens = 8000              # bump for large repos (the chunker handles oversize sections)

[include]
paths = ["."]
languages = ["typescript", "go"]   # informational; orchestrator dispatches by file extension

[exclude]
paths = ["**/test/**", "**/*.test.ts", "**/*.spec.ts", "**/*.generated.*"]

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
name = "qwen2.5-coder-7b-instruct"
quantization = "q4_k_m"
context_size = 4096
gpu_layers = -1                # -1 = auto (Metal on macOS, CPU otherwise)
# path_override = "/custom/path/to/model.gguf"
# cache_dir = "/custom/cache/dir"

[enrich.cache]
backend = "json"
path = ".repocontext/enrich-cache.json"
url = "redis://localhost:6379"
key_prefix = "repocontext:"

[profiles.minimal]
max_tokens = 1500

[profiles.api_only]
sections = ["overview", "data_models"]
```

Profiles let you switch between full and trimmed views: `repocontext generate --profile minimal`.

## CI usage

`repocontext check` is the CI entrypoint. Run it after build:

```yaml
# GitHub Actions example
- run: cargo install --path crates/repocontext-cli   # or download a release binary
- run: repocontext check                              # Stage 1 only
- run: repocontext check --enrich                     # also validates context.md via cache
```

To enable `check --enrich` in CI without an LLM runtime: remove `.repocontext/` from `.gitignore` and commit `enrich-cache.json` after each Stage 2 run. The cache is content-hash keyed, so it only changes when source changes meaningfully.

## Privacy

Everything runs locally. The only network call is the one-time GGUF download from Hugging Face. Model + inference + cache are all local — no telemetry, no remote LLM APIs.

## Layout

```
repocontext/
├── crates/
│   ├── repocontext-cli/        # binary crate (clap)
│   ├── repocontext-core/       # config, walker, salience, Stage 1 synthesis
│   ├── repocontext-lang-ts/    # tree-sitter TypeScript/TSX extractor
│   ├── repocontext-lang-go/    # tree-sitter Go extractor
│   └── repocontext-enrich/     # Stage 2: chunker, cache (JSON+Redis), inference, assembler
├── prompts/                    # versioned prompt templates (loaded via include_str!)
└── tests/fixtures/             # sample TS / Go projects for integration tests
```

## Status

- Stage 1 (deterministic): ✅ shipped (TypeScript + Go).
- Stage 2 (embedded LLM): ✅ wired end-to-end. With `--features inference-metal` (or `inference-cuda` / `inference`) plus the model pulled, `--enrich` produces real Qwen-generated narratives. Without the feature, the same flag falls back to a deterministic `MockBackend` (placeholder content) so the pipeline can be verified without paying the C++ compile cost.

## License

Apache-2.0. See [LICENSE](LICENSE).
