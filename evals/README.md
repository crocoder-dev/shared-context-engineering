# Evals

## Workflow token counting

Run from the `evals/` directory:

```bash
bun install
bun run token-count-workflows
```

Common options:

```bash
bun run token-count-workflows --run-id local-test
bun run token-count-workflows --baseline context/tmp/token-footprint/workflow-token-count-latest.json
bun run token-count-workflows --tokenizer cl100k_base
bun run token-count-workflows --manifest context/sce/workflow-token-footprint-manifest.json
```

Output artifacts are written to `context/tmp/token-footprint/`:

- `workflow-token-count-latest.json`
- `workflow-token-count-latest.md`
- `workflow-token-count-<run_id>.json` (when `--run-id` is provided)
