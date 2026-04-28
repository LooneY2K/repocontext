//! File discovery for repocontext.
//!
//! Walks the configured roots, applying these filters in order:
//!
//! 1. `ignore` crate's standard filters (`.gitignore`, `.ignore`, hidden-file rules).
//! 2. Hardcoded directory excludes for vendor/build dirs (`node_modules`, `dist`,
//!    `build`, `target`, `.git`, `vendor`, etc.). These apply regardless of config.
//! 3. User-configured glob excludes from `[exclude]` in `.repocontext.toml`.
//! 4. A hard cap on file size (`opts.max_file_bytes`, default 1 MiB).
//! 5. A binary-file sniff (null byte in first 8 KiB).
//!
//! Returns a deterministic, lexicographically-sorted list of source files. The
//! relative paths use forward slashes on every platform so downstream section
//! markers and sort keys are consistent.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::config::Config;

/// Directory names that are always excluded, regardless of config or `.gitignore`.
/// Match the spec's "Default excludes always apply" rule.
const HARDCODED_DIR_EXCLUDES: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    "target",
    ".git",
    "vendor",
    ".next",
    ".turbo",
    ".cache",
];

/// 1 MiB. Files larger than this are skipped entirely (likely generated, vendored,
/// or otherwise not useful for context).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;

const BINARY_SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct WalkOptions {
    pub exclude_globs: Vec<String>,
    pub max_file_bytes: u64,
    pub follow_symlinks: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            exclude_globs: Vec::new(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            follow_symlinks: false,
        }
    }
}

impl WalkOptions {
    /// Construct walk options from the parsed config. Convenience for the CLI.
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            exclude_globs: cfg.exclude.paths.clone(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            follow_symlinks: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Absolute filesystem path.
    pub absolute_path: PathBuf,
    /// Path relative to the walk root, with `/` separators on every platform.
    pub relative_path: PathBuf,
    /// Size in bytes from `metadata().len()`.
    pub size_bytes: u64,
}

/// Walk the given root directory, returning the sorted list of source files
/// that pass every filter.
pub fn walk(root: &Path, opts: &WalkOptions) -> Result<Vec<DiscoveredFile>> {
    let exclude_set = build_globset(&opts.exclude_globs)?;
    let mut walker_builder = WalkBuilder::new(root);
    walker_builder
        .standard_filters(true)
        // Don't walk up out of `root` looking for parent .gitignore files —
        // makes behaviour predictable in tests and embedded use cases.
        .parents(false)
        .follow_links(opts.follow_symlinks);

    let mut files = Vec::new();
    for entry in walker_builder.build() {
        let entry = entry.context("walking source tree")?;

        let path = entry.path();
        match entry.file_type() {
            Some(t) if t.is_file() => {}
            _ => continue,
        }

        // Hardcoded directory exclusions — match if any path component is on the list.
        if path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .map(|s| HARDCODED_DIR_EXCLUDES.contains(&s))
                .unwrap_or(false)
        }) {
            continue;
        }

        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_normalized = normalize_path(rel);

        if exclude_set.is_match(&rel_normalized) {
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("reading metadata for {}", path.display()))?;
        let size = metadata.len();
        if size > opts.max_file_bytes {
            continue;
        }

        if is_likely_binary(path) {
            continue;
        }

        files.push(DiscoveredFile {
            absolute_path: path.to_path_buf(),
            relative_path: rel_normalized,
            size_bytes: size,
        });
    }

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p).with_context(|| format!("invalid exclude glob: {p}"))?);
    }
    builder.build().context("building exclude glob set")
}

/// Replace platform-specific path separators with `/` so downstream output
/// (section markers, sort keys, snapshot tests) is platform-invariant.
fn normalize_path(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().replace('\\', "/"))
}

/// Returns true if the first 8 KiB of the file contains a null byte.
/// Files we can't open are treated as non-binary (let downstream surface the error).
fn is_likely_binary(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; BINARY_SNIFF_BYTES];
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(p: &Path, content: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn collect_names(files: &[DiscoveredFile]) -> Vec<String> {
        files
            .iter()
            .map(|f| f.relative_path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn finds_source_files_skips_hardcoded_dirs() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(&root.join("src/main.ts"), b"export const x = 1;\n");
        write(&root.join("src/util.ts"), b"export function f() {}\n");
        write(&root.join("node_modules/dep/index.js"), b"// dep\n");
        write(&root.join("dist/main.js"), b"// build\n");
        write(&root.join("target/release/blob"), b"// rust target\n");
        write(&root.join("vendor/lib/a.ts"), b"// vendor\n");

        let opts = WalkOptions::default();
        let files = walk(root, &opts).expect("walk");
        assert_eq!(
            collect_names(&files),
            vec!["src/main.ts".to_string(), "src/util.ts".to_string()]
        );
    }

    #[test]
    fn user_exclude_globs_apply() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(&root.join("src/main.ts"), b"// app\n");
        write(&root.join("src/main.test.ts"), b"// test\n");
        write(&root.join("src/main.spec.ts"), b"// spec\n");
        write(&root.join("src/auth.generated.ts"), b"// gen\n");

        let opts = WalkOptions {
            exclude_globs: vec![
                "**/*.test.ts".to_string(),
                "**/*.spec.ts".to_string(),
                "**/*.generated.*".to_string(),
            ],
            ..WalkOptions::default()
        };
        let files = walk(root, &opts).expect("walk");
        assert_eq!(collect_names(&files), vec!["src/main.ts".to_string()]);
    }

    #[test]
    fn skips_files_over_max_bytes() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(&root.join("small.ts"), b"// small\n");
        write(&root.join("big.ts"), &vec![b'x'; 2 * 1024 * 1024]);

        let opts = WalkOptions {
            max_file_bytes: 1024 * 1024,
            ..WalkOptions::default()
        };
        let files = walk(root, &opts).expect("walk");
        assert_eq!(collect_names(&files), vec!["small.ts".to_string()]);
    }

    #[test]
    fn skips_binary_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(&root.join("text.ts"), b"export const x = 1;\n");
        let mut binary = vec![1, 2, 3, 0, 4, 5, 6];
        binary.extend(std::iter::repeat_n(b'A', 100));
        write(&root.join("blob.ts"), &binary);

        let opts = WalkOptions::default();
        let files = walk(root, &opts).expect("walk");
        assert_eq!(collect_names(&files), vec!["text.ts".to_string()]);
    }

    #[test]
    fn deterministic_ordering() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        for name in ["zeta.ts", "alpha.ts", "middle.ts", "beta.ts"] {
            write(&root.join(name), b"// x\n");
        }
        let opts = WalkOptions::default();
        let files = walk(root, &opts).expect("walk");
        assert_eq!(
            collect_names(&files),
            vec![
                "alpha.ts".to_string(),
                "beta.ts".to_string(),
                "middle.ts".to_string(),
                "zeta.ts".to_string(),
            ]
        );
    }

    #[test]
    fn empty_dir_returns_empty_vec() {
        let dir = tempdir().unwrap();
        let opts = WalkOptions::default();
        let files = walk(dir.path(), &opts).expect("walk");
        assert!(files.is_empty());
    }

    #[test]
    fn invalid_glob_returns_actionable_error() {
        let dir = tempdir().unwrap();
        write(&dir.path().join("a.ts"), b"// x\n");
        let opts = WalkOptions {
            exclude_globs: vec!["[".to_string()],
            ..WalkOptions::default()
        };
        let err = walk(dir.path(), &opts).expect_err("invalid glob");
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid exclude glob"), "got: {msg}");
    }

    #[test]
    fn from_config_pulls_excludes() {
        let mut cfg = Config::default();
        cfg.exclude.paths = vec!["**/*.skip".to_string()];
        let opts = WalkOptions::from_config(&cfg);
        assert_eq!(opts.exclude_globs, vec!["**/*.skip".to_string()]);
        assert_eq!(opts.max_file_bytes, DEFAULT_MAX_FILE_BYTES);
    }
}
