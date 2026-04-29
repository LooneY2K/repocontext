# Contributing to repocontext

Thanks for your interest in contributing! This is a personal project, but PRs are welcome — bug fixes, new language extractors, prompt-template improvements, docs, anything.

## Before you start

- For non-trivial changes, please open an issue first so we can agree on scope before you spend time.
- For typos, tiny fixes, or new tests, just open the PR.

## Development setup

You need a Rust toolchain (stable, minimum **1.75**) and a C++ compiler (for building `llama.cpp` if you touch the inference path):

- macOS: `xcode-select --install`
- Debian/Ubuntu: `sudo apt install build-essential cmake pkg-config`
- Windows: Visual Studio Build Tools with the C++ workload

Clone and build:

```sh
git clone https://github.com/LooneY2K/repocontext.git
cd repocontext
cargo build --workspace
cargo test --workspace
```

Stage 2 (LLM-backed) tests are gated behind feature flags and environment variables:

- `REPOCONTEXT_TEST_LLAMA=1` — runs real-inference tests against a tiny GGUF fixture
- `REPOCONTEXT_TEST_REDIS=1` — runs the Redis-backed cache integration test (needs `redis-server` running)

The default test suite is fully mock-backed and runs in seconds.

## Before opening a PR

CI will run all of these — running them locally first saves a round trip:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Aim for:

- **Tests for new behaviour.** New code paths should have unit tests; bug fixes should ship with a regression test.
- **Determinism.** Stage 1 output must be byte-identical across runs on the same input. If your change could break that, write a determinism test.
- **No new `unwrap()` / `expect()` in production code paths.** Tests can use them; library/CLI code should propagate errors with `anyhow::Context`.
- **Doc comments on public APIs.** A one-line `///` is enough; explain *why* the function exists, not just what it returns.

## Adding a new language

The contract is small: a crate (`repocontext-lang-<lang>`) that exposes:

```rust
pub fn extract(source: &str, path: &Path) -> ExtractedSymbols;
```

`ExtractedSymbols` is defined in `repocontext-core::symbol`. The TS extractor (`crates/repocontext-lang-ts`) and Go extractor (`crates/repocontext-lang-go`) are good references. Wire it into the orchestrator's file-extension dispatch in `crates/repocontext-cli/src/orchestrator.rs`.

## Tweaking prompts

The Stage 2 LLM prompts live in [`prompts/`](prompts/). Each template has a `# version: N` header at the top — bump it whenever you change the prompt body so the cache invalidates entries that were generated with the old wording.

To iterate on a prompt without firing the LLM:

```sh
repocontext generate --enrich --dry-run-llm
```

That logs every rendered prompt to stdout. Paste one into LM Studio / Jan / koboldcpp and edit the prompt file based on what you see.

## Reporting bugs

Use the [bug report template](https://github.com/LooneY2K/repocontext/issues/new?template=bug_report.md). Please include:

- The `repocontext --version` you're running
- Your OS and architecture
- The output of `repocontext generate --verbose 2>&1 | tail -50`
- A minimal repro repo if possible

## Reporting security issues

**Do not open a public issue** for security vulnerabilities. See [SECURITY.md](SECURITY.md).

## Code of conduct

Be kind. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

By contributing, you agree your contributions are licensed under the [Apache-2.0 License](LICENSE) the project uses.
