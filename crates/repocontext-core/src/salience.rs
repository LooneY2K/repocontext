//! Salience scoring for extracted symbols.
//!
//! Combines a few signals into a per-symbol importance score. The synthesizer
//! uses this to pick "Key Implementations" and to decide what to drop when
//! over the token budget.
//!
//! Formula (verbatim from the spec):
//!
//! ```text
//! salience = log(reference_count + 1) * 3.0
//!          + has_doc_comment * 2.0
//!          + manually_marked_important * 10.0
//!          + clamp(symbol_size_lines / 50, 0, 1) * 1.5
//!          - is_test_file * 5.0
//! ```
//!
//! ## Reference counting is a heuristic, not a resolver
//!
//! For each symbol we count occurrences of the identifier (with `\b` word
//! boundaries) across every other file in the index. We do NOT resolve
//! imports, follow re-exports, or track scopes. This means:
//!
//! - A symbol named `add` will be inflated by every other unrelated `add` in
//!   the codebase.
//! - Symbols with overly generic names (`init`, `run`, `value`) will be
//!   inflated by language-keyword-adjacent uses.
//! - Aliased imports (`import { add as plus }`) are not tracked.
//!
//! That's deliberate — the spec describes this as a salience signal, not a
//! correctness guarantee.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::symbol::{IndexedFile, Symbol};

/// A symbol with the file it came from, its reference count, and its salience score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSymbol {
    pub symbol: Symbol,
    pub file_path: PathBuf,
    pub reference_count: usize,
    pub salience: f64,
    pub is_test_file: bool,
    pub manually_important: bool,
}

/// Score every symbol across all indexed files. Returns a flat vector with the
/// scoring metadata attached. Order matches `files` and within-file symbol order.
pub fn score_all(files: &[IndexedFile]) -> Vec<ScoredSymbol> {
    // Cache ref counts per (name, defining_file_path) so we don't re-grep
    // when the same name appears multiple times across files (rare, but keeps
    // the implementation honest).
    let mut ref_cache: HashMap<(String, PathBuf), usize> = HashMap::new();

    let mut out = Vec::with_capacity(files.iter().map(|f| f.extracted.symbols.len()).sum());
    for file in files {
        let test_file = is_test_file(&file.relative_path);
        for symbol in &file.extracted.symbols {
            let cache_key = (symbol.name.clone(), file.relative_path.clone());
            let ref_count = if let Some(c) = ref_cache.get(&cache_key) {
                *c
            } else {
                let c = count_references(&symbol.name, files, &file.relative_path);
                ref_cache.insert(cache_key, c);
                c
            };
            let lines = symbol.end_line.saturating_sub(symbol.start_line) + 1;
            let manually_important = is_manually_important(symbol);
            let salience =
                compute_salience(symbol, ref_count, test_file, manually_important, lines);
            out.push(ScoredSymbol {
                symbol: symbol.clone(),
                file_path: file.relative_path.clone(),
                reference_count: ref_count,
                salience,
                is_test_file: test_file,
                manually_important,
            });
        }
    }
    out
}

/// Detect a JSDoc tag that opts a symbol into the "manually important" boost.
/// Recognized markers: `@important`, `@public-api`. Documented for users.
pub fn is_manually_important(symbol: &Symbol) -> bool {
    symbol
        .doc_comment
        .as_deref()
        .is_some_and(|d| d.contains("@important") || d.contains("@public-api"))
}

/// Heuristic detection of test files based on path conventions.
pub fn is_test_file(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    s.contains("/test/")
        || s.contains("/tests/")
        || s.contains("/__tests__/")
        || s.starts_with("test/")
        || s.starts_with("tests/")
        || s.ends_with(".test.ts")
        || s.ends_with(".test.tsx")
        || s.ends_with(".test.js")
        || s.ends_with(".test.jsx")
        || s.ends_with(".spec.ts")
        || s.ends_with(".spec.tsx")
        || s.ends_with(".spec.js")
        || s.ends_with(".spec.jsx")
}

fn compute_salience(
    symbol: &Symbol,
    ref_count: usize,
    is_test_file: bool,
    manually_important: bool,
    lines: usize,
) -> f64 {
    let log_refs = ((ref_count + 1) as f64).ln();
    let has_doc = f64::from(u32::from(symbol.doc_comment.is_some()));
    let manually = f64::from(u32::from(manually_important));
    let size_factor = ((lines as f64) / 50.0).clamp(0.0, 1.0);
    let test_penalty = f64::from(u32::from(is_test_file));

    log_refs * 3.0 + has_doc * 2.0 + manually * 10.0 + size_factor * 1.5 - test_penalty * 5.0
}

fn count_references(name: &str, files: &[IndexedFile], excluding: &Path) -> usize {
    if !is_valid_identifier(name) {
        return 0;
    }
    let pattern = format!(r"\b{}\b", regex::escape(name));
    let Ok(re) = Regex::new(&pattern) else {
        return 0;
    };
    files
        .iter()
        .filter(|f| f.relative_path != excluding)
        .map(|f| re.find_iter(&f.source).count())
        .sum()
}

fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::{ExtractedSymbols, Symbol, SymbolKind};

    fn mk_symbol(name: &str, start_line: usize, end_line: usize, doc: Option<&str>) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            signature: format!("export function {name}()"),
            doc_comment: doc.map(|s| s.to_string()),
            source: format!("export function {name}() {{}}"),
            start_byte: 0,
            end_byte: 0,
            start_line,
            end_line,
            parent: None,
        }
    }

    fn mk_file(path: &str, source: &str, symbols: Vec<Symbol>) -> IndexedFile {
        IndexedFile {
            relative_path: PathBuf::from(path),
            source: source.to_string(),
            extracted: ExtractedSymbols {
                symbols,
                had_parse_errors: false,
            },
        }
    }

    #[test]
    fn detects_test_files() {
        assert!(is_test_file(Path::new("src/auth.test.ts")));
        assert!(is_test_file(Path::new("src/auth.spec.tsx")));
        assert!(is_test_file(Path::new("test/auth.ts")));
        assert!(is_test_file(Path::new("tests/integration/auth.ts")));
        assert!(is_test_file(Path::new("src/__tests__/auth.ts")));
        assert!(is_test_file(Path::new("packages/foo/test/x.ts")));

        assert!(!is_test_file(Path::new("src/auth.ts")));
        assert!(!is_test_file(Path::new("src/test_helpers.ts"))); // not in test dir
        assert!(!is_test_file(Path::new("src/testing-utils.ts")));
    }

    #[test]
    fn higher_ref_count_yields_higher_salience() {
        let files = vec![
            mk_file(
                "src/util.ts",
                "export function popular() {}",
                vec![mk_symbol("popular", 1, 1, None)],
            ),
            mk_file(
                "src/a.ts",
                "import { popular } from './util'; popular(); popular();",
                vec![],
            ),
            mk_file(
                "src/b.ts",
                "import { popular } from './util'; popular();",
                vec![],
            ),
            mk_file(
                "src/lonely.ts",
                "export function lonely() {}",
                vec![mk_symbol("lonely", 1, 1, None)],
            ),
        ];
        let scored = score_all(&files);
        let popular = scored.iter().find(|s| s.symbol.name == "popular").unwrap();
        let lonely = scored.iter().find(|s| s.symbol.name == "lonely").unwrap();
        assert!(popular.reference_count >= 3);
        assert_eq!(lonely.reference_count, 0);
        assert!(popular.salience > lonely.salience);
    }

    #[test]
    fn doc_comment_adds_two_points() {
        let with_doc = mk_symbol("foo", 1, 1, Some("/** doc */"));
        let without_doc = mk_symbol("bar", 1, 1, None);
        let s_with = compute_salience(&with_doc, 0, false, false, 1);
        let s_without = compute_salience(&without_doc, 0, false, false, 1);
        assert!((s_with - s_without - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_file_penalty_applies() {
        let sym = mk_symbol("foo", 1, 1, None);
        let normal = compute_salience(&sym, 0, false, false, 1);
        let test = compute_salience(&sym, 0, true, false, 1);
        assert!((normal - test - 5.0).abs() < 1e-9);
    }

    #[test]
    fn manually_important_adds_ten() {
        let sym = mk_symbol("foo", 1, 1, None);
        let normal = compute_salience(&sym, 0, false, false, 1);
        let important = compute_salience(&sym, 0, false, true, 1);
        assert!((important - normal - 10.0).abs() < 1e-9);
    }

    #[test]
    fn size_factor_clamped() {
        let sym = mk_symbol("foo", 1, 1, None);
        let small = compute_salience(&sym, 0, false, false, 10);
        let medium = compute_salience(&sym, 0, false, false, 50);
        let huge = compute_salience(&sym, 0, false, false, 5000);
        // 50-line gives full 1.5; smaller is proportional; huge is clamped to 1.5
        assert!(small < medium);
        assert!((medium - huge).abs() < 1e-9);
    }

    #[test]
    fn manually_important_via_jsdoc() {
        let imp = mk_symbol("foo", 1, 1, Some("/** Public API. @important */"));
        let pub_api = mk_symbol("foo", 1, 1, Some("/** @public-api */"));
        let plain = mk_symbol("foo", 1, 1, Some("/** ordinary */"));
        assert!(is_manually_important(&imp));
        assert!(is_manually_important(&pub_api));
        assert!(!is_manually_important(&plain));
    }

    #[test]
    fn ref_count_excludes_defining_file() {
        let files = vec![mk_file(
            "src/a.ts",
            // Symbol's name appears 3 times in the file but should not count
            "export function foo() { foo(); foo(); }",
            vec![mk_symbol("foo", 1, 1, None)],
        )];
        let scored = score_all(&files);
        assert_eq!(scored[0].reference_count, 0);
    }

    #[test]
    fn ref_count_uses_word_boundaries() {
        let files = vec![
            mk_file(
                "src/a.ts",
                "export function add() {}",
                vec![mk_symbol("add", 1, 1, None)],
            ),
            // `addItem` and `paddle` should NOT match `add`
            mk_file(
                "src/b.ts",
                "function addItem() {} const paddle = 1;",
                vec![],
            ),
            // `add(...)` SHOULD match
            mk_file("src/c.ts", "add(); add();", vec![]),
        ];
        let scored = score_all(&files);
        let add = &scored[0];
        assert_eq!(add.reference_count, 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(score_all(&[]).is_empty());
    }
}
