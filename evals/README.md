# Evals

Evaluation scripts for workflow token counting.

---

## Count workflow tokens

Run from the `evals/` directory with bun install and bun run token-count-workflows.

Common options include --run-id, --baseline, --tokenizer, and --manifest. The baseline flag accepts a path to a previous token-count artifact.

Output artifacts are written to `context/tmp/token-footprint/`. Artifacts include `workflow-token-count-latest.json`, `workflow-token-count-latest.md`, and run-specific JSON files when --run-id is provided.
