# Shared Context Engineering

Shared Context Engineering (SCE) is a practical methodology for AI-assisted software delivery that keeps intent, constraints, and decisions explicit and versioned.

Instead of relying on one-off prompts, SCE treats shared project context as a first-class artifact (in a `context/` directory), so agents can produce code that stays aligned with your architecture and team standards.

This repository contains system prompts, agent configuration patterns, and evals you can use across tools.

## Placeholder CLI

The repository includes an early Rust placeholder CLI at `cli/`. Use
`cli/README.md` for current command behavior, usage, and implementation
boundaries.

- [Docs](https://sce.crocoder.dev/docs)
- [Getting Started](https://sce.crocoder.dev/docs/getting-started)
- [Motivation](https://sce.crocoder.dev/docs/motivation)

Built by [CroCoder](https://www.crocoder.dev/)

## Workflow token counting

Static workflow token-footprint reports are produced by the T06 script at
`evals/token-count-workflows.ts` using the canonical manifest
`context/sce/workflow-token-footprint-manifest.json`.

```bash
cd evals
bun run token-count-workflows
```

Optional inputs:

```bash
bun run token-count-workflows --run-id local-test
bun run token-count-workflows --baseline ../context/tmp/token-footprint/workflow-token-count-latest.json
```

Outputs are written to `context/tmp/token-footprint/` as:
- `workflow-token-count-latest.json`
- `workflow-token-count-latest.md`
- `workflow-token-count-<run_id>.json` (when `--run-id` is provided)

## Dev shell agnix tooling

This repository exposes `agnix` and `agnix-lsp` through `nix develop` using a Nix-first shell with Rust toolchain support.

### Quick start

```bash
nix develop
agnix --help
agnix-lsp --help
```

### Shell behavior

- On shell entry, `shellHook` adds `~/.cargo/bin` to `PATH`.
- If `agnix` is missing, `shellHook` automatically runs `cargo install --locked agnix-cli`.
- `agnix-lsp` is provided by a shim that resolves in this order:
  1. `AGNIX_LSP_BIN` (when set to an executable path)
  2. `~/.cargo/bin/agnix-lsp`
  3. A manual-install guidance message (non-zero exit)

### Manual fallback for agnix-lsp

```bash
cargo install --locked agnix-lsp
```

Optional explicit override:

```bash
export AGNIX_LSP_BIN="$HOME/.cargo/bin/agnix-lsp"
```

### Verification

```bash
nix flake check
nix develop -c which agnix
nix develop -c which agnix-lsp
nix develop -c agnix --help
nix develop -c agnix-lsp --help
```
