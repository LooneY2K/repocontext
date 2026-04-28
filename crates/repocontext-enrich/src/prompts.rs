//! Embedded prompt templates — one per [`ChunkType`].
//!
//! Templates live as Markdown files in the workspace's `prompts/` directory
//! and are bundled into the binary via `include_str!`. Bumping a template's
//! `# version:` header invalidates all cached outputs for that task (the
//! version participates in the cache key).
//!
//! Iteration loop without firing real inference:
//! ```sh
//! repocontext generate --enrich --dry-run-llm
//! ```
//! dumps every rendered prompt to stdout. Paste a prompt into LM Studio /
//! Jan / koboldcpp to test it manually, edit the corresponding `.md` file,
//! bump `# version:`, re-run.

use anyhow::Result;

use crate::prompt::PromptTemplate;
use crate::types::ChunkType;

const OVERVIEW: &str = include_str!("../../../prompts/chunk_overview.md");
const ARCHITECTURE: &str = include_str!("../../../prompts/chunk_architecture.md");
const MODULE: &str = include_str!("../../../prompts/chunk_module.md");
const DATA_MODELS: &str = include_str!("../../../prompts/chunk_data_models.md");
const KEY_IMPL: &str = include_str!("../../../prompts/chunk_implementation.md");
const KEY_IMPL_ELIDED: &str = include_str!("../../../prompts/chunk_implementation_elided.md");

/// Return the template registered for a given chunk type. Errors only if a
/// template fails to parse — this is a build-time-ish failure (the templates
/// are baked into the binary), so it surfacing at runtime means a corrupt
/// build artifact.
pub fn template_for(chunk_type: ChunkType) -> Result<PromptTemplate> {
    let src = match chunk_type {
        ChunkType::Overview => OVERVIEW,
        ChunkType::Architecture => ARCHITECTURE,
        ChunkType::Module => MODULE,
        ChunkType::DataModels => DATA_MODELS,
        ChunkType::KeyImplementation => KEY_IMPL,
        ChunkType::KeyImplementationElided => KEY_IMPL_ELIDED,
    };
    PromptTemplate::parse(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chunk_type_has_a_loadable_template() {
        for kind in [
            ChunkType::Overview,
            ChunkType::Architecture,
            ChunkType::Module,
            ChunkType::DataModels,
            ChunkType::KeyImplementation,
            ChunkType::KeyImplementationElided,
        ] {
            let t = template_for(kind)
                .unwrap_or_else(|e| panic!("template for {kind:?} failed to parse: {e:#}"));
            assert!(
                t.version >= 1,
                "{kind:?} template has invalid version {}",
                t.version
            );
            assert!(!t.body.is_empty(), "{kind:?} template body is empty");
        }
    }

    #[test]
    fn module_template_renders_with_expected_placeholders() {
        let t = template_for(ChunkType::Module).unwrap();
        // Sanity: the template must reference the variables our orchestrator
        // will supply, otherwise rendering will leave `{var}` literally in the
        // prompt.
        assert!(
            t.body.contains("{section_name}"),
            "missing {{section_name}}"
        );
        assert!(
            t.body.contains("{cross_references}"),
            "missing {{cross_references}}"
        );
        assert!(t.body.contains("{content}"), "missing {{content}}");
    }

    #[test]
    fn templates_using_only_content_render_without_warnings() {
        for kind in [
            ChunkType::Overview,
            ChunkType::Architecture,
            ChunkType::DataModels,
            ChunkType::KeyImplementation,
            ChunkType::KeyImplementationElided,
        ] {
            let t = template_for(kind).unwrap();
            let rendered = t.render(&[("content", "<sample input>")]);
            assert!(
                !rendered.contains("{content}"),
                "{kind:?} template did not consume {{content}}: {rendered}"
            );
        }
    }
}
