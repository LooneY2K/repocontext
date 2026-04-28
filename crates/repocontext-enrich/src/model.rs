//! GGUF model download and cache-dir management.
//!
//! The default model is `qwen2.5-coder-7b-instruct` Q4_K_M (~4.5 GB) hosted
//! on Hugging Face. Resolution order for `cache_dir`:
//!
//! 1. `[enrich.model] cache_dir` from `.repocontext.toml`
//! 2. `dirs::cache_dir()` → `~/.cache/repocontext/models/` on macOS/Linux
//!
//! Downloads stream via `reqwest` blocking client with `indicatif` progress.
//! Partial downloads are saved to `<filename>.partial` and resumed on retry
//! via `Range:` headers — useful on slow links where a 4.5 GB pull may need
//! several attempts.
//!
//! SHA256 verification: if the descriptor's `sha256` is non-empty, the
//! downloaded file's hash is checked. Mismatch → actionable error pointing
//! to the file path so users can delete + retry.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use repocontext_core::config::ModelConfig;

/// One downloadable GGUF artifact. Combine with [`cache_dir`] to derive the
/// expected on-disk path via [`resolved_path`].
#[derive(Debug, Clone)]
pub struct ModelDescriptor {
    /// Logical name, e.g. `qwen2.5-coder-7b-instruct`.
    pub name: String,
    /// Quantization tag, e.g. `q4_k_m`. Combined with `name` to form the GGUF
    /// filename (`{name}-{quantization}.gguf`).
    pub quantization: String,
    /// HTTPS URL to fetch the file from.
    pub url: String,
    /// Expected SHA-256 (lowercase hex). Empty string disables verification —
    /// useful while we don't have a pinned hash for an exact upstream version.
    pub sha256: String,
    /// Approximate file size in bytes. Used as a fallback for the progress bar
    /// when the server doesn't return a `Content-Length` header.
    pub approx_size_bytes: u64,
}

impl ModelDescriptor {
    /// The default Qwen2.5-Coder 7B Instruct, Q4_K_M quantization, from the
    /// official Qwen Hugging Face repo.
    pub fn default_qwen() -> Self {
        Self {
            name: "qwen2.5-coder-7b-instruct".to_string(),
            quantization: "q4_k_m".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf".to_string(),
            // Empty == skip verification with a warning. Pin once we have
            // verified the upstream artifact byte-for-byte.
            sha256: String::new(),
            approx_size_bytes: 4_700_000_000,
        }
    }

    /// Build a descriptor from the project config's `[enrich.model]` block.
    /// Currently only Qwen2.5-Coder is wired in; non-default `name` values
    /// will use the configured name + quantization but inherit Qwen's URL,
    /// which won't actually serve a different model — that wiring lands once
    /// we add a real model registry.
    pub fn from_config(model_cfg: &ModelConfig) -> Self {
        let mut desc = Self::default_qwen();
        desc.name = model_cfg.name.clone();
        desc.quantization = model_cfg.quantization.clone();
        desc
    }

    pub fn filename(&self) -> String {
        format!("{}-{}.gguf", self.name, self.quantization)
    }
}

/// Resolve the cache directory for downloaded models. `override_dir` (from
/// `[enrich.model] cache_dir`) wins; otherwise we use the OS cache dir
/// (`dirs::cache_dir()`) joined with `repocontext/models`.
pub fn cache_dir(override_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_dir {
        return Ok(p.to_path_buf());
    }
    let base = dirs::cache_dir().ok_or_else(|| {
        anyhow!(
            "could not resolve OS cache dir; set [enrich.model] cache_dir explicitly in .repocontext.toml"
        )
    })?;
    Ok(base.join("repocontext").join("models"))
}

/// Where the file would live after a successful download.
pub fn resolved_path(descriptor: &ModelDescriptor, override_dir: Option<&Path>) -> Result<PathBuf> {
    Ok(cache_dir(override_dir)?.join(descriptor.filename()))
}

/// Returns true if the model file already exists on disk.
pub fn is_present(descriptor: &ModelDescriptor, override_dir: Option<&Path>) -> Result<bool> {
    Ok(resolved_path(descriptor, override_dir)?.exists())
}

/// Download (or resume) the GGUF model file. If the file is already present
/// at the resolved path, this is a no-op (with optional SHA256 verification).
///
/// `verify_sha256`: if the descriptor's `sha256` is non-empty, hash the local
/// file after download/on-resume completion and compare. Mismatch returns
/// an actionable error with the path to delete.
pub fn download(
    descriptor: &ModelDescriptor,
    override_dir: Option<&Path>,
    verify_sha256: bool,
) -> Result<PathBuf> {
    let path = resolved_path(descriptor, override_dir)?;
    if path.exists() {
        info!("Model already present at {}", path.display());
        if verify_sha256 {
            verify_file_sha256(&path, &descriptor.sha256)?;
        }
        return Ok(path);
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("model path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating model cache dir {}", parent.display()))?;

    let partial = path.with_extension("partial");
    let resume_from = if partial.exists() {
        partial.metadata().map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    info!(
        "Downloading {} from {}",
        descriptor.filename(),
        descriptor.url
    );
    if resume_from > 0 {
        info!(
            "Resuming from byte {} ({:.1} MB already on disk)",
            resume_from,
            resume_from as f64 / (1024.0 * 1024.0)
        );
    }

    let client = reqwest::blocking::Client::builder()
        // Some HF mirrors are slow on first byte; 60s connect timeout but no
        // total timeout (the file is huge).
        .connect_timeout(Duration::from_secs(60))
        .timeout(None)
        .build()
        .context("building HTTP client")?;

    let mut req = client.get(&descriptor.url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let response = req
        .send()
        .with_context(|| format!("sending GET {}", descriptor.url))?
        .error_for_status()
        .context("download response had non-2xx status")?;

    let total_remaining = response
        .content_length()
        .unwrap_or_else(|| descriptor.approx_size_bytes.saturating_sub(resume_from));
    let total = resume_from + total_remaining;

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner} [{elapsed_precise}] [{bar:30.cyan/blue}] {bytes:>10} / {total_bytes} ({bytes_per_sec}, ETA {eta})",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("##-"),
    );
    pb.set_position(resume_from);
    pb.set_message(descriptor.filename());

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(resume_from > 0)
        .write(true)
        .truncate(resume_from == 0)
        .open(&partial)
        .with_context(|| format!("opening partial file {}", partial.display()))?;

    let mut reader = response;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("reading from {}", descriptor.url))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("writing to {}", partial.display()))?;
        pb.inc(n as u64);
    }
    file.sync_all().context("fsync model partial file")?;
    drop(file);
    pb.finish_with_message(format!("{} downloaded", descriptor.filename()));

    std::fs::rename(&partial, &path)
        .with_context(|| format!("renaming {} → {}", partial.display(), path.display()))?;

    if verify_sha256 {
        verify_file_sha256(&path, &descriptor.sha256)?;
    }

    info!(
        "Model ready at {} ({:.2} GB)",
        path.display(),
        path.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1_000_000_000.0
    );

    Ok(path)
}

/// SHA-256 a file and compare to `expected`. Empty `expected` → log a
/// warning and skip (used while we don't have a pinned hash).
pub fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    if expected.is_empty() {
        warn!(
            "No expected SHA256 set for {}; skipping integrity check. \
             Pin a hash in ModelDescriptor::sha256 to enable verification.",
            path.display()
        );
        return Ok(());
    }
    let mut hasher = Sha256::new();
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        bail!(
            "SHA256 mismatch for {}: expected {}, got {}. \
             Delete the file (`rm {}`) and retry the download.",
            path.display(),
            expected,
            actual,
            path.display()
        );
    }
    info!("SHA256 verified for {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn descriptor_default_filename_includes_quantization() {
        let d = ModelDescriptor::default_qwen();
        assert_eq!(d.filename(), "qwen2.5-coder-7b-instruct-q4_k_m.gguf");
    }

    #[test]
    fn cache_dir_override_wins() {
        let dir = tempdir().unwrap();
        let resolved = cache_dir(Some(dir.path())).unwrap();
        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn cache_dir_default_under_repocontext_models() {
        // Don't pass an override; default should mention "repocontext/models".
        let resolved = cache_dir(None).unwrap();
        let s = resolved.to_string_lossy();
        assert!(s.contains("repocontext"), "got: {s}");
        assert!(s.ends_with("models"), "got: {s}");
    }

    #[test]
    fn resolved_path_is_cache_dir_plus_filename() {
        let dir = tempdir().unwrap();
        let d = ModelDescriptor::default_qwen();
        let p = resolved_path(&d, Some(dir.path())).unwrap();
        assert_eq!(p, dir.path().join(d.filename()));
    }

    #[test]
    fn is_present_false_until_file_created() {
        let dir = tempdir().unwrap();
        let d = ModelDescriptor::default_qwen();
        assert!(!is_present(&d, Some(dir.path())).unwrap());
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(dir.path().join(d.filename()), b"fake").unwrap();
        assert!(is_present(&d, Some(dir.path())).unwrap());
    }

    #[test]
    fn empty_sha256_skips_verification_with_warning() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("anything.gguf");
        std::fs::write(&p, b"some bytes").unwrap();
        // Empty expected → logs a warning and returns Ok.
        verify_file_sha256(&p, "").unwrap();
    }

    #[test]
    fn sha256_mismatch_returns_actionable_error() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("file.gguf");
        std::fs::write(&p, b"content").unwrap();
        // Expected hash is bogus on purpose.
        let err = verify_file_sha256(&p, "deadbeef").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("SHA256 mismatch"), "got: {msg}");
        assert!(msg.contains("Delete the file"), "got: {msg}");
    }

    #[test]
    fn sha256_match_returns_ok() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("file.gguf");
        std::fs::write(&p, b"hello").unwrap();
        // Pre-computed: sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        verify_file_sha256(
            &p,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        )
        .unwrap();
    }

    #[test]
    fn descriptor_from_config_inherits_url_from_default() {
        let model_cfg = ModelConfig {
            name: "custom-model".to_string(),
            quantization: "q5_k_m".to_string(),
            ..ModelConfig::default()
        };
        let d = ModelDescriptor::from_config(&model_cfg);
        assert_eq!(d.name, "custom-model");
        assert_eq!(d.quantization, "q5_k_m");
        // URL is inherited from default Qwen — note in docs that arbitrary
        // names are not yet wired through the registry.
        assert!(d.url.contains("Qwen"));
    }
}
