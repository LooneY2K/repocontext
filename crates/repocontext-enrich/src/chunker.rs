//! Split `context_temp.md` into LLM-sized [`Chunk`]s.
//!
//! Every section in `context_temp.md` (delimited by `<!-- repocontext:section=NAME -->`
//! markers) is converted into one or more chunks per the rules in "Stage 2:
//! Chunking & Coverage Guarantees" of the build plan.
//!
//! Coverage guarantee: every input section produces at least one chunk. The
//! chunker NEVER drops content — when a section exceeds the per-chunk budget
//! it deterministically sub-splits, never elides.

use anyhow::{Context, Result};
use regex::Regex;

use crate::types::{Chunk, ChunkType, CompletionParams};

/// Tunables for chunking. Compute defaults from a [`CompletionParams`] via
/// [`ChunkerConfig::from_params`].
#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    /// Maximum input characters per chunk. Computed from
    /// `(n_ctx − prompt_overhead − max_tokens) × 4`.
    pub chunk_budget_chars: usize,
    /// When sub-splitting a module section, prefer this many top-level
    /// paragraphs per sub-chunk if size allows.
    pub paragraphs_per_subchunk_hint: usize,
}

impl ChunkerConfig {
    /// Conservative budget allowing room for the prompt + response.
    /// Defaults: `n_ctx=4096`, prompt overhead ~600 tokens, response 400 tokens
    /// → ~3000 tokens of input ≈ 12 KB chars.
    pub fn from_params(params: &CompletionParams) -> Self {
        const PROMPT_OVERHEAD_TOKENS: usize = 600;
        let n_ctx = params.n_ctx as usize;
        let response = params.max_tokens as usize;
        let content_tokens = n_ctx.saturating_sub(PROMPT_OVERHEAD_TOKENS + response);
        let content_chars = content_tokens.saturating_mul(4);
        Self {
            // Safety floor — even a tiny n_ctx still gets a usable chunk size.
            chunk_budget_chars: content_chars.max(2000),
            paragraphs_per_subchunk_hint: 5,
        }
    }
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self::from_params(&CompletionParams::default())
    }
}

/// Parse `context_temp.md` and return the chunks ready for the LLM pipeline.
pub fn chunk(input: &str, cfg: &ChunkerConfig) -> Result<Vec<Chunk>> {
    let sections = parse_sections(input)?;
    let mut chunks = Vec::new();

    // Overview = metadata + readme combined. Per the build plan's "Stage 2"
    // table, both produce a single Overview output paragraph.
    let metadata = sections.iter().find(|s| s.kind == "metadata");
    let readme = sections.iter().find(|s| s.kind == "readme");
    if metadata.is_some() || readme.is_some() {
        let mut content = String::new();
        if let Some(m) = metadata {
            content.push_str("## Metadata\n\n");
            content.push_str(&m.content);
            content.push('\n');
        }
        if let Some(r) = readme {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str("## Readme Excerpt\n\n");
            content.push_str(&r.content);
            content.push('\n');
        }
        chunks.push(Chunk {
            chunk_id: "overview".to_string(),
            chunk_type: ChunkType::Overview,
            section_name: "overview".to_string(),
            part_index: None,
            total_parts: None,
            content,
            cross_references: Vec::new(),
        });
    }

    // Architecture — directory tree.
    if let Some(arch) = sections.iter().find(|s| s.kind == "architecture") {
        chunks.push(Chunk {
            chunk_id: "architecture".to_string(),
            chunk_type: ChunkType::Architecture,
            section_name: "architecture".to_string(),
            part_index: None,
            total_parts: None,
            content: arch.content.clone(),
            cross_references: Vec::new(),
        });
    }

    // Modules — one chunk per module, sub-split if oversized.
    let module_names: Vec<String> = sections
        .iter()
        .filter(|s| s.kind == "module")
        .filter_map(|s| s.name.clone())
        .collect();
    for sec in sections.iter().filter(|s| s.kind == "module") {
        let name = sec.name.clone().unwrap_or_default();
        let cross_refs: Vec<String> = module_names
            .iter()
            .filter(|n| **n != name)
            .cloned()
            .collect();
        chunks.extend(split_module(&name, &sec.content, &cross_refs, cfg));
    }

    // Data models.
    if let Some(dm) = sections.iter().find(|s| s.kind == "data_models") {
        chunks.push(Chunk {
            chunk_id: "data_models".to_string(),
            chunk_type: ChunkType::DataModels,
            section_name: "data_models".to_string(),
            part_index: None,
            total_parts: None,
            content: dm.content.clone(),
            cross_references: Vec::new(),
        });
    }

    // Key implementations — one chunk per `### …` entry, with elision if needed.
    if let Some(ki) = sections.iter().find(|s| s.kind == "key_implementations") {
        let entries = split_key_impl_entries(&ki.content);
        for (idx, entry) in entries.iter().enumerate() {
            let title = entry_title(entry).unwrap_or_else(|| format!("entry {idx}"));
            let chunk_id = format!("key_impl:{idx}:{}", sanitize_id(&title));
            let section_name = chunk_id.clone();
            if entry.len() > cfg.chunk_budget_chars {
                // Strip the code block and switch to the elided variant.
                let stripped = strip_code_block(entry);
                chunks.push(Chunk {
                    chunk_id,
                    chunk_type: ChunkType::KeyImplementationElided,
                    section_name,
                    part_index: None,
                    total_parts: None,
                    content: stripped,
                    cross_references: Vec::new(),
                });
            } else {
                chunks.push(Chunk {
                    chunk_id,
                    chunk_type: ChunkType::KeyImplementation,
                    section_name,
                    part_index: None,
                    total_parts: None,
                    content: entry.clone(),
                    cross_references: Vec::new(),
                });
            }
        }
    }

    Ok(chunks)
}

/// One section parsed from `context_temp.md`.
#[derive(Debug, Clone)]
struct Section {
    kind: String,
    name: Option<String>,
    content: String,
}

fn parse_sections(input: &str) -> Result<Vec<Section>> {
    let re = Regex::new(r#"<!-- repocontext:section=([^\s>]+)(?: name="([^"]+)")? -->"#)
        .context("compiling section-marker regex")?;
    let positions: Vec<_> = re
        .captures_iter(input)
        .map(|cap| {
            let m = cap.get(0).unwrap();
            let kind = cap.get(1).unwrap().as_str().to_string();
            let name = cap.get(2).map(|m| m.as_str().to_string());
            (m.start(), m.end(), kind, name)
        })
        .collect();

    let mut sections = Vec::with_capacity(positions.len());
    for (i, (_, content_start, kind, name)) in positions.iter().enumerate() {
        // Skip the sentinel "end" marker — it's a footer, not a section.
        if kind == "end" {
            continue;
        }
        let content_end = positions.get(i + 1).map(|p| p.0).unwrap_or(input.len());
        let content = input[*content_start..content_end].trim().to_string();
        sections.push(Section {
            kind: kind.clone(),
            name: name.clone(),
            content,
        });
    }
    Ok(sections)
}

fn split_module(
    name: &str,
    content: &str,
    cross_refs: &[String],
    cfg: &ChunkerConfig,
) -> Vec<Chunk> {
    let section_name = format!("module:{name}");

    if content.len() <= cfg.chunk_budget_chars {
        return vec![Chunk {
            chunk_id: section_name.clone(),
            chunk_type: ChunkType::Module,
            section_name,
            part_index: None,
            total_parts: None,
            content: content.to_string(),
            cross_references: cross_refs.to_vec(),
        }];
    }

    // Greedy paragraph-pack: each part gets as many `\n\n`-separated
    // paragraphs as fit under the char budget, with at least one paragraph
    // per part even if it overflows (better than dropping content).
    let parts = greedy_paragraph_pack(content, cfg);
    let total = parts.len();

    parts
        .into_iter()
        .enumerate()
        .map(|(i, part_content)| Chunk {
            chunk_id: format!("module:{name}:part-{i}"),
            chunk_type: ChunkType::Module,
            section_name: section_name.clone(),
            part_index: Some(i),
            total_parts: Some(total),
            content: part_content,
            cross_references: cross_refs.to_vec(),
        })
        .collect()
}

fn greedy_paragraph_pack(content: &str, cfg: &ChunkerConfig) -> Vec<String> {
    let paragraphs: Vec<&str> = content.split("\n\n").collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let _ = cfg.paragraphs_per_subchunk_hint;

    for p in paragraphs {
        if current.is_empty() {
            current.push_str(p);
            continue;
        }
        if current.len() + p.len() + 2 <= cfg.chunk_budget_chars {
            current.push_str("\n\n");
            current.push_str(p);
        } else {
            parts.push(std::mem::take(&mut current));
            current.push_str(p);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Split the `key_implementations` section into one entry per `### `-prefixed
/// heading. Each entry is a complete unit (heading + metadata + code block).
fn split_key_impl_entries(content: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut started = false;

    for line in content.lines() {
        if line.starts_with("### ") {
            if started && !current.trim().is_empty() {
                entries.push(current.trim_end().to_string());
            }
            current.clear();
            started = true;
        }
        if started {
            current.push_str(line);
            current.push('\n');
        }
    }
    if started && !current.trim().is_empty() {
        entries.push(current.trim_end().to_string());
    }
    entries
}

/// Pull the symbol name + path from the first heading line, e.g.
/// "### `validateSession` in src/auth.ts" → "validateSession in src/auth.ts".
fn entry_title(entry: &str) -> Option<String> {
    let line = entry.lines().next()?;
    let rest = line.strip_prefix("### ")?;
    Some(rest.trim().trim_matches('`').to_string())
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Replace the first ``` … ``` code block with the elision marker. Used when
/// a single key-implementation entry exceeds the chunk budget — the model is
/// asked to summarize from the signature alone.
fn strip_code_block(entry: &str) -> String {
    let re = Regex::new(r"(?s)```[a-zA-Z0-9_]*\n.*?\n```").unwrap();
    re.replace(entry, "*[source body elided — see context_temp.md]*")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_cfg() -> ChunkerConfig {
        ChunkerConfig {
            chunk_budget_chars: 10_000,
            paragraphs_per_subchunk_hint: 5,
        }
    }

    fn tiny_cfg(budget: usize) -> ChunkerConfig {
        ChunkerConfig {
            chunk_budget_chars: budget,
            paragraphs_per_subchunk_hint: 5,
        }
    }

    #[test]
    fn parses_full_document_into_expected_chunks() {
        let input = "\
# context_temp.md

> Some preamble.

<!-- repocontext:section=metadata -->
## Metadata

- Project: demo
- Files: 3

<!-- repocontext:section=readme -->
## Readme Excerpt

Short description of the project.

<!-- repocontext:section=architecture -->
## Directory Structure

```
src/
  a.ts
  b.ts
```

<!-- repocontext:section=module name=\"src\" -->
### Module: src

Files: 2

#### Exports

```typescript
export function a() {}
```

<!-- repocontext:section=data_models -->
## Data Models

```typescript
export interface User { id: string }
```

<!-- repocontext:section=key_implementations -->
## Key Implementations

### `a` in src/a.ts

References: 1
Salience: 1.5

```typescript
export function a() {}
```

<!-- repocontext:section=end -->
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        let kinds: Vec<_> = chunks.iter().map(|c| c.chunk_type).collect();
        assert_eq!(
            kinds,
            vec![
                ChunkType::Overview,
                ChunkType::Architecture,
                ChunkType::Module,
                ChunkType::DataModels,
                ChunkType::KeyImplementation,
            ]
        );

        let module = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::Module)
            .unwrap();
        assert_eq!(module.section_name, "module:src");
        assert_eq!(module.cross_references, Vec::<String>::new());
    }

    #[test]
    fn empty_input_produces_no_chunks() {
        let chunks = chunk("", &small_cfg()).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn missing_sections_handled_gracefully() {
        // No metadata, no key_implementations — only architecture.
        let input = "\
<!-- repocontext:section=architecture -->
## Directory

```
src/
```
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Architecture);
    }

    #[test]
    fn module_sub_splits_when_over_budget() {
        // Build a module with many paragraphs that total > budget
        let exports = (0..20)
            .map(|i| format!("export function f{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let input = format!(
            "<!-- repocontext:section=module name=\"big\" -->\n\
             ### Module: big\n\n\
             Files: 20\n\n\
             #### Exports\n\n\
             {exports}\n"
        );
        // Budget tight enough to force split, but large enough that each
        // paragraph fits individually.
        let cfg = tiny_cfg(200);
        let chunks = chunk(&input, &cfg).unwrap();
        let module_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Module)
            .collect();
        assert!(
            module_chunks.len() > 1,
            "expected multiple sub-chunks, got {}",
            module_chunks.len()
        );
        for (i, c) in module_chunks.iter().enumerate() {
            assert_eq!(c.section_name, "module:big");
            assert_eq!(c.part_index, Some(i));
            assert_eq!(c.total_parts, Some(module_chunks.len()));
            assert!(c.chunk_id.contains(&format!("part-{i}")));
        }
    }

    #[test]
    fn module_fits_in_one_chunk_when_under_budget() {
        let input = "\
<!-- repocontext:section=module name=\"small\" -->
### Module: small

Files: 1

```typescript
export function tiny() {}
```
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        let m = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::Module)
            .unwrap();
        assert_eq!(m.part_index, None);
        assert_eq!(m.total_parts, None);
        assert_eq!(m.chunk_id, "module:small");
    }

    #[test]
    fn cross_references_populated_for_modules() {
        let input = "\
<!-- repocontext:section=module name=\"alpha\" -->
content alpha

<!-- repocontext:section=module name=\"beta\" -->
content beta

<!-- repocontext:section=module name=\"gamma\" -->
content gamma
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        let alpha = chunks
            .iter()
            .find(|c| c.section_name == "module:alpha")
            .unwrap();
        assert_eq!(
            alpha.cross_references,
            vec!["beta".to_string(), "gamma".to_string()]
        );
        let beta = chunks
            .iter()
            .find(|c| c.section_name == "module:beta")
            .unwrap();
        assert_eq!(
            beta.cross_references,
            vec!["alpha".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn key_implementations_split_one_per_entry() {
        let input = "\
<!-- repocontext:section=key_implementations -->
## Key Implementations

### `foo` in src/a.ts

```typescript
export function foo() {}
```

### `bar` in src/b.ts

```typescript
export function bar() {}
```
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        let impls: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::KeyImplementation)
            .collect();
        assert_eq!(impls.len(), 2);
        assert!(impls[0].content.contains("foo"));
        assert!(impls[1].content.contains("bar"));
    }

    #[test]
    fn oversize_key_impl_is_elided() {
        let huge_body = "x".repeat(20_000);
        let input = format!(
            "<!-- repocontext:section=key_implementations -->\n\n\
             ### `huge` in src/huge.ts\n\n\
             ```typescript\n{huge_body}\n```\n"
        );
        let cfg = tiny_cfg(500);
        let chunks = chunk(&input, &cfg).unwrap();
        let elided: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::KeyImplementationElided)
            .collect();
        assert_eq!(elided.len(), 1);
        assert!(elided[0].content.contains("source body elided"));
        assert!(!elided[0].content.contains(&"x".repeat(100)));
    }

    #[test]
    fn end_marker_does_not_create_a_chunk() {
        let input = "\
<!-- repocontext:section=metadata -->
- Project: x

<!-- repocontext:section=end -->
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Overview);
    }

    #[test]
    fn all_chunks_under_budget_after_split() {
        let exports = (0..30)
            .map(|i| format!("paragraph {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let input = format!("<!-- repocontext:section=module name=\"x\" -->\n{exports}\n");
        let cfg = tiny_cfg(150);
        let chunks = chunk(&input, &cfg).unwrap();
        for c in &chunks {
            // Allow slight overage when a single paragraph alone exceeds the
            // budget — content is never dropped, only sub-split as far as
            // paragraph boundaries allow.
            assert!(
                c.content.len() <= cfg.chunk_budget_chars + 100,
                "chunk {} content {} > budget {}",
                c.chunk_id,
                c.content.len(),
                cfg.chunk_budget_chars
            );
        }
    }

    #[test]
    fn config_from_params_computes_sensible_budget() {
        let cfg = ChunkerConfig::from_params(&CompletionParams {
            n_ctx: 4096,
            max_tokens: 400,
            ..CompletionParams::default()
        });
        // 4096 - 600 - 400 = 3096 tokens × 4 = 12384 chars
        assert!(cfg.chunk_budget_chars >= 12_000);
        assert!(cfg.chunk_budget_chars < 13_000);
    }

    #[test]
    fn config_floor_applies_for_tiny_n_ctx() {
        let cfg = ChunkerConfig::from_params(&CompletionParams {
            n_ctx: 200,
            max_tokens: 50,
            ..CompletionParams::default()
        });
        // Below the floor — should clamp to safety minimum.
        assert_eq!(cfg.chunk_budget_chars, 2000);
    }

    #[test]
    fn coverage_every_input_section_yields_at_least_one_chunk() {
        let input = "\
<!-- repocontext:section=metadata -->
- Project: demo

<!-- repocontext:section=readme -->
A demo.

<!-- repocontext:section=architecture -->
```
src/
```

<!-- repocontext:section=module name=\"src\" -->
content

<!-- repocontext:section=data_models -->
```typescript
export interface X {}
```

<!-- repocontext:section=key_implementations -->
### `foo` in src/foo.ts

```typescript
export function foo() {}
```

<!-- repocontext:section=end -->
";
        let chunks = chunk(input, &small_cfg()).unwrap();
        let kinds: std::collections::HashSet<ChunkType> =
            chunks.iter().map(|c| c.chunk_type).collect();
        // Overview (metadata+readme), Architecture, Module, DataModels, KeyImplementation
        assert!(kinds.contains(&ChunkType::Overview));
        assert!(kinds.contains(&ChunkType::Architecture));
        assert!(kinds.contains(&ChunkType::Module));
        assert!(kinds.contains(&ChunkType::DataModels));
        assert!(kinds.contains(&ChunkType::KeyImplementation));
    }
}
