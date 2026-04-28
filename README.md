# repocontext

A two-stage CLI that produces structured Markdown context for codebases — built for use as input to AI tools (Claude, ChatGPT, Cursor, etc.).

- **Stage 1 (deterministic):** parses the codebase with `tree-sitter`, extracts symbols and signatures, and writes `context_temp.md`. Fast, reproducible, no LLM involved.
- **Stage 2 (embedded LLM, opt-in):** loads `qwen2.5-coder:7b` (Q4_K_M GGUF) in-process via `llama.cpp`, reads `context_temp.md`, and produces `context.md` describing the business logic and purpose of the codebase.

Language support: TypeScript (`.ts`, `.tsx`, `.mts`, `.cts`) and Go (`.go`). Each language is a separate extractor crate (`repocontext-lang-ts`, `repocontext-lang-go`); adding more is a drop-in.

Status: under active development. Full README, install instructions, and CI usage docs land at v0.2 release time.
# repocontext
