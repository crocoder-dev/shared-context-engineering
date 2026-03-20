# Change summary

Restore CLI config validation compatibility so repo-local and global `sce/config.json` files may include the canonical `"$schema": "https://sce.crocoder.dev/config.json"` field without breaking command startup. The fix should cover runtime validation paths used before normal command dispatch so installed `sce` binaries can still start `version`, `mcp`, and other commands when `@.sce/config.json` includes `$schema`.

# Success criteria

- `@.sce/config.json` may keep the canonical `$schema` field and still validate successfully.
- Startup paths that currently fail before command execution (`sce version`, `sce mcp`, and other config-loading commands) no longer reject `$schema` as an unknown property.
- Operator-facing validation/help text and durable context consistently describe `$schema` as an allowed config key.
- Regression coverage exists for the failing case so future schema/config changes do not reintroduce the startup failure.

# Constraints and non-goals

- In scope: CLI config loading and validation behavior, generated schema ownership, embedded-schema/runtime parity, targeted tests, and context updates needed to reflect the allowed `$schema` field.
- In scope: diagnosing whether the failure comes from Rust-side validation, stale embedded schema usage, generated-schema drift, or command-startup wiring that validates config before dispatch.
- Out of scope: changing unrelated observability defaults, MCP tool behavior beyond unblocking startup, or redesigning config precedence.
- Out of scope: removing `$schema` from operator config files; `$schema` must remain supported.

# Task stack

- [x] T01: `Diagnose config-schema startup mismatch` (status:done)
  - Task ID: T01
  - Goal: Identify the exact mismatch that causes installed `sce` binaries to reject `$schema` even though the generated schema artifact currently defines it.
  - Boundaries (in/out of scope): In - `cli/src/services/config.rs`, related startup/config-loading paths, schema embedding source, generated schema artifact, and existing tests/docs that describe allowed keys. Out - fixing unrelated config behaviors or altering command semantics beyond diagnosis-backed scope.
  - Done when: The root cause is documented in the plan execution notes and narrowed to a concrete code/config contract issue that can be fixed atomically in follow-up tasks.
  - Verification notes (commands or checks): Reproduce the failure with the current repo-local `@.sce/config.json`; inspect generated schema ownership and embedded-schema loading path; confirm whether the failing validator reads stale/hand-maintained allowed-key lists or a different schema payload than `config/schema/sce-config.schema.json`.
  - Completed: 2026-03-19
  - Files changed: `context/plans/config-schema-dollar-schema-fix.md`
  - Evidence: `nix develop -c cargo run --manifest-path cli/Cargo.toml -- version` from repo root fails with `unknown key '$schema'`; `config/pkl/base/sce-config-schema.pkl` and `config/schema/sce-config.schema.json` both allow `$schema`; `cli/src/services/config.rs` embeds the current generated schema but still rejects `$schema` in a separate manual top-level allow-list inside `parse_file_config`.
  - Notes: The startup failure is not stale-schema drift. It is a runtime parity bug: schema validation succeeds, then `parse_file_config` runs an extra Rust-owned unknown-key gate that omits `$schema` (`cli/src/services/config.rs`). The failure only appears on startup paths that discover `.sce/config.json` from the actual working directory, which is why `cd cli && cargo run -- version` passes while installed or repo-root invocations fail.

- [x] T02: `Implement runtime support for $schema in sce/config.json` (status:done)
  - Task ID: T02
  - Goal: Update the CLI config validation path so `$schema` is accepted consistently anywhere `sce/config.json` is loaded during startup.
  - Boundaries (in/out of scope): In - Rust config validation/loading code, generated schema integration or embedding fixes, and any required schema-source updates if the canonical source is wrong. Out - broad config refactors, new config keys, or unrelated CLI output changes.
  - Done when: The runtime accepts `$schema` in config files and command startup proceeds normally for affected commands without regressing existing unknown-key enforcement.
  - Verification notes (commands or checks): Add or update targeted tests covering config validation/startup with `$schema` present and unknown-key rejection for truly unsupported fields.
  - Completed: 2026-03-19
  - Files changed: `cli/src/services/config.rs`, `context/architecture.md`, `context/cli/config-precedence-contract.md`, `context/context-map.md`, `context/glossary.md`, `context/overview.md`, `context/plans/config-schema-dollar-schema-fix.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test schema_key_in_config_file && cargo test unknown_config_keys'`; `nix develop -c sh -c 'cd cli && cargo fmt --check'`; `nix develop -c sh -c 'cd cli && cargo build'`
  - Notes: The Rust-owned top-level allowed-key gate in `parse_file_config` now explicitly includes `$schema`, keeping runtime startup config parsing aligned with the embedded generated schema and preserving unknown-key rejection for unsupported fields.

- [x] T03: `Align docs and current-state context` (status:done)
  - Task ID: T03
  - Goal: Update operator-facing docs and SCE context so allowed-key documentation matches the implemented `$schema` support.
  - Boundaries (in/out of scope): In - relevant current-state docs under `context/` and any nearby CLI-facing documentation that lists supported config keys. Out - unrelated wording cleanup or broad documentation rewrites.
  - Done when: Current-state docs no longer contradict the implementation and explicitly reflect that `$schema` is permitted in `sce/config.json`.
  - Verification notes (commands or checks): Check `context/cli/config-precedence-contract.md`, `context/overview.md`, and any touched docs for consistent allowed-key wording.
  - Completed: 2026-03-20
  - Files changed: `context/plans/config-schema-dollar-schema-fix.md`
  - Evidence: Verified consistent `$schema` support wording in `context/cli/config-precedence-contract.md`, `context/overview.md`, `context/architecture.md`, `context/context-map.md`, and `context/glossary.md`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: The operator-facing current-state docs already describe `$schema` as an allowed top-level `sce/config.json` key, so this task completed as a doc-consistency verification plus plan-state update without additional content edits.

- [x] T04: `Run validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Verify the fix end-to-end, confirm generated/config parity, and remove any temporary diagnostics or scaffolding from the implementation tasks.
  - Boundaries (in/out of scope): In - targeted command validation for the reproduced failure, repo-standard verification, and cleanup of temporary test/debug artifacts. Out - unrelated refactors or follow-on improvements.
  - Done when: The failing startup scenario is verified fixed, repo-required checks for this change have run, and no temporary scaffolding remains.
  - Verification notes (commands or checks): `sce version`; `sce mcp` startup smoke validation; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-03-20
  - Files changed: `context/plans/config-schema-dollar-schema-fix.md`
  - Evidence: `nix run .#sce -- version`; `nix build .#sce && timeout 5s ./result/bin/sce mcp`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Packaged `sce version` now completes successfully from repo root with startup config loading active. A bounded packaged `sce mcp` smoke run reaches MCP runtime startup and fails only after launch because no client `initialize` request is sent, which confirms the previous pre-dispatch `$schema` rejection no longer blocks command startup. Generated output parity and repository flake checks both pass, and no task-specific temporary diagnostics or scaffolding required removal.

# Open questions

- None at plan time; root-cause findings from T01 should be recorded before T02 implementation begins.

# Validation Report

## Commands run

- `nix run .#sce -- version` -> exit 0; packaged CLI started from repo root and printed `sce 0.1.0 (a83e317d7684)`.
- `nix build .#sce && timeout 5s ./result/bin/sce mcp` -> bounded smoke reached MCP runtime startup, then exited with `Error [SCE-ERR-RUNTIME]: MCP server error: connection closed: initialize request` because no MCP client initialized the server during the smoke run.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; flake package, app, parity, CLI tests, clippy, and fmt checks passed.

## Success-criteria verification

- [x] `@.sce/config.json` may keep the canonical `$schema` field and still validate successfully -> covered by the completed runtime fix in `cli/src/services/config.rs` and preserved by passing packaged startup validation in `nix run .#sce -- version`.
- [x] Startup paths that currently fail before command execution (`sce version`, `sce mcp`, and other config-loading commands) no longer reject `$schema` as an unknown property -> confirmed by successful packaged `sce version` startup and by packaged `sce mcp` reaching MCP runtime initialization instead of failing during config load.
- [x] Operator-facing validation/help text and durable context consistently describe `$schema` as an allowed config key -> verified during the context-sync pass against `context/overview.md`, `context/architecture.md`, `context/context-map.md`, `context/glossary.md`, and `context/cli/config-precedence-contract.md`.
- [x] Regression coverage exists for the failing case so future schema/config changes do not reintroduce the startup failure -> maintained by the targeted config tests recorded in T02 and revalidated indirectly by `nix flake check`.

## Context sync result

- Classification: verify-only final-task sync.
- Verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` still match current code truth for `$schema` startup support.
- Feature existence documentation is already present and discoverable through `context/cli/config-precedence-contract.md` and linked shared context files; no additional root-context edits were required.

## Temporary scaffolding cleanup

- No task-specific temporary diagnostics, debug code, or scaffolding remained to remove.

## Failed checks and follow-ups

- None.

## Residual risks

- None identified for this plan scope.
