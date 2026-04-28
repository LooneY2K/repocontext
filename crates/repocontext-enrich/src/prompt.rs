//! Prompt template loader for Stage 2.
//!
//! Templates live in the workspace's `prompts/` directory and are bundled into
//! the binary via `include_str!`. Each has a small header followed by a body
//! with `{variable}` placeholders:
//!
//! ```text
//! # version: 1
//! # task: module_summary
//! # expected_output_tokens: 60-120     (optional, for documentation)
//!
//! You are documenting a software module...
//!
//! Module: {section_name}
//! Sibling modules: {cross_references}
//!
//! ---
//! {content}
//! ---
//! ```
//!
//! The `# version: N` header participates in the cache key — bumping it
//! invalidates all cached outputs for that task. This is how we iterate on
//! prompt wording without leaving stale cache entries lying around.

use anyhow::{bail, Context, Result};

/// A parsed prompt template. Render with [`PromptTemplate::render`] passing
/// `(name, value)` pairs for every `{name}` placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// Stable task name from the `# task:` header (e.g. `module_summary`).
    pub task: String,
    /// Version from the `# task:` header. Bumped on prompt edits.
    pub version: u32,
    /// Body text with `{variable}` placeholders. `render()` substitutes values.
    pub body: String,
}

impl PromptTemplate {
    /// Parse a template from its raw source. Headers are `# key: value` lines
    /// at the top; the body starts after the first blank line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut task: Option<String> = None;
        let mut version: Option<u32> = None;
        let mut body_lines: Vec<&str> = Vec::new();
        let mut in_body = false;

        for line in text.lines() {
            if in_body {
                body_lines.push(line);
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // Blank line separates header from body.
                in_body = true;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("# version:") {
                version = Some(
                    rest.trim()
                        .parse::<u32>()
                        .context("parsing `# version:` as u32")?,
                );
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("# task:") {
                task = Some(rest.trim().to_string());
                continue;
            }
            if trimmed.starts_with('#') {
                // Other `# key:` metadata — accepted but ignored (e.g.
                // `# expected_output_tokens:`, `# author:` etc.).
                continue;
            }
            // Non-header content without a separating blank line: assume
            // the header is implicit and body starts here.
            in_body = true;
            body_lines.push(line);
        }

        let task = task.context("prompt template missing `# task:` header")?;
        let version = version.context("prompt template missing `# version:` header")?;
        if body_lines.is_empty() {
            bail!("prompt template `{task}` v{version} has empty body");
        }

        // Trim leading/trailing blank lines from body for stable rendering.
        while body_lines.first().is_some_and(|l| l.trim().is_empty()) {
            body_lines.remove(0);
        }
        while body_lines.last().is_some_and(|l| l.trim().is_empty()) {
            body_lines.pop();
        }

        Ok(Self {
            task,
            version,
            body: body_lines.join("\n"),
        })
    }

    /// Substitute every `{name}` placeholder with the corresponding value.
    /// Unknown placeholders are left untouched (so they're visible in the
    /// rendered output, which surfaces template bugs early).
    pub fn render(&self, vars: &[(&str, &str)]) -> String {
        let mut out = self.body.clone();
        for (k, v) in vars {
            let placeholder = format!("{{{k}}}");
            out = out.replace(&placeholder, v);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_template() {
        let src = "\
# version: 2
# task: module_summary
# expected_output_tokens: 60-120

You are documenting a software module.

Module: {section_name}
{content}
";
        let t = PromptTemplate::parse(src).unwrap();
        assert_eq!(t.task, "module_summary");
        assert_eq!(t.version, 2);
        assert!(t.body.starts_with("You are documenting"));
        assert!(t.body.contains("{section_name}"));
        assert!(t.body.contains("{content}"));
    }

    #[test]
    fn renders_variables() {
        let src = "\
# version: 1
# task: t

Hello {name}, you are {role}.
";
        let t = PromptTemplate::parse(src).unwrap();
        let out = t.render(&[("name", "Alice"), ("role", "admin")]);
        assert_eq!(out, "Hello Alice, you are admin.");
    }

    #[test]
    fn unknown_placeholders_left_untouched() {
        let src = "\
# version: 1
# task: t

Hello {name}, you are {missing}.
";
        let t = PromptTemplate::parse(src).unwrap();
        let out = t.render(&[("name", "Alice")]);
        assert!(out.contains("{missing}"), "got: {out}");
    }

    #[test]
    fn missing_version_errors_actionably() {
        let src = "\
# task: t

body
";
        let err = PromptTemplate::parse(src).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("# version:"), "got: {msg}");
    }

    #[test]
    fn missing_task_errors_actionably() {
        let src = "\
# version: 1

body
";
        let err = PromptTemplate::parse(src).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("# task:"), "got: {msg}");
    }

    #[test]
    fn empty_body_errors() {
        let src = "\
# version: 1
# task: t

";
        let err = PromptTemplate::parse(src).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("empty body"), "got: {msg}");
    }

    #[test]
    fn unrecognised_header_ignored() {
        let src = "\
# version: 1
# task: t
# author: someone
# expected_output_tokens: 100

body content
";
        let t = PromptTemplate::parse(src).unwrap();
        assert_eq!(t.body, "body content");
    }

    #[test]
    fn render_replaces_all_occurrences_of_same_placeholder() {
        let src = "\
# version: 1
# task: t

{x} and {x} again
";
        let t = PromptTemplate::parse(src).unwrap();
        assert_eq!(t.render(&[("x", "Y")]), "Y and Y again");
    }
}
