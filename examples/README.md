# Examples

## `sample-context_temp.md`

The Stage 1 output of running `repocontext` against a small Go service. This is what `context_temp.md` looks like — the deterministic structural snapshot you can paste straight into Claude / ChatGPT / Cursor as context.

To produce the same shape of file for your own project:

```sh
cd /your/project
repocontext init
repocontext generate
# context_temp.md is written to the repo root
```

Stage 2 (`context.md`, the LLM-narrated version) is not committed here because the output is a few hundred KB and depends on your hardware/model — see the [main README](../README.md) for a hardware/speed table.
