# repocontext

[![CI](https://github.com/LooneY2K/repocontext/actions/workflows/ci.yml/badge.svg)](https://github.com/LooneY2K/repocontext/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)

> Turn any codebase into a rich AI-ready context document — locally, privately, in seconds.

`repocontext` is a Rust CLI that produces two files from your codebase:

| File | What it contains | How fast |
|------|-----------------|----------|
| `context_temp.md` | Structural snapshot: every exported symbol, type signature, doc comment, directory layout, and salience-ranked implementation | ~1–2 seconds on any size repo |
| `context.md` | Narrative document: business logic, architecture, module purposes, domain model — written by a local LLM | ~3–10 minutes first run, instant thereafter (cached) |

Everything runs on your machine. No telemetry. No cloud APIs. The only network call is the one-time model download.

**See an example:** [`examples/sample-context_temp.md`](examples/sample-context_temp.md) — what Stage 1 produces for a small Go service.

## Features

- **Deterministic Stage 1** — `context_temp.md` is byte-identical across runs on the same input, making it safe to commit and diff in pull requests.
- **Embedded LLM** — `qwen2.5-coder:7b` runs in-process via `llama.cpp`. No Ollama, no Docker, no external runtime.
- **Metal + CUDA acceleration** — runs fast on Apple Silicon (M1/M2/M3) and NVIDIA GPUs.
- **Intelligent chunking** — when a codebase section is too large for the model's context window, it's deterministically sub-split. Every section always appears in the output, even if the LLM fails for that chunk.
- **Content-hash cache** — each LLM response is cached by SHA-256 of the input. Re-runs are instant. Commit the cache file so CI works without an LLM runtime.
- **TypeScript + Go** — `.ts`, `.tsx`, `.mts`, `.cts`, and `.go` files are parsed with tree-sitter. More languages are a drop-in extension.
- **No lock-in** — the output is plain Markdown. Use it with any AI tool.

## Quick start

> **Build prerequisite:** A C++ compiler is required for any `--features inference*` build because `llama.cpp` compiles from source. macOS: `xcode-select --install`. Debian/Ubuntu: `sudo apt install build-essential cmake pkg-config`. Windows: Visual Studio Build Tools with the C++ workload. Stage-1-only installs (no inference features) only need a Rust toolchain.

### Stage 1 only (no LLM, no model download)

```sh
cargo install --path crates/repocontext-cli

cd /your/project
repocontext init       # create .repocontext.toml and add to .gitignore
repocontext generate   # write context_temp.md
repocontext check      # exit 0 if current, 1 if stale (useful in CI)
```

`context_temp.md` is ready to paste into any AI chat as context. It contains every exported function, type, and class with full signatures and doc comments, ranked by how widely each symbol is referenced across the codebase.

### Stage 2 — real LLM narrative

Stage 2 writes `context.md`, a human-readable document describing what the codebase *does* and *why* — not just what symbols exist.

**Step 1: build with inference enabled**

```sh
# Apple Silicon (recommended — Metal-accelerated)
cargo install --path crates/repocontext-cli --features inference-metal

# NVIDIA GPU (requires CUDA toolkit installed at build time)
cargo install --path crates/repocontext-cli --features inference-cuda

# CPU-only (works everywhere, slower)
cargo install --path crates/repocontext-cli --features inference
```

The first build compiles `llama.cpp` from C++ source (~5–15 minutes). Subsequent rebuilds are incremental.

**Step 2: download the model**

```sh
repocontext model pull
# Downloads ~4.5 GB to ~/Library/Caches/repocontext/models/ (macOS)
#                    or ~/.cache/repocontext/models/          (Linux)
```

**Step 3: generate**

```sh
repocontext generate --enrich
```

The first run sends each chunk through the LLM and caches every response. Second run is instant — the cache serves everything without loading the model.

## Run with Docker

Prebuilt images on GHCR — skip the 5–15 minute `llama.cpp` compile.

### CPU image (any Linux/Windows host)

```sh
docker pull ghcr.io/looney2k/repocontext:cpu

# Stage 1 only
docker run --rm -v "$PWD:/workspace" ghcr.io/looney2k/repocontext:cpu generate

# Stage 2 with a persistent model + cache volume (model downloads on first run, ~4.5 GB)
docker run --rm \
  -v "$PWD:/workspace" \
  -v repocontext-cache:/root/.cache/repocontext \
  ghcr.io/looney2k/repocontext:cpu generate --enrich
```

### CUDA image (NVIDIA GPU)

Requires the [NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/) on the host.

```sh
docker pull ghcr.io/looney2k/repocontext:cuda

docker run --rm --gpus all \
  -v "$PWD:/workspace" \
  -v repocontext-cache:/root/.cache/repocontext \
  ghcr.io/looney2k/repocontext:cuda generate --enrich
```

### Notes

- The model file is *not* baked into the image. The first `--enrich` run downloads it to the `repocontext-cache` named volume; subsequent runs reuse it.
- To pull the model explicitly before running enrich: `docker run --rm -v repocontext-cache:/root/.cache/repocontext ghcr.io/looney2k/repocontext:cpu model pull`
- **Apple Silicon users:** install natively with `--features inference-metal` instead — Docker for Mac runs in a Linux VM and cannot expose Metal.

## Hardware requirements

| Setup | RAM | Speed |
|-------|-----|-------|
| CPU-only | 16 GB minimum | ~10–30 tokens/s |
| Apple Silicon M1+ | 16 GB unified memory | ~40–80 tokens/s |
| NVIDIA RTX 4060 (8 GB VRAM) | 8 GB VRAM + 8 GB system | ~50–100 tokens/s |

A medium-sized repo with ~20 chunks typically takes 3–8 minutes on first run and under a second on every subsequent run.

## Commands

```
repocontext init                              Create .repocontext.toml
repocontext generate                          Stage 1 only → context_temp.md
repocontext generate --enrich                 Stage 1 + 2 → context_temp.md + context.md
repocontext generate --enrich --dry-run-llm   Log prompts to stdout without calling the LLM
repocontext generate --enrich --no-cache      Bypass the cache this run
repocontext check                             Exit 0 if context_temp.md is current, 1 if stale
repocontext check --enrich                    Also validate context.md via cache
repocontext extract                           Dump indexed symbols as JSON (debug)
repocontext model pull                        Download the default model
repocontext model list                        Show cached models and sizes
repocontext model remove                      Delete the configured model file
```

All commands accept `--repo <path>` to target a directory other than the current one, plus `--quiet` and `--verbose`.

## Configuration

`repocontext init` writes a `.repocontext.toml` with all defaults. Every field is optional.

```toml
[output]
temp_path = "context_temp.md"
final_path = "context.md"
max_tokens = 8000              # raise for large repos

[exclude]
paths = [
    "**/test/**",
    "**/*.test.ts",
    "**/*.spec.ts",
    "**/*.generated.*",
]

[enrich]
temperature = 0.2              # lower = more focused output
max_tokens_per_request = 400   # max tokens per LLM response chunk
context_size = 4096            # model context window

[enrich.cache]
backend = "json"               # "json" (default) or "redis"
path = ".repocontext/enrich-cache.json"

[enrich.model]
name = "qwen2.5-coder-7b-instruct"
quantization = "q4_k_m"
# path_override = "/path/to/custom.gguf"
```

## Cache backends

**JSON (default):** A flat file at `.repocontext/enrich-cache.json`. Human-readable, committable to git, zero infrastructure required.

**Redis (opt-in):** Useful when multiple developers share a codebase — everyone gets cache hits for code they didn't touch. Connects lazily with a clear actionable error if the server isn't running.

```toml
[enrich.cache]
backend = "redis"
url = "redis://localhost:6379"
key_prefix = "repocontext:"
```

## CI usage

Stage 1 check (no LLM required):

```yaml
- run: repocontext check
```

Stage 2 check without running the LLM: remove `.repocontext/` from `.gitignore` and commit `enrich-cache.json` after each local Stage 2 run. The cache is content-hash keyed, so it only changes when the underlying source changes meaningfully.

```yaml
- run: repocontext check --enrich   # validates against committed cache, no inference needed
```

## Prompt iteration

To iterate on LLM output quality without running inference each time:

```sh
repocontext generate --enrich --dry-run-llm
```

This logs every rendered prompt to stdout without calling the model. Paste a prompt into any local inference UI (LM Studio, Jan, koboldcpp), iterate on the wording, edit the corresponding template in `prompts/`, bump its `# version:` line (which invalidates that task's cache entries), then re-run.

## Supported languages

| Language | Extensions |
|----------|-----------|
| TypeScript | `.ts` `.tsx` `.mts` `.cts` |
| Go | `.go` |

Adding a new language requires one crate implementing a single `extract(source, path) -> ExtractedSymbols` function.

## Privacy

All inference runs locally. Nothing is sent to external servers. The only outbound request is the one-time model download from Hugging Face. `context_temp.md` and `context.md` never leave your machine unless you explicitly share them.

## Contributing

Bug reports, prompt-template improvements, and new language extractors are all welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and [SECURITY.md](SECURITY.md) for vulnerability reporting.

## License

Apache-2.0. See [LICENSE](LICENSE).
