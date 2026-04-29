# Security Policy

## Supported versions

Only the latest released version of `repocontext` receives security fixes.

| Version | Supported |
|---------|-----------|
| 0.2.x   | Yes       |
| < 0.2   | No        |

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

If you've found a security issue — particularly anything that could let a hostile `.repocontext.toml`, source file, or LLM response cause repocontext to:

- write outside the configured project root,
- exfiltrate environment variables or `.env*` files,
- execute attacker-controlled code,
- corrupt the model cache or trick the binary into running an unverified GGUF,

please email **sabyasachim356@gmail.com** with:

- a short description of the issue,
- the affected version (`repocontext --version`),
- a minimal reproduction (a config snippet or test repo is ideal),
- any suggested mitigation.

You'll get an acknowledgement within ~72 hours. We'll work with you on a fix and coordinate disclosure timing — typically a fix is merged within 14 days for high-severity issues, longer for low-severity.

We don't currently run a paid bounty program, but we'll credit you in the release notes (or keep your report anonymous if you prefer).

## Threat model

`repocontext` is a local-only CLI. It does **one** thing over the network: download the GGUF model from Hugging Face on first `--enrich` run. That download is HTTPS-pinned and SHA-256 verified against a hash baked into the binary; mismatches abort with an actionable error.

In scope:

- Path-traversal via `.repocontext.toml` (e.g. malicious `[output] temp_path = "../../etc/foo"`).
- Prompt injection via source files containing Qwen ChatML control tokens.
- Model integrity (MITM, mirror tampering, supply-chain).
- Walker DoS (unbounded file enumeration).

Out of scope:

- Anything that requires the user to have already given an attacker code execution on their machine. (At that point, repocontext isn't your weakest link.)
- The output of the LLM itself — the project produces Markdown, and it's the user's responsibility to review LLM-generated content before pasting it into another agent.
- Hugging Face's availability or trustworthiness as a model host. If you don't trust HF, supply your own GGUF via `[enrich.model] path_override`.
