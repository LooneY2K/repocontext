//! Shared symbol types produced by language extractors.
//!
//! Each extractor (e.g. `repocontext-lang-ts`) produces a `Vec<Symbol>` plus
//! a parse-error flag. The synthesizer in `crate::synth` (phase 6) consumes
//! these to produce `context_temp.md`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Class,
    Interface,
    TypeAlias,
    Enum,
    Const,
    Method,
    Property,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Just the declaration up to (but not including) the body for
    /// functions/classes/methods. For declarations that are inherently
    /// signature-only (interface, type alias, enum, const), this is the full
    /// declaration source.
    pub signature: String,
    /// JSDoc-style block comment (`/** ... */`) immediately preceding the
    /// symbol, with only whitespace between. `None` if no doc comment.
    pub doc_comment: Option<String>,
    /// Full source text from start to end of the declaration node.
    pub source: String,
    pub start_byte: usize,
    pub end_byte: usize,
    /// 1-indexed line number of the declaration's first byte.
    pub start_line: usize,
    /// 1-indexed line number of the declaration's last byte.
    pub end_line: usize,
    /// For class members, the enclosing class name. `None` for top-level symbols.
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedSymbols {
    pub symbols: Vec<Symbol>,
    /// True if the parser flagged any syntax errors. Extraction continues;
    /// downstream consumers may surface this as a per-file warning.
    pub had_parse_errors: bool,
}

/// A discovered file plus its extracted symbols and full source text.
/// Used as the unit passed from indexing to salience scoring and synthesis.
/// The full source is needed for cross-file reference counting and for
/// the "Key Implementations" section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFile {
    pub relative_path: PathBuf,
    pub source: String,
    pub extracted: ExtractedSymbols,
}
