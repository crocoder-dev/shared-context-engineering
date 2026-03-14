# AGENTS.md

This file is for coding agents working in this repository.
It summarizes the commands, workflows, and code conventions that are visible in the current codebase.

This repository uses the Shared Context Engineering (SCE) approach for AI-assisted software delivery with explicit, versioned context: `https://sce.crocoder.dev/`

## Repository shape

- Root repo contains three main working areas:
- `cli/` - Rust CLI (`sce`)
- `evals/` - Bun + TypeScript eval harness
- `config/` - generated agent config, skills, and Pkl sources
- `context/` - shared context docs, plans, decisions, and handovers

## Rule files checked

- No root `AGENTS.md` existed before this file.
- No `.cursor/rules/` directory was found.
- No `.cursorrules` file was found.
- No `.github/copilot-instructions.md` file was found.
- If any of those files are added later, update this document to fold their instructions in.

## Tooling and environment

- Nix is the primary reproducible entrypoint at repo root.
- Root `flake.nix` provides Bun, TypeScript, Pkl, jq, and the Rust toolchain.
- `cli/flake.nix` defines Rust packaging and CI checks for the CLI.
- Run Cargo via Nix, not directly from the host shell. Prefer `nix develop -c sh -c 'cd cli && <cargo command>'`.
- Bun is used for the eval harness, not npm or pnpm scripts.
- Rust edition is `2021`.
- TypeScript runs in strict mode in `evals/tsconfig.json`.

## High-value commands

### Root-level setup

- Enter dev shell: `nix develop`
- Run all flake checks visible at root: `nix flake check`
- Run generated-output parity check: `nix run .#pkl-check-generated`
- Regenerate and sync `.opencode` config: `nix run .#sync-opencode-config`
- Run workflow token counter: `nix run .#token-count-workflows`

### Rust CLI commands

Run these through Nix from repo root unless noted otherwise.

- Build CLI: `nix develop -c sh -c 'cd cli && cargo build'`
- Run CLI: `nix develop -c sh -c 'cd cli && cargo run -- --help'`
- Run all CLI tests: `nix develop -c sh -c 'cd cli && cargo test'`
- Run a single test by exact name: `nix develop -c sh -c 'cd cli && cargo test parser_routes_mcp -- --exact'`
- Run tests in one module/file pattern: `nix develop -c sh -c 'cd cli && cargo test setup'`
- Run ignored? none were found; do not assume ignored-test flows exist.
- Format check: `nix develop -c sh -c 'cd cli && cargo fmt --check'`
- Auto-format: `nix develop -c sh -c 'cd cli && cargo fmt'`
- Lint: `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'`

### Bun eval commands

Run these from `evals/`.

- Run eval test suite: `bun test ./evals.test.ts`
- Run a single Bun test by name: `bun test ./evals.test.ts -t "runs context bootstrap flow with Shared Context"`
- Run token workflow script: `bun run token-count-workflows`

### Useful combined validation flows

- CLI validation from repo root: `nix develop -c sh -c 'cd cli && cargo fmt --check && cargo clippy --all-targets --all-features && cargo test'`
- Evals validation from repo root: `nix develop -c sh -c 'cd evals && bun test ./evals.test.ts'`
- Generated-config validation from repo root: `nix run .#pkl-check-generated`

## Testing notes

- Rust tests live inline in source files and in module test files such as `cli/src/services/setup/tests.rs`.
- Rust/Cargo commands should be executed through `nix develop`, even for one-off builds, tests, fmt, and clippy runs.
- Rust single-test selection uses standard Cargo substring matching; add `-- --exact` for deterministic one-test runs.
- Bun tests use `bun:test` and support `-t` name filtering.
- Evals create runtime artifacts under `evals/.results` and model-run directories; do not treat those as source files.
- Some eval tests depend on an OpenCode SDK server and model configuration; they are heavier than unit tests.

## CI and release hints

- GitHub Actions publish Tessl tiles from `config/.opencode/skills/**` and `config/.claude/skills/**`.
- Release workflow packages agent files from `config/.opencode/agent/Shared Context.md` and `config/.claude/agents/shared-context.md`.
- Changes under generated config trees may need a Pkl regeneration or parity check.

## Code style: general

- Follow existing local patterns before introducing new abstractions.
- Keep changes scoped and incremental.
- Prefer deterministic behavior and stable output text; this matters in CLI tests.
- Use explicit constants for repeated strings, timeouts, intervals, exit codes, and numeric formatting.
- Prefer small helper functions when they improve readability of branching or setup code.
- Avoid introducing framework-heavy patterns; this repo is mostly plain Rust, Bun, shell, and config assets.

## Code style: imports

### Rust imports

- Group imports in this order: standard library, third-party crates, then `crate::...` imports.
- Use grouped `std` imports such as `use std::path::{Path, PathBuf};`.
- Prefer explicit imported items over wildcard imports.
- Keep import lists stable and reasonably compact.

### TypeScript imports

- Use ESM `import` syntax only.
- Keep imports grouped: Node builtins, external packages, then local files.
- Use `type` imports inline where appropriate, for example `import { foo, type Bar } from "pkg";`.
- Use explicit relative file paths like `./test-setup`.

## Code style: formatting

- Rust formatting is delegated to `rustfmt`; do not hand-format against it.
- Rust uses 4-space indentation.
- TypeScript in `evals/` uses 2-space indentation, semicolons, trailing commas where multiline, and double-quoted strings.
- Shell scripts use `#!/usr/bin/env bash` and `set -euo pipefail`.
- Quote shell expansions unless you intentionally need word splitting.
- Prefer readable multi-line expressions over dense one-liners.

## Code style: types and data modeling

- In Rust, prefer strong enums and structs for command requests, runtime state, and result payloads.
- Derive common traits explicitly; common order in this repo is `Clone, Copy, Debug, Eq, PartialEq` when applicable.
- In TypeScript, prefer named `type` aliases for payloads and test result structures.
- Keep strict-mode friendliness: handle `undefined`, use narrow unions, and avoid implicit any.
- Prefer explicit return types on exported TypeScript helpers.
- Keep data structures serialization-friendly when they are written to JSON or surfaced by CLI output.

## Code style: naming

- Rust types and enums: `UpperCamelCase`.
- Rust functions, modules, and variables: `snake_case`.
- Rust constants: `SCREAMING_SNAKE_CASE`.
- TypeScript types: `PascalCase`.
- TypeScript variables and functions: `camelCase`.
- Test names are descriptive, behavior-oriented, and usually sentence-like with underscores in Rust.
- Prefer names that encode intent, not implementation trivia.

## Code style: error handling

- Rust uses `anyhow::Result` broadly for service-layer operations.
- Add context to I/O and process failures with `Context` / `with_context`.
- Use `bail!` and `anyhow!` for concise early exits when appropriate.
- Preserve user-facing diagnostics as stable strings when tests assert on them.
- Separate machine classification from rendered messages when the CLI contract cares about exit codes.
- In TypeScript, throw `Error` with direct, actionable messages.
- Convert unknown thrown values with helper functions like `getErrorMessage` before logging or persisting.

## Code style: CLI and output contracts

- Keep stdout reserved for intended command payloads.
- Keep errors on stderr and preserve stable prefixes/codes when existing code does so.
- Do not casually rewrite help text, error phrasing, or JSON field names; tests may depend on exact wording.
- Prefer deterministic ordering in rendered collections, embedded asset lists, and discovered file paths.

## Code style: tests

- Add unit tests close to the code they exercise.
- Match the repo's current pattern of focused behavioral test names.
- Assert on exact output when the CLI contract is supposed to be stable.
- For filesystem or manifest checks, sort collected paths before asserting.
- Keep tests isolated; clean up temporary state and abort long-running resources in teardown.

## Code style: shell and generated config workflows

- Shell scripts should fail fast, validate prerequisites early, and print concrete remediation steps.
- Prefer staging-and-swap workflows for generated config updates instead of in-place mutation.
- Treat `config/.opencode`, `config/.claude`, and repository-root `.opencode/` as sensitive generated trees.
- If you edit generated outputs manually, verify whether the corresponding Pkl source should be updated instead.

## Working safely as an agent

- Check for unrelated worktree changes before broad edits.
- Avoid destructive git commands unless the user explicitly asks for them.
- When touching both `config/` and `.opencode/`, verify whether sync/regeneration is expected.
- When changing CLI behavior, run at least the most relevant Rust test(s).
- When changing eval harness code, run the narrowest Bun test or script that covers the change.

## Recommended minimum verification by change type

- Rust-only logic change: `nix develop -c sh -c 'cd cli && cargo test <targeted-test> && cargo clippy --all-targets --all-features'`
- Rust output/help/error change: `nix develop -c sh -c 'cd cli && cargo test <exact-test-name> -- --exact'`
- TypeScript eval change: `cd evals && bun test ./evals.test.ts -t "<test name>"`
- Generated config or Pkl change: `nix run .#pkl-check-generated`
- Cross-cutting repo change: `nix flake check`

## File references worth checking

- `README.md`
- `flake.nix`
- `context/context-map.md`
- `context/overview.md`
- `cli/flake.nix`
- `cli/Cargo.toml`
- `cli/src/app.rs`
- `cli/src/services/setup/tests.rs`
- `evals/package.json`
- `evals/tsconfig.json`
- `evals/evals.test.ts`
- `evals/test-setup.ts`
- `scripts/sync-opencode-config.sh`
