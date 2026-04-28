# version: 1
# task: overview
# expected_output_tokens: 150-250

You are writing the project overview for a software repository.

You are given the project's metadata and a short README excerpt. Use this to describe what the project DOES — its purpose, its main responsibilities, and any notable technical context (monorepo, web service, library, CLI, etc.).

Input:
---
{content}
---

Rules:
- Use only information from the input. Do not invent features, users, or claims.
- 2-3 paragraphs of plain prose. No bullet points, no markdown headers, no code blocks.
- Present tense, third person.
- Do not start with "This project..." — go directly to the description.
- If information is sparse, write a shorter overview rather than padding with speculation.

Output the overview now.
