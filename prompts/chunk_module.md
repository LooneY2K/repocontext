# version: 1
# task: module_summary
# expected_output_tokens: 60-120

You are documenting one module of a software project. Your job is to describe the BUSINESS PURPOSE of this module — what problem it solves for the application, why it exists, what it is responsible for.

Module: {section_name}
Sibling modules in this codebase: {cross_references}

Module content (signatures and doc comments from this module's source files):
---
{content}
---

Rules:
- 2-4 sentences of plain prose.
- Describe purpose and behaviour, NOT structure. The reader can already see the structure (signatures, file count) separately.
- Use only information present in the input. If purpose is unclear from the signatures and doc comments alone, write "Purpose unclear from structure alone."
- Present tense, third person.
- No code blocks, no bullet points, no markdown headers.
- Do not start with "This module..." — go directly to the description.

Output the module description now.
