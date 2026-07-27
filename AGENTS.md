# AGENTS.md

This file is for coding agents working in this repository.
It summarizes the commands, workflows, and code conventions that are visible in the current codebase.

This repository uses the Shared Context Engineering (SCE) approach for AI-assisted software delivery with explicit, versioned context: `https://sce.crocoder.dev/`

## Repository shape

- Root repo contains three main working areas:
- `cli/` - Rust CLI (`sce`)
- `config/` - generated agent config, skills, and Pkl sources
- `context/` - shared context docs, plans, decisions, and handovers

## Rule files checked

- Root agent guidance lives in this `AGENTS.md`.
- Bash command policies live in `.sce/config.json` under `policies.bash.custom` (enforced by the SCE bash-policy plugin).
- No `.cursor/rules/` directory was found.
- No `.cursorrules` file was found.
- No `.github/copilot-instructions.md` file was found.
- If any of those files are added later, update this document to fold their instructions in.

## How to run commands (agents)

**Default rule:** host `coreutils` / POSIX basics are fine as-is. **Everything else must go through Nix** — do not assume `rg`, `jq`, `bun`, `python3`, `node`, `cargo`, etc. are on the host `PATH`.

### Allowed without Nix

Use the host shell for ordinary shell built-ins and common coreutils-style tools, for example:

- shell: `cd`, `export`, `true`, `false`, pipelines, redirections
- coreutils-ish: `ls`, `cat`, `cp`, `mv`, `rm`, `mkdir`, `chmod`, `ln`, `head`, `tail`, `sort`, `uniq`, `wc`, `cut`, `tr`, `tee`, `echo`, `printf`, `test`, `[`, `basename`, `dirname`, `realpath`, `pwd`, `env`, `xargs`, `find` (when already available)
- git (repo workflows), and `nix` itself

### Non-coreutils → Nix

For any other CLI (`rg`, `fd`, `jq`, `yq`, `bun`, `node`, `python3`, `shellcheck`, `actionlint`, `hyperfine`, language toolchains, etc.), run it via one of:

| Pattern | When to use | Example |
| --- | --- | --- |
| `nix develop -c …` | Repo flake tools (Rust toolchain, Bun, Pkl from this project’s dev shell) | `nix develop -c sh -c 'cd cli && cargo build'` |
| `nix shell nixpkgs#pkg -c …` | One-shot tool from nixpkgs (preferred for policy-satisfied ad-hoc tools) | `nix shell nixpkgs#ripgrep -c rg pattern path` |
| `nix run nixpkgs#pkg -- …` | Alternate one-shot form | `nix run nixpkgs#jq -- . file.json` |
| `nix run .#attr -- …` | Flake apps / checks defined in this repo | `nix run .#pkl-check-generated` |
| `nix flake check` | Default full verification | Prefer this over raw `cargo test` / `cargo check` / `cargo fmt --check` |

Repo bash policy (`.sce/config.json`) already blocks bare invocations of several tools and steers agents to Nix. Known policy-covered tools:

- **Cargo verification:** prefer `nix flake check` over bare `cargo test`, `cargo check`, and `cargo fmt --check`. Keep `cargo fmt` (autofix) only through `nix develop`.
- **Ad-hoc tools via Nix:** `rg` → `nixpkgs#ripgrep`, `jq` → `nixpkgs#jq`, `python3` → `nixpkgs#python3`, `node` → `nixpkgs#nodejs`, `bun` → `nixpkgs#bun` (or repo `nix develop`), `fd` → `nixpkgs#fd`, `shellcheck` → `nixpkgs#shellcheck`, `actionlint` → `nixpkgs#actionlint`, `yq` → `nixpkgs#yq-go`, `hyperfine` → `nixpkgs#hyperfine`.

If a tool is not listed above but is not coreutils, still run it through `nix shell` / `nix run` / `nix develop` the same way. Do not install host packages to work around missing binaries.

### Choosing the right Nix entrypoint

1. **Repo work (Rust CLI, Bun plugin, flake tools):** `nix develop -c sh -c '…'` from repo root.
2. **Single external utility:** `nix shell nixpkgs#<pkg> -c <cmd> …` or `nix run nixpkgs#<pkg> -- …`.
3. **Validation / CI parity:** `nix flake check` (and `nix run .#pkl-check-generated` when touching generated config).

## Tooling and environment

- Nix is the primary reproducible entrypoint at repo root.
- Root `flake.nix` provides Bun, TypeScript, Pkl, jq, and the Rust toolchain.
- Root `flake.nix` defines Crane-based Rust packaging and check derivations for the CLI.
- Run Cargo via Nix, not directly from the host shell. Prefer `nix develop -c sh -c 'cd cli && <cargo command>'`.
- For validation, prefer `nix flake check` and avoid running `cargo test` / `cargo check` / `cargo fmt --check` directly unless a user explicitly requests it.
- Optional local Nix tuning can live in user-level `~/.config/nix/nix.conf`; recommended values are `max-jobs = auto` and `cores = 0`.
- `auto-optimise-store = true` is intentionally treated as a system-level `/etc/nix/nix.conf` setting, not a repo-managed user setting.
- Bun is used for repo-owned config/plugin workflows; prefer Bun rather than npm or pnpm scripts when working in those areas. Always invoke Bun through Nix (`nix develop` or `nix shell nixpkgs#bun`).
- Rust edition is `2021`.
- TypeScript is still used in repo-owned config/plugin sources and should remain strict-mode friendly.

## High-value commands

### Root-level setup

- Enter dev shell: `nix develop`
- Run all flake checks visible at root: `nix flake check`
- Run generated-output parity check: `nix run .#pkl-check-generated`

### Rust CLI commands

Run these through Nix from repo root unless noted otherwise.

- Build CLI: `nix develop -c sh -c 'cd cli && cargo build'`
- Run CLI: `nix develop -c sh -c 'cd cli && cargo run -- --help'`
- Build packaged CLI output: `nix build .#default`
- Run packaged CLI app: `nix run .#sce -- --help`
- Preferred repo-level verification: `nix flake check`
- Run a single Rust test by exact name when explicitly needed: `nix develop -c sh -c 'cd cli && cargo test parser_routes_mcp -- --exact'`
- Run Rust tests in one module/file pattern when explicitly needed: `nix develop -c sh -c 'cd cli && cargo test setup'`
- Run ignored? none were found; do not assume ignored-test flows exist.
- Rust format verification is covered by `nix flake check`
- Auto-format only (not verification): `nix develop -c sh -c 'cd cli && cargo fmt'`
- Rust lint verification is covered by `nix flake check`

### Bun config/plugin commands

Run from repo root through Nix (do not call bare `bun` on the host). Working directory for the plugin tests is `config/lib/bash-policy-plugin/`.

- Run plugin/runtime test suite: `nix develop -c sh -c 'cd config/lib/bash-policy-plugin && bun test'`
- Run a single Bun test by name: `nix develop -c sh -c 'cd config/lib/bash-policy-plugin && bun test -t "<test name>"'`
- One-shot Bun without the full flake shell: `nix shell nixpkgs#bun -c bun test` (run after `cd` into the plugin dir)

### Useful combined validation flows

- Preferred repo validation from repo root: `nix flake check`
- Config/plugin validation from repo root: `nix develop -c sh -c 'cd config/lib/bash-policy-plugin && bun test'`
- Generated-config validation from repo root: `nix run .#pkl-check-generated`

### Ad-hoc tool examples (non-coreutils)

- Ripgrep: `nix shell nixpkgs#ripgrep -c rg <pattern> <path>`
- jq: `nix shell nixpkgs#jq -c jq . file.json`
- Python: `nix shell nixpkgs#python3 -c python3 script.py`
- Node: `nix shell nixpkgs#nodejs -c node script.js`
- fd: `nix shell nixpkgs#fd -c fd <pattern>`
- shellcheck: `nix shell nixpkgs#shellcheck -c shellcheck script.sh`
- actionlint: `nix shell nixpkgs#actionlint -c actionlint`
- yq: `nix shell nixpkgs#yq-go -c yq . file.yaml`
- hyperfine: `nix shell nixpkgs#hyperfine -c hyperfine '<command>'`

## Testing notes

- Rust tests live inline in source files and in module test files such as `cli/src/services/setup/tests.rs`.
- Rust/Cargo commands should be executed through `nix develop`, even for one-off builds, tests, fmt, and clippy runs.
- Prefer `nix flake check` for routine verification and avoid bare `cargo test` / `cargo check` / `cargo fmt --check` unless the user explicitly asks.
- Rust single-test selection uses standard Cargo substring matching; add `-- --exact` for deterministic one-test runs.
- Bun tests use `bun:test` and support `-t` name filtering; always launch Bun via Nix.
- Bun/plugin tests under `config/lib/bash-policy-plugin/` are lighter-weight repo validation and remain part of the flake check surface.

## CI and release hints

- Release builds generate assistant payloads from canonical `config/pkl/` sources; crates.io and Flatpak stage packaging-only fallbacks in temporary or ignored locations.
- Root `flake.nix` packages `sce` through Crane's `buildDepsOnly` + `buildPackage` pipeline and runs `cli-tests`, `cli-clippy`, and `cli-fmt` through Crane-backed checks.
- Changes to Pkl generation inputs require `nix run .#pkl-check-generated`; generated OpenCode, Claude, and Pi target trees are not committed.

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
- TypeScript uses 2-space indentation, semicolons, trailing commas where multiline, and double-quoted strings in the repo's remaining TS-owned areas.
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
- Treat repository-root `.opencode/` as runtime-managed and keep `config/.opencode`, `config/.claude`, `config/.pi`, and `cli/assets/generated/` absent.
- Edit canonical Pkl and `config/lib` authoring sources rather than temporary generated outputs.

## Working safely as an agent

- Check for unrelated worktree changes before broad edits.
- Avoid destructive git commands unless the user explicitly asks for them.
- When touching canonical generation inputs, verify ephemeral output with `nix run .#pkl-check-generated`; do not regenerate target trees into the repository.
- When verifying changes, prefer `nix flake check` instead of bare `cargo test` / `cargo check`.
- When changing Bun-owned config/plugin code, run the narrowest Bun test or script that covers the change — always through Nix.
- Do not run non-coreutils CLIs from the host PATH; use `nix develop`, `nix shell`, or `nix run` (see **How to run commands**).

## Recommended minimum verification by change type

- Default verification for code changes: `nix flake check`
- Bun/TypeScript config-plugin change: `nix develop -c sh -c 'cd config/lib/bash-policy-plugin && bun test -t "<test name>"'`
- Generated config or Pkl change: `nix run .#pkl-check-generated`
- Cross-cutting repo change: `nix flake check`

## File references worth checking

- `README.md`
- `flake.nix`
- `context/context-map.md`
- `context/overview.md`
- `cli/Cargo.toml`
- `cli/src/app.rs`
- `cli/src/services/setup/tests.rs`
- `config/lib/bash-policy-plugin/package.json`
- `config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`
- `config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts`
