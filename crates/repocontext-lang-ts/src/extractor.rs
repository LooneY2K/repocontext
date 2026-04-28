//! Tree-sitter-based extraction of TypeScript/TSX symbols.
//!
//! Captures top-level exported declarations (functions, classes, interfaces,
//! type aliases, enums, const/let/var bindings) plus class members (methods,
//! public fields). JSDoc-style block comments (`/** ... */`) immediately
//! preceding a symbol with only whitespace between are attached as
//! [`Symbol::doc_comment`].
//!
//! ## Limitations (documented; revisit in follow-ups)
//!
//! - Default exports (`export default ...`) are not captured. Most APIs use
//!   named exports; default exports can be added if user repos need them.
//! - Re-exports (`export { foo } from './bar'`) are not captured — they don't
//!   declare new symbols here.
//! - Interface members are not extracted as separate symbols; the interface's
//!   full source is captured as the signature.
//! - Visibility (public/private/protected) is not filtered. All methods/fields
//!   are extracted; the synthesizer can decide what to surface.

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use repocontext_core::symbol::{ExtractedSymbols, Symbol, SymbolKind};
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::Language;

const TS_QUERY: &str = include_str!("queries/typescript.scm");
const COMMENT_QUERY: &str = "(comment) @comment";

pub fn extract(source: &str, language: Language) -> Result<ExtractedSymbols> {
    let ts_lang: tree_sitter::Language = match language {
        Language::TypeScript => tree_sitter_typescript::language_typescript(),
        Language::Tsx => tree_sitter_typescript::language_tsx(),
    };

    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .context("setting tree-sitter typescript language")?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter parser returned None"))?;

    let root = tree.root_node();
    let had_parse_errors = root.has_error();

    let comments = collect_comments(&ts_lang, root, source)?;

    let query = Query::new(&ts_lang, TS_QUERY).context("compiling typescript symbol query")?;
    let capture_names = query.capture_names().to_vec();

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, root, source.as_bytes());

    let mut symbols = Vec::new();
    // Dedup key includes the symbol name so multi-declarator statements like
    // `export const a = 1, b = 2;` (both variable_declarators share the same
    // def_node) produce two distinct symbols rather than collapsing.
    let mut seen: HashSet<(usize, usize, String, SymbolKind)> = HashSet::new();

    for m in matches {
        let mut caps: HashMap<&str, Node> = HashMap::new();
        for c in m.captures {
            caps.insert(capture_names[c.index as usize], c.node);
        }

        if let Some(symbol) = build_symbol(&caps, source, &comments)? {
            let key = (
                symbol.start_byte,
                symbol.end_byte,
                symbol.name.clone(),
                symbol.kind,
            );
            if seen.insert(key) {
                symbols.push(symbol);
            }
        }
    }

    symbols.sort_by(|a, b| a.start_byte.cmp(&b.start_byte).then(a.name.cmp(&b.name)));

    Ok(ExtractedSymbols {
        symbols,
        had_parse_errors,
    })
}

/// Collect all `(comment)` nodes in source-order. Used for doc-comment lookup.
fn collect_comments(
    ts_lang: &tree_sitter::Language,
    root: Node,
    source: &str,
) -> Result<Vec<(usize, usize)>> {
    let query = Query::new(ts_lang, COMMENT_QUERY).context("compiling comment query")?;
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, root, source.as_bytes());
    let mut out = Vec::new();
    for m in matches {
        for c in m.captures {
            out.push((c.node.start_byte(), c.node.end_byte()));
        }
    }
    out.sort_by_key(|(s, _)| *s);
    Ok(out)
}

fn build_symbol(
    caps: &HashMap<&str, Node>,
    source: &str,
    comments: &[(usize, usize)],
) -> Result<Option<Symbol>> {
    if let Some(name_node) = caps.get("function.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("function.def").unwrap_or(name_node),
            caps.get("function.body").copied(),
            SymbolKind::Function,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("class.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("class.def").unwrap_or(name_node),
            caps.get("class.body").copied(),
            SymbolKind::Class,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("interface.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("interface.def").unwrap_or(name_node),
            None,
            SymbolKind::Interface,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("typealias.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("typealias.def").unwrap_or(name_node),
            None,
            SymbolKind::TypeAlias,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("enum.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("enum.def").unwrap_or(name_node),
            None,
            SymbolKind::Enum,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("const.name") {
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("const.def").unwrap_or(name_node),
            None,
            SymbolKind::Const,
            None,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("method.name") {
        let parent = caps
            .get("method.parent")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("method.def").unwrap_or(name_node),
            caps.get("method.body").copied(),
            SymbolKind::Method,
            parent,
            source,
            comments,
        )?));
    }
    if let Some(name_node) = caps.get("field.name") {
        let parent = caps
            .get("field.parent")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("field.def").unwrap_or(name_node),
            None,
            SymbolKind::Property,
            parent,
            source,
            comments,
        )?));
    }
    Ok(None)
}

fn make_symbol(
    name_node: &Node,
    def_node: Node,
    body_node: Option<Node>,
    kind: SymbolKind,
    parent: Option<String>,
    source: &str,
    comments: &[(usize, usize)],
) -> Result<Symbol> {
    let name = name_node
        .utf8_text(source.as_bytes())
        .context("extracting symbol name")?
        .to_string();

    let start_byte = def_node.start_byte();
    let end_byte = def_node.end_byte();
    let symbol_source = source[start_byte..end_byte].to_string();

    let signature = match body_node {
        Some(b) => source[start_byte..b.start_byte()]
            .trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string(),
        None => symbol_source.clone(),
    };

    let doc_comment = doc_comment_for(start_byte, comments, source);

    Ok(Symbol {
        name,
        kind,
        signature,
        doc_comment,
        source: symbol_source,
        start_byte,
        end_byte,
        start_line: def_node.start_position().row + 1,
        end_line: def_node.end_position().row + 1,
        parent,
    })
}

/// Returns the JSDoc-style block comment ending immediately before
/// `symbol_start` (with only whitespace between), or `None`.
fn doc_comment_for(
    symbol_start: usize,
    comments: &[(usize, usize)],
    source: &str,
) -> Option<String> {
    let pos = comments.partition_point(|(_, e)| *e <= symbol_start);
    if pos == 0 {
        return None;
    }
    let (cstart, cend) = comments[pos - 1];
    let gap = &source[cend..symbol_start];
    if !gap.chars().all(char::is_whitespace) {
        return None;
    }
    let text = &source[cstart..cend];
    if text.starts_with("/**") {
        Some(text.to_string())
    } else {
        None
    }
}
