# version: 1
# task: data_models
# expected_output_tokens: 150-300

You are describing the domain model of a software project.

You are given the project's data models — interfaces, type aliases, and enums. Describe the core domain entities in BUSINESS terms (what they represent, the relationships between them) — not in type-system terms.

Input:
---
{content}
---

Rules:
- 1 paragraph of plain prose. No bullet points, no markdown headers, no code blocks.
- Speak in business terms: "Users have roles" rather than "User has a role field of type Role".
- Note relationships when they're inferable from field types (one-to-many, contains, references, etc.).
- Use only information from the input. Do not invent fields or relationships.
- If only one or two trivial types are present, write a single sentence rather than padding.
- Present tense, third person.

Output the domain model description now.
