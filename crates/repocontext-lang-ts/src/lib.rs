//! repocontext-lang-ts
//!
//! TypeScript (`.ts`) and TSX (`.tsx`) symbol extraction backed by
//! `tree-sitter-typescript`. Public API:
//!
//! - [`extract`] — parses a source string with the chosen [`Language`] and
//!   returns the extracted symbols.
//! - [`extract_file`] — convenience that picks the [`Language`] from the file
//!   extension (`.tsx` / `.jsx` → TSX, anything else → TypeScript).

mod extractor;

use std::path::Path;

use anyhow::Result;
use repocontext_core::symbol::ExtractedSymbols;

pub use extractor::extract;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    TypeScript,
    Tsx,
}

/// Extract symbols from `source`, picking the grammar from the file extension.
pub fn extract_file(source: &str, path: &Path) -> Result<ExtractedSymbols> {
    let lang = match path.extension().and_then(|e| e.to_str()) {
        Some("tsx") | Some("jsx") => Language::Tsx,
        _ => Language::TypeScript,
    };
    extract(source, lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repocontext_core::symbol::SymbolKind;

    #[test]
    fn extracts_exported_function() {
        let source = "\
export function add(a: number, b: number): number {
  return a + b;
}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        assert_eq!(extracted.symbols.len(), 1);
        let s = &extracted.symbols[0];
        assert_eq!(s.name, "add");
        assert_eq!(s.kind, SymbolKind::Function);
        assert!(s
            .signature
            .contains("export function add(a: number, b: number): number"));
        assert!(!s.signature.contains("return a + b"));
        assert!(s.source.contains("return a + b"));
        assert_eq!(s.start_line, 1);
        assert!(s.end_line >= 3);
        assert!(s.parent.is_none());
    }

    #[test]
    fn extracts_async_function() {
        let source = "\
export async function fetchUser(id: string): Promise<User> {
  return await db.users.findById(id);
}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        let s = &extracted.symbols[0];
        assert_eq!(s.name, "fetchUser");
        assert!(s.signature.contains("async"));
    }

    #[test]
    fn extracts_exported_class_with_methods() {
        let source = "\
export class Calculator {
  private total: number = 0;

  add(n: number): number {
    this.total += n;
    return this.total;
  }

  reset(): void {
    this.total = 0;
  }
}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        assert!(!extracted.had_parse_errors);

        let class_count = extracted
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .count();
        let method_count = extracted
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .count();
        assert_eq!(class_count, 1);
        assert_eq!(method_count, 2);

        let class_sym = extracted
            .symbols
            .iter()
            .find(|s| s.name == "Calculator")
            .unwrap();
        assert!(class_sym.signature.contains("export class Calculator"));
        assert!(!class_sym.signature.contains("this.total"));

        let add_method = extracted.symbols.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(add_method.parent.as_deref(), Some("Calculator"));
    }

    #[test]
    fn extracts_interface_type_enum() {
        let source = "\
export interface User { id: string; name: string; }
export type Role = \"admin\" | \"member\";
export enum Status { Active, Inactive }
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        let kinds: Vec<_> = extracted.symbols.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&SymbolKind::Interface));
        assert!(kinds.contains(&SymbolKind::TypeAlias));
        assert!(kinds.contains(&SymbolKind::Enum));

        let user = extracted.symbols.iter().find(|s| s.name == "User").unwrap();
        assert!(user.signature.contains("interface User"));
        assert!(user.signature.contains("id: string"));
    }

    #[test]
    fn extracts_const() {
        let source = "\
export const DEFAULT_TIMEOUT = 5000;
export const config = { foo: \"bar\" };
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        let consts: Vec<_> = extracted
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Const)
            .collect();
        assert_eq!(consts.len(), 2);
        let names: Vec<_> = consts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"DEFAULT_TIMEOUT"));
        assert!(names.contains(&"config"));
    }

    #[test]
    fn extracts_doc_comments() {
        let source = "\
/**
 * Adds two numbers.
 * @param a first
 * @param b second
 */
export function add(a: number, b: number): number {
  return a + b;
}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        let s = &extracted.symbols[0];
        assert_eq!(s.name, "add");
        let doc = s.doc_comment.as_ref().expect("doc comment present");
        assert!(doc.contains("Adds two numbers"));
        assert!(doc.starts_with("/**"));
        assert!(doc.ends_with("*/"));
    }

    #[test]
    fn line_comments_not_treated_as_doc() {
        let source = "\
// not a doc comment
export function foo() {}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        assert!(extracted.symbols[0].doc_comment.is_none());
    }

    #[test]
    fn block_comment_not_jsdoc_not_treated_as_doc() {
        let source = "\
/* regular block comment */
export function foo() {}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        assert!(extracted.symbols[0].doc_comment.is_none());
    }

    #[test]
    fn doc_comment_must_be_immediately_above() {
        let source = "\
/** This belongs to nothing. */

const _filler = 1;

export function foo() {}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        let foo = extracted.symbols.iter().find(|s| s.name == "foo").unwrap();
        assert!(foo.doc_comment.is_none());
    }

    #[test]
    fn parse_errors_flagged_but_extraction_continues() {
        let source = "\
export function fine() {}

export function broken( {
}
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        assert!(extracted.had_parse_errors);
        let fine = extracted.symbols.iter().find(|s| s.name == "fine");
        assert!(
            fine.is_some(),
            "should extract `fine` despite later parse error"
        );
    }

    #[test]
    fn tsx_parses_jsx_syntax() {
        let source = "\
export function Button({ label }: { label: string }) {
  return <button>{label}</button>;
}
";
        let extracted = extract(source, Language::Tsx).unwrap();
        assert!(!extracted.had_parse_errors);
        assert_eq!(extracted.symbols.len(), 1);
        assert_eq!(extracted.symbols[0].name, "Button");
    }

    #[test]
    fn extract_file_picks_tsx_for_tsx_extension() {
        let extracted = extract_file("export const x = <div />;", Path::new("foo.tsx")).unwrap();
        assert!(!extracted.had_parse_errors);
    }

    #[test]
    fn empty_source_no_symbols_no_errors() {
        let extracted = extract("", Language::TypeScript).unwrap();
        assert_eq!(extracted.symbols.len(), 0);
        assert!(!extracted.had_parse_errors);
    }

    #[test]
    fn deterministic_order() {
        let source = "\
export const c = 3;
export const a = 1;
export const b = 2;
";
        let extracted = extract(source, Language::TypeScript).unwrap();
        // sorted by source position, so insertion order is preserved
        let names: Vec<_> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["c", "a", "b"]);
    }
}
