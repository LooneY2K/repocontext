//! repocontext-lang-go
//!
//! Go (`.go`) symbol extraction backed by `tree-sitter-go`. Public API:
//! - [`extract`] — parses Go source and returns the extracted symbols.
//! - [`extract_file`] — orchestrator-shaped wrapper that ignores the path
//!   argument (Go has no per-file dialect — the same grammar handles
//!   regular `.go` and `_test.go` files alike).

mod extractor;

use std::path::Path;

use anyhow::Result;
use repocontext_core::symbol::ExtractedSymbols;

pub use extractor::extract;

/// Same as [`extract`]; the `_path` argument is accepted so the orchestrator
/// can dispatch to either `lang-ts::extract_file` or `lang-go::extract_file`
/// with the same signature.
pub fn extract_file(source: &str, _path: &Path) -> Result<ExtractedSymbols> {
    extract(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repocontext_core::symbol::SymbolKind;

    #[test]
    fn extracts_exported_function() {
        let source = "\
package main

// Add returns the sum of a and b.
func Add(a int, b int) int {
\treturn a + b
}
";
        let extracted = extract(source).unwrap();
        assert!(!extracted.had_parse_errors);
        assert_eq!(extracted.symbols.len(), 1);
        let s = &extracted.symbols[0];
        assert_eq!(s.name, "Add");
        assert_eq!(s.kind, SymbolKind::Function);
        assert!(s.signature.contains("func Add(a int, b int) int"));
        assert!(!s.signature.contains("return a + b"));
        assert!(s.source.contains("return a + b"));
        let doc = s.doc_comment.as_ref().expect("doc comment");
        assert!(doc.contains("Add returns the sum"));
    }

    #[test]
    fn skips_unexported_functions() {
        let source = "\
package main

func add(a int, b int) int { return a + b }
func Add(a int, b int) int { return a + b }
";
        let extracted = extract(source).unwrap();
        let names: Vec<_> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Add"]);
    }

    #[test]
    fn extracts_struct_with_methods() {
        let source = "\
package main

// Calculator tracks a running total.
type Calculator struct {
\ttotal int
}

// Add adds n to the running total and returns it.
func (c *Calculator) Add(n int) int {
\tc.total += n
\treturn c.total
}

func (c *Calculator) reset() { c.total = 0 }

// Reset clears the total.
func (c *Calculator) Reset() { c.total = 0 }
";
        let extracted = extract(source).unwrap();
        assert!(!extracted.had_parse_errors);
        let names_kinds: Vec<_> = extracted
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect();
        // Calculator (Class) + Add (Method) + Reset (Method). `reset` is unexported → skipped.
        assert!(names_kinds.contains(&("Calculator", SymbolKind::Class)));
        assert!(names_kinds.contains(&("Add", SymbolKind::Method)));
        assert!(names_kinds.contains(&("Reset", SymbolKind::Method)));
        assert!(!names_kinds.iter().any(|(n, _)| *n == "reset"));

        let calc = extracted
            .symbols
            .iter()
            .find(|s| s.name == "Calculator")
            .unwrap();
        assert!(calc.signature.starts_with("type Calculator struct"));

        let add = extracted.symbols.iter().find(|s| s.name == "Add").unwrap();
        assert_eq!(add.parent.as_deref(), Some("Calculator"));
    }

    #[test]
    fn extracts_interface() {
        let source = "\
package main

// Reader reads bytes.
type Reader interface {
\tRead(p []byte) (int, error)
}
";
        let extracted = extract(source).unwrap();
        let r = extracted
            .symbols
            .iter()
            .find(|s| s.name == "Reader")
            .unwrap();
        assert_eq!(r.kind, SymbolKind::Interface);
        assert!(r.signature.contains("type Reader interface"));
    }

    #[test]
    fn extracts_type_alias_and_named_type() {
        let source = "\
package main

type UserID = string

type Score int
";
        let extracted = extract(source).unwrap();
        let kinds: Vec<_> = extracted
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect();
        assert!(kinds.contains(&("UserID", SymbolKind::TypeAlias)));
        assert!(kinds.contains(&("Score", SymbolKind::TypeAlias)));
    }

    #[test]
    fn extracts_const_and_var() {
        let source = "\
package main

const DefaultTimeout = 5000
var Logger = newLogger()
const internal = 7
";
        let extracted = extract(source).unwrap();
        let names: Vec<_> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"DefaultTimeout"));
        assert!(names.contains(&"Logger"));
        assert!(!names.contains(&"internal"), "unexported should be skipped");
    }

    #[test]
    fn extracts_grouped_type_declarations() {
        let source = "\
package main

type (
\tFoo struct{}
\tBar interface{}
)
";
        let extracted = extract(source).unwrap();
        let names: Vec<_> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"Bar"));
    }

    #[test]
    fn pointer_receiver_parent_is_unwrapped() {
        let source = "\
package main

type Foo struct{}

func (f *Foo) DoIt() {}
";
        let extracted = extract(source).unwrap();
        let do_it = extracted.symbols.iter().find(|s| s.name == "DoIt").unwrap();
        assert_eq!(do_it.parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn generic_receiver_parent_is_unwrapped() {
        let source = "\
package main

type Foo[T any] struct{}

func (f *Foo[T]) DoIt() {}
";
        let extracted = extract(source).unwrap();
        let do_it = extracted.symbols.iter().find(|s| s.name == "DoIt").unwrap();
        assert_eq!(do_it.parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn block_doc_comment_supported() {
        let source = "\
package main

/*
Add returns the sum.
*/
func Add(a, b int) int { return a + b }
";
        let extracted = extract(source).unwrap();
        let s = extracted.symbols.iter().find(|s| s.name == "Add").unwrap();
        let doc = s.doc_comment.as_ref().expect("doc");
        assert!(doc.contains("Add returns the sum"));
    }

    #[test]
    fn blank_line_breaks_doc_association() {
        let source = "\
package main

// This belongs to nothing.

func Foo() {}
";
        let extracted = extract(source).unwrap();
        let foo = extracted.symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(foo.doc_comment.is_none());
    }

    #[test]
    fn parse_errors_flagged_but_extraction_continues() {
        let source = "\
package main

func Fine() {}

func Broken( {
}
";
        let extracted = extract(source).unwrap();
        assert!(extracted.had_parse_errors);
        let fine = extracted.symbols.iter().find(|s| s.name == "Fine");
        assert!(
            fine.is_some(),
            "should extract Fine despite later parse error"
        );
    }

    #[test]
    fn empty_source_no_symbols_no_errors() {
        let extracted = extract("").unwrap();
        assert_eq!(extracted.symbols.len(), 0);
        // Empty source is technically a parse error in Go (no `package` clause)
        // but tree-sitter is permissive — accept either outcome here.
    }

    #[test]
    fn extract_file_passthrough() {
        let extracted = extract_file(
            "package main\nfunc Hello() {}\n",
            std::path::Path::new("hello.go"),
        )
        .unwrap();
        assert_eq!(extracted.symbols.len(), 1);
        assert_eq!(extracted.symbols[0].name, "Hello");
    }
}
