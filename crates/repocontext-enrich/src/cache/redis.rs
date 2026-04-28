//! Redis-backed cache. Opt-in alternative to [`super::JsonFileCache`].
//!
//! Connection model: lazy. The `redis::Client` is built up-front (validates
//! the URL) but the actual TCP connection isn't opened until the first
//! `get`/`put` call. This way constructing the cache never fails just
//! because Redis isn't running yet — the error is surfaced when we actually
//! need to talk to it, with an actionable message.
//!
//! Storage: each entry is JSON-serialised and stored at the key
//! `{key_prefix}{sha256_hex}`. No TTL — invalidation is content-hash based,
//! same as `JsonFileCache`. To clear, run `redis-cli del repocontext:*`
//! or restart with the same prefix and let new content keys orphan the old.
//!
//! Concurrency: single connection guarded by a `Mutex`. `redis::Connection`
//! is `Send` but not `Sync`, and the whole cache must be `Sync` for
//! `EnrichCache`. We use `parking_lot`-style `Mutex<Option<Connection>>` so
//! only the first thread to use the cache pays the connect cost.

use std::sync::Mutex;

use anyhow::{Context, Result};
use redis::Commands;

use crate::cache::EnrichCache;
use crate::types::CachedEntry;

/// Redis-backed cache.
pub struct RedisCache {
    client: redis::Client,
    url: String,
    key_prefix: String,
    connection: Mutex<Option<redis::Connection>>,
}

impl RedisCache {
    /// Open a Redis client for `url`. Does NOT connect — the first `get`/`put`
    /// triggers the actual connection (and surfaces unreachability errors).
    pub fn new(url: impl Into<String>, key_prefix: impl Into<String>) -> Result<Self> {
        let url = url.into();
        let client = redis::Client::open(url.as_str())
            .with_context(|| format!("invalid Redis URL: {url}"))?;
        Ok(Self {
            client,
            url,
            key_prefix: key_prefix.into(),
            connection: Mutex::new(None),
        })
    }

    fn full_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }

    /// Borrow a connected `redis::Connection`, opening one lazily on first use.
    fn with_connection<R>(&self, f: impl FnOnce(&mut redis::Connection) -> Result<R>) -> Result<R> {
        let mut guard = self.connection.lock().expect("redis cache mutex poisoned");
        if guard.is_none() {
            let conn = self.client.get_connection().with_context(|| {
                format!(
                    "Redis unreachable at {}. Is the server running? \
                     Try: `brew services start redis` or `redis-server`.",
                    self.url
                )
            })?;
            *guard = Some(conn);
        }
        let conn = guard.as_mut().expect("connection just inserted");
        f(conn)
    }
}

impl std::fmt::Debug for RedisCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisCache")
            .field("url", &self.url)
            .field("key_prefix", &self.key_prefix)
            .finish_non_exhaustive()
    }
}

impl EnrichCache for RedisCache {
    fn get(&self, key: &str) -> Result<Option<CachedEntry>> {
        let full = self.full_key(key);
        self.with_connection(|conn| {
            let value: Option<String> = conn
                .get(&full)
                .with_context(|| format!("Redis GET {full}"))?;
            match value {
                None => Ok(None),
                Some(json) => {
                    let entry: CachedEntry = serde_json::from_str(&json).with_context(|| {
                        format!(
                            "parsing cached entry for {full} (delete via `redis-cli del {full}`)"
                        )
                    })?;
                    Ok(Some(entry))
                }
            }
        })
    }

    fn put(&self, key: &str, entry: CachedEntry) -> Result<()> {
        let full = self.full_key(key);
        let json = serde_json::to_string(&entry).context("serialising entry to JSON")?;
        self.with_connection(|conn| {
            let _: () = conn
                .set(&full, json)
                .with_context(|| format!("Redis SET {full}"))?;
            Ok(())
        })
    }

    fn flush(&self) -> Result<()> {
        // Every put is durable in Redis. Nothing to do here.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChunkType;

    fn mk_entry() -> CachedEntry {
        CachedEntry {
            chunk_type: ChunkType::Module,
            section_name: "module:foo".to_string(),
            input_preview: "preview".to_string(),
            output: "Foo handles X.".to_string(),
            prompt_version: 1,
            model_id: "qwen2.5-coder-7b-instruct-q4_k_m".to_string(),
        }
    }

    #[test]
    fn invalid_url_errors_actionably() {
        let err = RedisCache::new("not-a-url", "rc-test:").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Redis URL"), "got: {msg}");
    }

    #[test]
    fn full_key_uses_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", "rc-test:")
            .expect("URL parse should succeed even without a server");
        assert_eq!(cache.full_key("abc"), "rc-test:abc");
    }

    #[test]
    fn missing_server_yields_actionable_error_on_first_use() {
        // Use a port that is almost certainly unused. The `client.open` call
        // succeeds because no connection is opened yet; the first `get` triggers
        // a connection attempt which fails with our wrapped message.
        let cache = RedisCache::new("redis://127.0.0.1:1", "rc-test:").expect("URL parse");
        let err = cache.get("anything").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Redis unreachable"),
            "expected actionable error, got: {msg}"
        );
        assert!(
            msg.contains("redis-server") || msg.contains("brew"),
            "got: {msg}"
        );
    }

    /// Real-Redis round trip. Gated by `REPOCONTEXT_TEST_REDIS=1` env var so
    /// `cargo test` doesn't fail on machines without a running Redis.
    #[test]
    fn round_trip_via_real_redis() {
        if std::env::var("REPOCONTEXT_TEST_REDIS").is_err() {
            eprintln!("REPOCONTEXT_TEST_REDIS not set; skipping real-Redis test");
            return;
        }
        let url = std::env::var("REPOCONTEXT_TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://localhost:6379".to_string());
        // Unique prefix per test run avoids collisions with other test runs.
        let prefix = format!("rc-test-{}:", std::process::id());
        let cache = RedisCache::new(url, prefix.clone()).unwrap();

        let entry = mk_entry();
        cache.put("k1", entry.clone()).unwrap();
        let got = cache.get("k1").unwrap().expect("entry should be present");
        assert_eq!(got, entry);

        // Cleanup — best effort.
        let _ = cache.with_connection(|conn| {
            let _: i64 = conn.del(format!("{prefix}k1")).unwrap_or(0);
            Ok(())
        });
    }
}
