//! Tree-sitter-based extraction of Go symbols.
//!
//! Captures top-level declarations (functions, methods, types, consts, vars)
//! and filters to only **exported** symbols (identifier starting with an
//! uppercase letter) — Go's universal visibility rule.
//!
//! ## Doc comments
//!
//! Go convention is consecutive `//` line comments immediately before the
//! declaration, with no blank line between. We walk back through preceding
//! `(comment)` nodes and concatenate them. Block comments (`/* ... */`) are
//! also surfaced when used as doc comments.
//!
//! ## Limitations (documented; revisit in follow-ups)
//!
//! - Grouped multi-name `const`/`var` declarations like `const A, B = 1, 2`
//!   surface only the first name. Rare in practice.
//! - Generic type parameters in receivers (`func (r *Foo[T]) Bar()`) are
//!   stripped from the parent name (so `Foo` is captured, not `Foo[T]`).
//! - Embedded fields and anonymous types in structs/interfaces are not
//!   surfaced as separate symbols; the type's full source is captured.

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use repocontext_core::symbol::{ExtractedSymbols, Symbol, SymbolKind};
use tree_sitter::{Node, Parser, Query, QueryCursor};

const GO_QUERY: &str = include_str!("queries/go.scm");
const COMMENT_QUERY: &str = "(comment) @comment";

pub fn extract(source: &str) -> Result<ExtractedSymbols> {
    let go_lang: tree_sitter::Language = tree_sitter_go::language();

    let mut parser = Parser::new();
    parser
        .set_language(&go_lang)
        .context("setting tree-sitter go language")?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter parser returned None"))?;

    let root = tree.root_node();
    let had_parse_errors = root.has_error();

    let comments = collect_comments(&go_lang, root, source)?;

    let query = Query::new(&go_lang, GO_QUERY).context("compiling go symbol query")?;
    let capture_names = query.capture_names().to_vec();

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, root, source.as_bytes());

    let mut symbols = Vec::new();
    let mut seen: HashSet<(usize, usize, String, SymbolKind)> = HashSet::new();

    for m in matches {
        let mut caps: HashMap<&str, Node> = HashMap::new();
        for c in m.captures {
            caps.insert(capture_names[c.index as usize], c.node);
        }

        if let Some(symbol) = build_symbol(&caps, source, &comments)? {
            if !is_exported(&symbol.name) {
                continue;
            }
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

    symbols.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then(a.name.cmp(&b.name))
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
    });

    Ok(ExtractedSymbols {
        symbols,
        had_parse_errors,
    })
}

fn is_exported(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

fn collect_comments(
    go_lang: &tree_sitter::Language,
    root: Node,
    source: &str,
) -> Result<Vec<(usize, usize)>> {
    let query = Query::new(go_lang, COMMENT_QUERY).context("compiling comment query")?;
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
            false,
        )?));
    }
    if let Some(name_node) = caps.get("method.name") {
        let parent = caps
            .get("method.receiver")
            .and_then(|n| extract_receiver_type(*n, source));
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("method.def").unwrap_or(name_node),
            caps.get("method.body").copied(),
            SymbolKind::Method,
            parent,
            source,
            comments,
            false,
        )?));
    }
    if let Some(name_node) = caps.get("type.name") {
        let kind_node = caps.get("type.kind").copied();
        let symbol_kind = match kind_node.map(|n| n.kind()) {
            Some("struct_type") => SymbolKind::Class,
            Some("interface_type") => SymbolKind::Interface,
            _ => SymbolKind::TypeAlias,
        };
        return Ok(Some(make_symbol(
            name_node,
            *caps.get("type.def").unwrap_or(name_node),
            None,
            symbol_kind,
            None,
            source,
            comments,
            true, // prepend "type " — type_spec doesn't include the keyword
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
            true,
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
            false,
        )?));
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn make_symbol(
    name_node: &Node,
    def_node: Node,
    body_node: Option<Node>,
    kind: SymbolKind,
    parent: Option<String>,
    source: &str,
    comments: &[(usize, usize)],
    prepend_type_keyword: bool,
) -> Result<Symbol> {
    let name = name_node
        .utf8_text(source.as_bytes())
        .context("extracting symbol name")?
        .to_string();

    let start_byte = def_node.start_byte();
    let end_byte = def_node.end_byte();
    let raw = &source[start_byte..end_byte];
    let symbol_source = if prepend_type_keyword {
        format!("type {raw}")
    } else {
        raw.to_string()
    };

    let signature = match body_node {
        Some(b) => source[start_byte..b.start_byte()].trim_end().to_string(),
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

/// Extract the receiver type name from a parameter list like `(r *Foo)` or
/// `(*Foo)` or `(r Foo[T])`. Strips pointer `*` and generic params.
fn extract_receiver_type(receiver_node: Node, source: &str) -> Option<String> {
    let text = receiver_node.utf8_text(source.as_bytes()).ok()?;
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    // Receiver can be `(r *Foo)`, `(*Foo)`, `(r Foo[T])`, `(Foo)` etc.
    // The type is always the last whitespace-separated word.
    let last_word = inner.split_whitespace().last()?;
    let no_pointer = last_word.trim_start_matches('*');
    let no_generic = no_pointer.split('[').next().unwrap_or(no_pointer);
    if no_generic.is_empty() {
        None
    } else {
        Some(no_generic.to_string())
    }
}

/// Walk back through consecutive line comments (separated only by single
/// newlines) ending immediately before `symbol_start`. Returns the
/// concatenated comment block, or `None` if no doc comment exists.
fn doc_comment_for(
    symbol_start: usize,
    comments: &[(usize, usize)],
    source: &str,
) -> Option<String> {
    let pos = comments.partition_point(|(_, e)| *e <= symbol_start);
    if pos == 0 {
        return None;
    }

    let (last_start, last_end) = comments[pos - 1];
    let gap = &source[last_end..symbol_start];
    if !gap.chars().all(char::is_whitespace) {
        return None;
    }
    if gap.matches('\n').count() > 1 {
        return None; // blank line breaks the association
    }
    let last_text = &source[last_start..last_end];

    // Walk back through more line comments (only `//`-style chains together).
    let mut block_start = last_start;
    if last_text.starts_with("//") {
        let mut i = pos - 1;
        while i > 0 {
            let (prev_start, prev_end) = comments[i - 1];
            let between = &source[prev_end..block_start];
            if !between.chars().all(char::is_whitespace) || between.matches('\n').count() > 1 {
                break;
            }
            let prev_text = &source[prev_start..prev_end];
            if !prev_text.starts_with("//") {
                break;
            }
            block_start = prev_start;
            i -= 1;
        }
    } else if !last_text.starts_with("/*") {
        return None;
    }

    Some(source[block_start..last_end].to_string())
}
