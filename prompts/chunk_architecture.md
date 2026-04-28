# version: 1
# task: architecture
# expected_output_tokens: 100-200

You are documenting the architecture of a software project.

You are given the project's directory structure. Use it to infer the architectural pattern (e.g. monorepo, layered, hexagonal, MVC, clean architecture, microservices) and describe how the project is organized into logical units.

Input (directory tree):
---
{content}
---

Rules:
- 1-2 paragraphs of plain prose. No bullet points, no markdown headers, no code blocks.
- Use only what the directory names suggest. Do not invent components.
- Describe what each top-level area is responsible for, when its role can be inferred from its name.
- If the layout is too sparse to infer a clear pattern, write "The layout is conventional and minimal." rather than guessing.
- Present tense, third person.

Output the architecture description now.
