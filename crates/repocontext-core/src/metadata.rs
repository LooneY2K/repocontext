//! Project metadata extraction (package.json, README) for the Stage 1 synthesizer.
//!
//! These helpers are best-effort: missing or malformed metadata yields empty
//! values, never an error. The synthesizer renders sensible "(none discovered)"
//! placeholders so `context_temp.md` is always valid.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

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

/// Read `package.json` at `repo_root` and project metadata. Returns a
/// `ProjectMetadata` with sensible defaults if no `package.json` exists.
pub fn collect_metadata(repo_root: &Path) -> ProjectMetadata {
    let mut metadata = ProjectMetadata {
        primary_language: "TypeScript".to_string(),
        ..ProjectMetadata::default()
    };

    let pkg_path = repo_root.join("package.json");
    let Ok(pkg_text) = std::fs::read_to_string(&pkg_path) else {
        return metadata;
    };
    let Ok(pkg) = serde_json::from_str::<PackageJson>(&pkg_text) else {
        return metadata;
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

    // Top 10 alphabetically. Combining runtime deps + peer deps gives a
    // useful signal of what the project is built on.
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

    metadata
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
    fn missing_package_json_yields_default() {
        let dir = tempdir().unwrap();
        let m = collect_metadata(dir.path());
        assert!(m.name.is_none());
        assert!(m.entry_points.is_empty());
        assert!(m.key_dependencies.is_empty());
        assert_eq!(m.primary_language, "TypeScript");
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
