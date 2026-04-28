//! Project metadata extraction (package.json, go.mod, README) for the Stage 1
//! synthesizer.
//!
//! These helpers are best-effort: missing or malformed metadata yields empty
//! values, never an error. The synthesizer renders sensible "(none discovered)"
//! placeholders so `context_temp.md` is always valid.
//!
//! Resolution order for project metadata: `package.json` → `go.mod` → empty
//! defaults. Mixed projects (both files present) currently surface the
//! `package.json` view; primary-language detection downstream re-overrides
//! based on what files were actually indexed.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::symbol::IndexedFile;
use crate::synth::ProjectMetadata;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PackageJson {
    name: Option<String>,
    main: Option<String>,
    module: Option<String>,
    bin: serde_json::Value,
    dependencies: BTreeMap<String, String>,
    #[serde(rename = "peerDependencies")]
    peer_dependencies: BTreeMap<String, String>,
}

/// Read project metadata from `package.json` (TypeScript) or `go.mod` (Go),
/// whichever is present. Returns sensible defaults if neither exists.
pub fn collect_metadata(repo_root: &Path) -> ProjectMetadata {
    if let Some(m) = collect_node_metadata(repo_root) {
        return m;
    }
    if let Some(m) = collect_go_metadata(repo_root) {
        return m;
    }
    ProjectMetadata::default()
}

fn collect_node_metadata(repo_root: &Path) -> Option<ProjectMetadata> {
    let pkg_path = repo_root.join("package.json");
    let pkg_text = std::fs::read_to_string(&pkg_path).ok()?;
    let pkg: PackageJson = serde_json::from_str(&pkg_text).ok()?;

    let mut metadata = ProjectMetadata {
        primary_language: "TypeScript".to_string(),
        ..ProjectMetadata::default()
    };
    metadata.name = pkg.name;

    if let Some(main) = pkg.main {
        metadata.entry_points.push(main);
    }
    if let Some(module) = pkg.module {
        metadata.entry_points.push(module);
    }
    if let Some(bin_obj) = pkg.bin.as_object() {
        for k in bin_obj.keys() {
            metadata.entry_points.push(format!("bin: {k}"));
        }
    } else if let Some(bin_str) = pkg.bin.as_str() {
        metadata.entry_points.push(format!("bin: {bin_str}"));
    }
    metadata.entry_points.sort();
    metadata.entry_points.dedup();

    let mut deps: Vec<String> = pkg
        .dependencies
        .keys()
        .chain(pkg.peer_dependencies.keys())
        .cloned()
        .collect();
    deps.sort();
    deps.dedup();
    deps.truncate(10);
    metadata.key_dependencies = deps;

    Some(metadata)
}

fn collect_go_metadata(repo_root: &Path) -> Option<ProjectMetadata> {
    let path = repo_root.join("go.mod");
    let text = std::fs::read_to_string(&path).ok()?;
    let (module_name, deps) = parse_go_mod(&text);
    let mut metadata = ProjectMetadata {
        primary_language: "Go".to_string(),
        ..ProjectMetadata::default()
    };
    metadata.name = module_name;
    let mut deps = deps;
    deps.sort();
    deps.dedup();
    deps.truncate(10);
    metadata.key_dependencies = deps;
    Some(metadata)
}

/// Parse a `go.mod` file. Returns `(module_name, direct_dependencies)`. Skips
/// `// indirect` deps so the dependency list reflects what the project
/// actually depends on, not its transitive closure.
fn parse_go_mod(text: &str) -> (Option<String>, Vec<String>) {
    let mut module_name: Option<String> = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_require_block = false;

    for raw_line in text.lines() {
        let raw_trimmed = raw_line.trim();
        let line = raw_trimmed.split("//").next().unwrap_or(raw_trimmed).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            module_name = Some(rest.trim().trim_matches('"').to_string());
        } else if line == "require (" {
            in_require_block = true;
        } else if in_require_block && line == ")" {
            in_require_block = false;
        } else if in_require_block {
            if raw_trimmed.contains("// indirect") {
                continue;
            }
            if let Some(dep) = line.split_whitespace().next() {
                deps.push(dep.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("require ") {
            if raw_trimmed.contains("// indirect") {
                continue;
            }
            if let Some(dep) = rest.split_whitespace().next() {
                deps.push(dep.to_string());
            }
        }
    }

    (module_name, deps)
}

/// Determine the project's primary language from the actually-indexed files.
/// Counts files by extension and picks a label. For mixed projects, surfaces
/// both languages with the majority first.
pub fn primary_language_from_files(files: &[IndexedFile]) -> String {
    let mut ts = 0usize;
    let mut go = 0usize;
    for f in files {
        match f.relative_path.extension().and_then(|e| e.to_str()) {
            Some("ts") | Some("tsx") | Some("mts") | Some("cts") => ts += 1,
            Some("go") => go += 1,
            _ => {}
        }
    }
    match (ts, go) {
        (0, 0) => "Unknown".to_string(),
        (_, 0) => "TypeScript".to_string(),
        (0, _) => "Go".to_string(),
        (t, g) if t >= g => "TypeScript, Go".to_string(),
        _ => "Go, TypeScript".to_string(),
    }
}

/// Find the README file (case-insensitive) and return the first paragraph
/// after the H1 title, or `None` if no README/no paragraph found.
pub fn read_readme_excerpt(repo_root: &Path) -> Option<String> {
    let candidates = ["README.md", "Readme.md", "readme.md", "README.MD"];
    for name in &candidates {
        if let Ok(content) = std::fs::read_to_string(repo_root.join(name)) {
            if let Some(excerpt) = extract_first_paragraph_after_h1(&content) {
                return Some(excerpt);
            }
        }
    }
    None
}

fn extract_first_paragraph_after_h1(s: &str) -> Option<String> {
    let mut after_h1 = false;
    let mut paragraph: Vec<&str> = Vec::new();
    for line in s.lines() {
        if !after_h1 {
            if line.starts_with("# ") || line == "#" {
                after_h1 = true;
            }
            continue;
        }
        if line.trim().is_empty() {
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        if line.starts_with('#') {
            break;
        }
        paragraph.push(line);
    }
    if paragraph.is_empty() {
        None
    } else {
        Some(paragraph.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_manifests_yields_default() {
        // Neither package.json nor go.mod present.
        let dir = tempdir().unwrap();
        let m = collect_metadata(dir.path());
        assert!(m.name.is_none());
        assert!(m.entry_points.is_empty());
        assert!(m.key_dependencies.is_empty());
        // primary_language is left empty here; the orchestrator overrides it
        // with primary_language_from_files() after indexing.
        assert_eq!(m.primary_language, "");
    }

    #[test]
    fn parses_go_mod() {
        let dir = tempdir().unwrap();
        let go_mod = "\
module github.com/foo/bar

go 1.21

require (
\tgithub.com/spf13/cobra v1.7.0
\tgithub.com/stretchr/testify v1.8.0 // indirect
\tgo.uber.org/zap v1.24.0
)

require github.com/sirupsen/logrus v1.9.0
";
        std::fs::write(dir.path().join("go.mod"), go_mod).unwrap();
        let m = collect_metadata(dir.path());
        assert_eq!(m.name.as_deref(), Some("github.com/foo/bar"));
        assert_eq!(m.primary_language, "Go");
        // testify is // indirect → excluded
        assert!(m
            .key_dependencies
            .contains(&"github.com/spf13/cobra".to_string()));
        assert!(m.key_dependencies.contains(&"go.uber.org/zap".to_string()));
        assert!(m
            .key_dependencies
            .contains(&"github.com/sirupsen/logrus".to_string()));
        assert!(!m.key_dependencies.iter().any(|d| d.contains("testify")));
    }

    #[test]
    fn package_json_takes_precedence_over_go_mod() {
        // If both manifests exist, package.json wins. Mixed projects have
        // their primary_language re-derived from indexed files downstream.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name": "ts-project"}"#).unwrap();
        std::fs::write(dir.path().join("go.mod"), "module github.com/foo/bar\n").unwrap();
        let m = collect_metadata(dir.path());
        assert_eq!(m.name.as_deref(), Some("ts-project"));
        assert_eq!(m.primary_language, "TypeScript");
    }

    #[test]
    fn primary_language_from_files_majority() {
        use crate::symbol::ExtractedSymbols;
        let mk = |p: &str| crate::symbol::IndexedFile {
            relative_path: std::path::PathBuf::from(p),
            source: String::new(),
            extracted: ExtractedSymbols::default(),
        };
        assert_eq!(primary_language_from_files(&[]), "Unknown");
        assert_eq!(
            primary_language_from_files(&[mk("a.ts"), mk("b.tsx")]),
            "TypeScript"
        );
        assert_eq!(primary_language_from_files(&[mk("a.go"), mk("b.go")]), "Go");
        assert_eq!(
            primary_language_from_files(&[mk("a.ts"), mk("b.go"), mk("c.go")]),
            "Go, TypeScript"
        );
        assert_eq!(
            primary_language_from_files(&[mk("a.ts"), mk("b.ts"), mk("c.go")]),
            "TypeScript, Go"
        );
    }

    #[test]
    fn parses_package_json() {
        let dir = tempdir().unwrap();
        let pkg = r#"{
            "name": "my-app",
            "main": "dist/index.js",
            "module": "dist/index.mjs",
            "dependencies": {
                "react": "^18.0.0",
                "express": "^4.18.0",
                "zod": "^3.0.0"
            },
            "peerDependencies": {
                "typescript": "^5.0.0"
            }
        }"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        let m = collect_metadata(dir.path());
        assert_eq!(m.name.as_deref(), Some("my-app"));
        assert!(m.entry_points.contains(&"dist/index.js".to_string()));
        assert!(m.entry_points.contains(&"dist/index.mjs".to_string()));
        // alphabetical order
        assert_eq!(
            m.key_dependencies,
            vec![
                "express".to_string(),
                "react".to_string(),
                "typescript".to_string(),
                "zod".to_string()
            ]
        );
    }

    #[test]
    fn malformed_package_json_yields_default() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "not json").unwrap();
        let m = collect_metadata(dir.path());
        assert!(m.name.is_none());
    }

    #[test]
    fn readme_first_paragraph_after_h1() {
        let dir = tempdir().unwrap();
        let readme = "\
# My Project

This is the first paragraph.
It spans multiple lines.

This is a second paragraph.

## Section
";
        std::fs::write(dir.path().join("README.md"), readme).unwrap();
        let excerpt = read_readme_excerpt(dir.path()).unwrap();
        assert_eq!(
            excerpt,
            "This is the first paragraph.\nIt spans multiple lines."
        );
    }

    #[test]
    fn missing_readme_returns_none() {
        let dir = tempdir().unwrap();
        assert!(read_readme_excerpt(dir.path()).is_none());
    }

    #[test]
    fn readme_without_h1_returns_none() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "Just some text.\nNo header.\n",
        )
        .unwrap();
        assert!(read_readme_excerpt(dir.path()).is_none());
    }

    #[test]
    fn case_insensitive_readme_lookup() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Title\n\nBody.\n").unwrap();
        assert_eq!(read_readme_excerpt(dir.path()).as_deref(), Some("Body."));
    }
}
