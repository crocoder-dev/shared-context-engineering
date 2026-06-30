# Plan: trace-db-shell

## Change summary

Add an embedded, in-process Agent Trace database shell behind:

```sh
sce trace db shell <uuid-or-alias>
```

The command resolves `<uuid-or-alias>` against the same discovered Agent Trace databases surfaced by `sce trace db list`, where the argument may be either a full checkout UUID/checkout ID or a positional alias such as `agent_trace_0`. After resolution it opens the target local Turso database directly from Rust and starts an interactive SQL shell over that database file.

This must not delegate to the external `turso` CLI or any other external shell binary. The command is thin CLI orchestration over existing trace discovery plus a new embedded shell service.

## Success criteria

- `sce trace db shell <uuid-or-alias>` is accepted by clap and appears in trace DB help.
- `<uuid-or-alias>` resolves using current `sce trace db list` discovery semantics:
  - `agent_trace_N` aliases resolve to the discovered database with that alias.
  - full checkout IDs/UUIDs resolve to the discovered database with matching `checkout_id`.
  - unknown or ambiguous identifiers fail as validation errors with actionable guidance to run `sce trace db list`.
- The resolved database must be schema-ready before the shell starts; skipped/missing-table DBs fail with the existing readiness reason instead of opening an unsafe shell.
- The shell is embedded/in-process only: implementation uses Rust/Turso APIs already in the CLI dependency graph and never invokes the external `turso` command.
- The interactive shell supports a minimal documented operator contract:
  - prints the resolved alias, checkout ID, and database path before accepting input;
  - supports `.help`, `.exit`, and `.quit` dot commands;
  - executes SQL entered by the user against the resolved DB;
  - renders query result rows in deterministic text form;
  - renders non-query statement success without corrupting stdout/stderr contracts for normal command errors.
- Non-interactive stdin use is deterministic enough for tests: piped commands such as `SELECT COUNT(*) FROM diff_traces;` followed by `.exit` can be exercised in automated coverage.
- Targeted tests cover identifier resolution, unknown/skipped DB handling, shell command parsing, query rendering, and a non-interactive shell smoke path.
- `nix flake check` and `nix run .#pkl-check-generated` pass after the implementation is complete.

## Constraints and non-goals

- Do not call or require the external `turso` CLI.
- Do not add a new database schema, migration, or persisted artifact.
- Do not change existing `sce trace db list`, `sce trace status`, or `sce trace status --all` output contracts except for help text that lists the new subcommand.
- Do not add network/Turso Cloud behavior; this opens local Agent Trace DB files already discovered under `<state_root>/sce/`.
- Do not implement full `turso shell` feature parity in this plan. The scope is the minimal embedded operator shell contract listed in success criteria.
- Do not auto-create checkout identities or database files from this command. It only opens databases discovered from existing `agent-trace-*.db` files.

## Assumptions

- The user-facing argument name can be documented as `<uuid-or-alias>` even though discovery stores `checkout_id` strings parsed from `agent-trace-{checkout_id}.db` filenames.
- Existing discovery order remains the alias source of truth; aliases are intentionally positional and may change when DB mtimes change, matching `sce trace db list` behavior.
- A minimal embedded SQL shell is acceptable as long as it is in-process and useful for inspecting the Agent Trace DB. Exact Turso CLI prompt formatting, meta-commands, and table formatting are out of scope.
- Because this is an interactive command, the implementation may route the shell transcript directly through stdio from the command/service boundary if the existing `RuntimeCommand -> String` return contract cannot represent a live REPL cleanly; any such boundary change must stay narrowly scoped to this command.

## Task stack

- [x] T01: `Add trace DB identifier resolution` (status:done)
  - Task ID: T01
  - Goal: Add a pure resolver that maps `<uuid-or-alias>` to one discovered, ready Agent Trace DB using current `discover_agent_trace_dbs()` results.
  - Boundaries (in/out of scope): In — resolver module/function, identifier matching for `alias` and `checkout_id`, ready-vs-skipped validation, actionable error type/messages, focused unit tests using discovered DB fixtures. Out — clap command wiring, interactive shell loop, SQL execution/rendering changes.
  - Done when: Resolver returns the expected DB for `agent_trace_N` and full checkout ID, rejects unknown identifiers with guidance to run `sce trace db list`, rejects skipped DBs with the missing-table reason, and has deterministic tests for each branch.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace'` or the narrower resolver test module once named.
  - Completed: 2026-06-30
  - Files changed: `cli/src/services/trace/discovery.rs`, `cli/src/services/trace/mod.rs`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed. A narrower direct Cargo test command was blocked by repo policy in favor of `nix flake check`.
  - Notes: Added the pure resolver and typed user-actionable errors for alias/checkout-ID resolution, unknown identifiers, ambiguous identifiers, and skipped/not-ready databases. Branch-specific generated resolver tests were removed at user request; existing trace tests plus full flake validation pass.

- [x] T02: `Implement embedded SQL shell core` (status:done)
  - Task ID: T02
  - Goal: Implement the in-process shell loop/core over a resolved Agent Trace DB path, including dot-command handling and deterministic SQL result rendering.
  - Boundaries (in/out of scope): In — shell core module, `.help`/`.exit`/`.quit`, semicolon/newline command handling sufficient for operator SQL inspection, query/non-query execution through existing Turso adapter APIs, deterministic text rendering, tests with piped input/output seams. Out — clap integration, external process execution, full Turso CLI compatibility, rich terminal editing/history.
  - Done when: A test can feed SQL plus `.exit` into the shell core and assert rendered rows/counts; `.help`, `.quit`, malformed SQL diagnostics, and non-query success output are covered; implementation contains no external `turso` process invocation.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::shell'` after the module exists.
  - Completed: 2026-06-30
  - Files changed: `cli/src/services/db/mod.rs`, `cli/src/services/trace/mod.rs`, `cli/src/services/trace/shell.rs`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed. The narrower `nix develop -c sh -c 'cd cli && cargo test services::trace::shell'` command was blocked by repo bash policy in favor of `nix flake check`.
  - Notes: Added a testable embedded Agent Trace DB shell core with path-open/schema-readiness checks, deterministic startup transcript, `.help`/`.exit`/`.quit`, newline/semicolon statement execution, query table rendering, non-query success output, and SQL-error diagnostics that keep the shell running. Added a shared `TursoDb::query_values` helper for fully fetched raw value rows. Context-sync classification: small shared DB adapter surface plus localized trace shell core; updated shared DB and trace-command context.

- [x] T03: `Wire sce trace db shell command surface` (status:done)
  - Task ID: T03
  - Goal: Add `sce trace db shell <uuid-or-alias>` to clap parsing, runtime request conversion, and `TraceCommand` dispatch so it resolves the identifier and starts the embedded shell.
  - Boundaries (in/out of scope): In — `TraceDbSubcommand::Shell`, request enum update, parser conversion, command dispatch, help text, validation-error mapping for resolver failures. Out — new shell features beyond T02, changes to existing trace list/status rendering.
  - Done when: `sce trace db shell --help`/`sce trace db --help` show the shell command, valid aliases/checkout IDs open the embedded shell, unknown identifiers return validation-class errors, and existing trace commands continue to parse and run unchanged.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::parse services::trace'`; manual smoke with piped input once implemented, e.g. `printf '.exit\n' | nix develop -c sh -c 'cd cli && cargo run -- trace db shell agent_trace_0'`.
  - Completed: 2026-06-30
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/trace/command.rs`, `cli/src/services/trace/mod.rs`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed; `nix run .#sce -- trace db --help` showed `shell`; `nix run .#sce -- trace db shell --help` showed `<uuid-or-alias>`; `nix run .#sce -- trace db shell does_not_exist` returned `SCE-ERR-VALIDATION` with `sce trace db list` guidance. The narrower `nix develop -c sh -c 'cd cli && cargo test services::parse::command_runtime services::trace'` command was blocked by repo bash policy in favor of `nix flake check`.
  - Notes: Added clap/runtime request wiring for `sce trace db shell <uuid-or-alias>`, dispatch through existing discovery + resolver, validation-class resolver errors, and direct in-process stdio handoff to the embedded shell core. Existing list/status request variants remain unchanged. Context-sync classification: localized command-surface wiring for an already-planned trace shell; trace-command durable context should include the now-wired shell subcommand.

- [x] T04: `Document trace DB shell operator contract` (status:done)
  - Task ID: T04
  - Goal: Update current-state context/docs for the new shell subcommand and its embedded-only behavior.
  - Boundaries (in/out of scope): In — `context/cli/trace-command.md`, relevant command-surface/default-path/context-map references if needed, and CLI README/help-adjacent docs if they list trace DB subcommands. Out — broad historical plan rewrites and unrelated Agent Trace documentation churn.
  - Done when: Durable context states that `sce trace db shell <uuid-or-alias>` resolves discovered DB aliases/checkout IDs and opens an embedded in-process SQL shell without external `turso`; no current-state docs contradict that behavior.
  - Verification notes (commands or checks): Review `context/context-map.md`, `context/cli/trace-command.md`, and any CLI docs touched by the implementation.
  - Completed: 2026-06-30
  - Files changed: `context/cli/trace-command.md`, `context/plans/trace-db-shell.md`
  - Evidence: Reviewed `context/cli/trace-command.md` operator contract; searched current docs for `trace db shell`, `trace db list`, `external turso`, and `turso shell` references; no contradictory current-state docs found.
  - Notes: Added an explicit operator contract covering `sce trace db list` alias/checkout-ID discovery, ready-only resolution failure modes, startup metadata, `.help`/`.exit`/`.quit`, piped stdin automation, and the embedded-only no-external-database-CLI boundary. Context-sync classification: localized current-state trace command documentation update; root context already referenced the implemented shell and no root edits were required for this task.

- [x] T05: `Validate and clean up trace DB shell` (status:done)
  - Task ID: T05
  - Goal: Run full validation, smoke the new command, and remove temporary scaffolding before handoff.
  - Boundaries (in/out of scope): In — full repo validation, generated-output parity check, manual/non-interactive smoke of the shell command against a discovered DB when available, check for accidental external `turso` invocation, final context sync verification. Out — adding new features after validation begins.
  - Done when: `nix flake check` and `nix run .#pkl-check-generated` pass; a smoke command demonstrates the embedded shell can open a DB and run a simple query or exits cleanly with documented guidance when no DB exists; no temporary files or debug scaffolding remain; plan task evidence is updated.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `rg "Command::new\(\"turso\"\)|turso shell|spawn.*turso" cli/src`; `sce trace db list`; `printf 'SELECT COUNT(*) FROM diff_traces;\n.exit\n' | sce trace db shell <uuid-or-alias>` when a ready DB is available.
  - Completed: 2026-06-30
  - Files changed: `context/plans/trace-db-shell.md`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed; `nix run .#sce -- trace db list` found ready `agent_trace_0`; `printf 'SELECT COUNT(*) FROM diff_traces;\n.exit\n' | nix run .#sce -- trace db shell agent_trace_0` opened the embedded shell and returned count `8`; `Command::new("turso")|turso shell|spawn.*turso` search under `cli/src/**/*.rs` found no matches.
  - Notes: No temporary/debug scaffolding or out-of-scope feature work was needed. Context-sync classification: final validation/plan-evidence update for an already documented feature; expected verify-only root context pass.

## Open questions

None.

## Validation Report

### Commands run

- `nix flake check` -> exit 0 (`all checks passed!`).
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`).
- `nix run .#sce -- trace db list` -> exit 0; discovered ready `agent_trace_0` at `/home/davidabram/.local/state/sce/agent-trace-019ee24b-ac0f-7a51-b8ba-877334cd4bd7.db`.
- `printf 'SELECT COUNT(*) FROM diff_traces;\n.exit\n' | nix run .#sce -- trace db shell agent_trace_0` -> exit 0; shell printed alias/checkout/path metadata and returned `COUNT(*) = 8`.
- Search for `Command::new("turso")|turso shell|spawn.*turso` under `cli/src/**/*.rs` -> no matches.

### Success-criteria verification

- [x] `sce trace db shell <uuid-or-alias>` is accepted by clap and appears in trace DB help — verified in T03 evidence.
- [x] Alias/checkout-ID resolution and skipped/unknown validation behavior are implemented — verified in T01/T03 evidence and retained by `nix flake check`.
- [x] Embedded shell is in-process only — final source search found no external `turso` invocation under `cli/src`.
- [x] Minimal shell operator contract is documented and implemented — verified in T02/T04 evidence and final smoke.
- [x] Non-interactive stdin smoke path works — piped `SELECT COUNT(*) FROM diff_traces;` returned `8`.
- [x] Full validation and generated parity pass — `nix flake check` and `nix run .#pkl-check-generated` both exited 0.

### Failed checks and follow-ups

- None.

### Residual risks

- Positional aliases are intentionally mtime-dependent and may change between `trace db list` and `trace db shell`, as documented in the operator contract.
