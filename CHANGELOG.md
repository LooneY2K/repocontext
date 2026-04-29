# Changelog

All notable changes to this project are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project follows [SemVer](https://semver.org/) once it reaches 1.0.0 — until then, minor releases (0.x.0) may include breaking changes and patch releases (0.x.y) are non-breaking.

## [Unreleased]

## [0.2.0] — 2026-04-30

First public release.

### Added

- **Stage 1 — deterministic codebase indexing.** `repocontext generate` walks the project with `tree-sitter`, extracts every exported symbol with its signature and doc comment, ranks implementations by salience (cross-file reference count + doc presence + size), and writes a byte-deterministic `context_temp.md` ready to paste into any AI tool.
- **Stage 2 — embedded LLM narrative.** `repocontext generate --enrich` loads `qwen2.5-coder:7b` (Q4_K_M GGUF) in-process via `llama.cpp` and writes `context.md` describing architecture, module purposes, domain model, and key behaviors. Code blocks are copied verbatim from `context_temp.md` — the model never reproduces code.
- **Coverage guarantee.** Every input section produces an output section, even when the LLM fails (deterministic placeholder). Enforced by integration test.
- **Cache-replay determinism.** Re-runs with a populated cache are byte-identical and never load the model. Gates `repocontext check --enrich` for CI.
- **Pluggable cache backends.** `JsonFileCache` (default, committable to git for CI) and `RedisCache` (opt-in, shared team caches).
- **Language support.** TypeScript (`.ts`, `.tsx`, `.mts`, `.cts`) and Go (`.go`). Adding a new language is a single `extract(source, path) -> ExtractedSymbols` function in a new crate.
- **Hardware acceleration.** Apple Silicon (Metal, `--features inference-metal`) and NVIDIA GPUs (CUDA, `--features inference-cuda`).
- **Docker images** at `ghcr.io/looney2k/repocontext:cpu` (multi-arch) and `:cuda` (linux/amd64).
- **CLI commands.** `init`, `generate`, `check`, `extract`, `model {pull,list,remove}`.

### Security

- Model downloads are **HTTPS-only** and **SHA-256 verified** against a hash baked into the binary. Mismatches abort with an actionable error.
- **Path-traversal guard.** `.repocontext.toml` paths containing `..` segments are rejected at config-load time.
- **Walker file-count cap.** Default 50,000 files per walk to prevent OOM when pointed at the wrong directory.
- **Hardcoded excludes for secrets and lockfiles** — `.env*` (excluding `.env.example` / `.env.template`), `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Gemfile.lock`, `composer.lock`, `poetry.lock`, `uv.lock`, `Pipfile.lock`.
- **Prompt-injection guard.** Qwen ChatML control tokens (`<|im_start|>`, `<|im_end|>`, `<|endoftext|>`) appearing in user-provided source/text are sanitized before reaching the prompt template.

[Unreleased]: https://github.com/LooneY2K/repocontext/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/LooneY2K/repocontext/releases/tag/v0.2.0
