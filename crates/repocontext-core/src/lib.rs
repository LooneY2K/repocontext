//! repocontext-core
//!
//! Deterministic Stage 1: walks a codebase, parses it with tree-sitter (via
//! language-specific extractor crates), scores symbols by salience, and synthesizes
//! `context_temp.md`. No HTTP, no `clap`, no LLM dependencies — this crate is
//! intentionally pure so it can be used as a library in other tools.

pub mod config;
pub mod metadata;
pub mod salience;
pub mod symbol;
pub mod synth;
pub mod walker;

pub use config::Config;
pub use salience::{score_all, ScoredSymbol};
pub use symbol::{ExtractedSymbols, IndexedFile, Symbol, SymbolKind};
pub use synth::{synthesize_stage1, ProjectMetadata, SynthesisInput};
pub use walker::{walk, DiscoveredFile, WalkOptions};
