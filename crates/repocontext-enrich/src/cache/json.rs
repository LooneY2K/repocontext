//! Flat-file JSON cache. Default backend.
//!
//! Layout on disk:
//!
//! ```json
//! {
//!   "version": 1,
//!   "entries": {
//!     "<sha256_hex>": {
//!       "chunk_type": "module",
//!       "section_name": "module:src/services",
//!       "input_preview": "...",
//!       "output": "...",
//!       "prompt_version": 1,
//!       "model_id": "qwen2.5-coder-7b-instruct-q4_k_m"
//!     }
//!   }
//! }
//! ```
//!
//! Entries are stored in a `BTreeMap` so the serialised JSON has alphabetical
//! key order — keeps diffs clean when teams commit the cache for CI.
//!
//! Writes are atomic: write to a sibling tempfile, fsync, rename. A crash
//! between `put` and `flush` loses uncommitted entries but never corrupts the
//! existing cache file.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::cache::EnrichCache;
use crate::types::CachedEntry;

/// Bumped only when the on-disk JSON layout changes incompatibly. The cache
/// key already incorporates the prompt version + model id, so prompt/model
/// bumps don't need a schema bump.
const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    /// `BTreeMap` for deterministic key ordering in the serialised JSON.
    entries: BTreeMap<String, CachedEntry>,
}

/// File-backed cache. Loads on construction, accumulates writes in memory,
/// commits to disk on [`flush`](EnrichCache::flush).
#[derive(Debug)]
pub struct JsonFileCache {
    path: PathBuf,
    entries: RwLock<BTreeMap<String, CachedEntry>>,
    dirty: AtomicBool,
}

impl JsonFileCache {
    /// Load the cache file at `path`. Missing file → empty cache. Schema
    /// version mismatch → actionable error so users know to delete the file.
    pub fn load(path: PathBuf) -> Result<Self> {
        let entries = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading cache file {}", path.display()))?;
            let file: CacheFile = serde_json::from_str(&text)
                .with_context(|| format!("parsing cache file {}", path.display()))?;
            if file.version != CACHE_SCHEMA_VERSION {
                bail!(
                    "cache file {} has schema version {} but this build expects {}. \
                     Delete the file (`rm {}`) or upgrade repocontext to a matching version.",
                    path.display(),
                    file.version,
                    CACHE_SCHEMA_VERSION,
                    path.display()
                );
            }
            file.entries
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path,
            entries: RwLock::new(entries),
            dirty: AtomicBool::new(false),
        })
    }

    /// Construct an in-memory-only cache (never persisted). Useful for tests
    /// and for the `--no-cache` CLI flag.
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            entries: RwLock::new(BTreeMap::new()),
            dirty: AtomicBool::new(false),
        }
    }

    /// Number of entries currently held (in memory + disk).
    pub fn len(&self) -> usize {
        self.entries.read().expect("cache lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl EnrichCache for JsonFileCache {
    fn get(&self, key: &str) -> Result<Option<CachedEntry>> {
        Ok(self
            .entries
            .read()
            .expect("cache lock poisoned")
            .get(key)
            .cloned())
    }

    fn put(&self, key: &str, entry: CachedEntry) -> Result<()> {
        self.entries
            .write()
            .expect("cache lock poisoned")
            .insert(key.to_string(), entry);
        self.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        // No path → in-memory cache. Drop the dirty flag, but don't write.
        if self.path.as_os_str().is_empty() {
            self.dirty.store(false, Ordering::SeqCst);
            return Ok(());
        }
        if !self.dirty.load(Ordering::SeqCst) {
            return Ok(());
        }

        let entries_snapshot = self.entries.read().expect("cache lock poisoned").clone();
        let file = CacheFile {
            version: CACHE_SCHEMA_VERSION,
            entries: entries_snapshot,
        };
        let json = serde_json::to_string_pretty(&file).context("serialising cache to JSON")?;

        write_atomic(&self.path, &json)
            .with_context(|| format!("writing cache to {}", self.path.display()))?;
        self.dirty.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Atomic file write: stream to a sibling tempfile, fsync it, rename over the
/// target. Guarantees readers either see the old contents or the new — never
/// a torn write.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating directory {}", parent.display()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(&parent)
        .with_context(|| format!("creating tempfile in {}", parent.display()))?;
    tmp.write_all(content.as_bytes())
        .context("writing cache content")?;
    tmp.as_file_mut().sync_all().context("fsync tempfile")?;
    tmp.persist(path)
        .map_err(|e| anyhow::anyhow!("renaming tempfile over {}: {}", path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChunkType;
    use tempfile::tempdir;

    fn mk_entry(section: &str, output: &str) -> CachedEntry {
        CachedEntry {
            chunk_type: ChunkType::Module,
            section_name: section.to_string(),
            input_preview: format!("preview of {section}"),
            output: output.to_string(),
            prompt_version: 1,
            model_id: "qwen2.5-coder-7b-instruct-q4_k_m".to_string(),
        }
    }

    #[test]
    fn fresh_cache_is_empty() {
        let dir = tempdir().unwrap();
        let cache = JsonFileCache::load(dir.path().join("cache.json")).unwrap();
        assert!(cache.is_empty());
        assert!(cache.get("any-key").unwrap().is_none());
    }

    #[test]
    fn put_then_get_round_trips() {
        let cache = JsonFileCache::in_memory();
        let entry = mk_entry("module:foo", "Foo handles X.");
        cache.put("abc123", entry.clone()).unwrap();
        let got = cache.get("abc123").unwrap().unwrap();
        assert_eq!(got, entry);
    }

    #[test]
    fn flush_writes_to_disk_and_load_reads_it_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subdir").join("cache.json");

        let cache = JsonFileCache::load(path.clone()).unwrap();
        cache.put("key1", mk_entry("a", "out a")).unwrap();
        cache.put("key2", mk_entry("b", "out b")).unwrap();
        cache.flush().unwrap();

        assert!(path.exists(), "cache file should exist after flush");

        // Reload from disk and verify
        let reloaded = JsonFileCache::load(path).unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.get("key1").unwrap().unwrap().output, "out a");
        assert_eq!(reloaded.get("key2").unwrap().unwrap().output, "out b");
    }

    #[test]
    fn flush_is_a_noop_when_clean() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.json");
        // First flush after some writes
        let cache = JsonFileCache::load(path.clone()).unwrap();
        cache.put("k", mk_entry("s", "o")).unwrap();
        cache.flush().unwrap();
        let mtime1 = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Second flush with no changes — should not rewrite
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.flush().unwrap();
        let mtime2 = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2, "clean flush should not rewrite the file");
    }

    #[test]
    fn json_output_keys_are_alphabetical() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = JsonFileCache::load(path.clone()).unwrap();
        // Insert in non-alphabetical order
        cache.put("zebra", mk_entry("z", "z")).unwrap();
        cache.put("apple", mk_entry("a", "a")).unwrap();
        cache.put("mango", mk_entry("m", "m")).unwrap();
        cache.flush().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let apple_pos = text.find("\"apple\"").unwrap();
        let mango_pos = text.find("\"mango\"").unwrap();
        let zebra_pos = text.find("\"zebra\"").unwrap();
        assert!(apple_pos < mango_pos);
        assert!(mango_pos < zebra_pos);
    }

    #[test]
    fn unsupported_schema_version_errors_actionably() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, r#"{"version": 999, "entries": {}}"#).unwrap();
        let err = JsonFileCache::load(path.clone()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema version 999"), "got: {msg}");
        assert!(msg.contains("Delete the file"), "got: {msg}");
    }

    #[test]
    fn corrupted_json_yields_actionable_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, "not valid json").unwrap();
        let err = JsonFileCache::load(path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("parsing cache file"), "got: {msg}");
    }

    #[test]
    fn in_memory_cache_does_not_write_to_disk_on_flush() {
        let cache = JsonFileCache::in_memory();
        cache.put("k", mk_entry("s", "o")).unwrap();
        cache.flush().unwrap();
        // No path → no file. Just confirm flush succeeded.
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn overwrite_replaces_entry() {
        let cache = JsonFileCache::in_memory();
        cache.put("k", mk_entry("s", "first")).unwrap();
        cache.put("k", mk_entry("s", "second")).unwrap();
        let got = cache.get("k").unwrap().unwrap();
        assert_eq!(got.output, "second");
    }

    #[test]
    fn cache_is_send_sync() {
        // Compile-time check: trait objects are Send + Sync.
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let cache: Box<dyn EnrichCache> = Box::new(JsonFileCache::in_memory());
        assert_send_sync(&cache);
    }
}
