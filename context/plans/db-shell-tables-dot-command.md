# Add `.tables` to Agent Trace DB shell

## 1) Change summary

Add SQLite-like `.tables` support to the embedded `sce trace db shell <uuid-or-alias>` command. The dot command should list all tables visible in the opened Agent Trace DB, including SCE internal tables such as `__sce_migrations`, using deterministic output suitable for interactive and piped shell use.

## 2) Success criteria

- `.tables` is accepted by the embedded Agent Trace DB shell alongside existing `.help`, `.exit`, and `.quit` commands.
- Output mimics SQLite shell intent: table names only, not row counts or schema details.
- Internal/system tables are included, including `__sce_migrations` when present.
- Table ordering is deterministic.
- `.help` documents `.tables`.
- SQL execution behavior, shell startup output, and existing dot commands remain unchanged.
- Focused tests cover `.tables`, help text, and error/non-regression behavior.

## 3) Constraints and non-goals

- Do not shell out to external `turso`, `sqlite3`, or other database CLI binaries.
- Do not introduce a dependency on the upstream Turso CLI application code.
- Do not add schema-detail commands such as `.schema`, `.indexes`, `.mode`, or row-count summaries in this plan.
- Do not change `sce trace db list`, `sce trace status`, DB discovery, migrations, or Agent Trace persistence semantics.
- Preserve the shell's current deterministic rendering and non-terminating SQL-error behavior.

## 4) Task stack

- [x] T01: `Implement .tables shell command` (status:done)
  - Task ID: T01
  - Goal: Add `.tables` handling to `services::trace::shell` so it queries and prints all table names from the opened Agent Trace DB.
  - Boundaries (in/out of scope): In - dot-command parsing, table-name query, deterministic ordering, direct shell output, `.help` update. Out - new shell modes, external CLI delegation, schema/row-count output, changes outside the trace shell implementation except tests needed for this behavior.
  - Done when: `.tables` prints table names only; includes internal tables such as `__sce_migrations`; output order is stable; `.help` lists `.tables`; `.exit`/`.quit` and regular SQL statements behave as before.
  - Verification notes (commands or checks): Run focused Rust tests for the trace shell module, preferably through Nix, e.g. `nix develop -c sh -c 'cd cli && cargo test trace::shell'` or the nearest exact test names added/updated for `.tables`.
  - Completed: 2026-06-30
  - Files changed: `cli/src/services/trace/shell.rs`; context sync updated `context/cli/trace-command.md` and `context/context-map.md`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test trace::shell'` was blocked by repository bash policy (`use-nix-flake-check-over-cargo-test`); `nix flake check` compiled and ran trace shell tests but failed in unrelated `services::trace::discovery::tests::missing_required_table_reports_skipped_with_first_missing` with `table diff_traces already exists`.
  - Notes: Added `.tables` dot-command handling via `sqlite_schema` table-name query ordered by name, help text coverage, and focused shell tests for table-name output and ordering.

- [x] T02: `Document .tables shell behavior in context` (status:done)
  - Task ID: T02
  - Goal: Update current-state context to describe `.tables` as a supported embedded shell dot command.
  - Boundaries (in/out of scope): In - `context/cli/trace-command.md` shell contract and any small context-map wording update if needed. Out - broad architecture rewrites, completed-work summaries, or unrelated DB documentation churn.
  - Done when: Context states that `.tables` is supported, lists all tables including internal/system tables, and remains aligned with code behavior.
  - Verification notes (commands or checks): Read the updated context against the implementation; no generated config regeneration should be required unless implementation unexpectedly touches generated assets.
  - Completed: 2026-06-30
  - Files changed: `context/plans/db-shell-tables-dot-command.md`; verified existing current-state coverage in `context/cli/trace-command.md` and `context/context-map.md`.
  - Evidence: Read `cli/src/services/trace/shell.rs` and `context/cli/trace-command.md`; context already documents `.tables`, table-name-only output, deterministic ordering, and inclusion of internal SCE tables such as `__sce_migrations`.
  - Notes: No code or generated-config changes were required for this documentation-only task.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run final checks and clean up the plan state after implementation tasks are complete.
  - Boundaries (in/out of scope): In - full repository validation, formatting/lint/test confirmation, plan checkbox/status updates, context sync verification. Out - additional shell features or opportunistic DB refactors.
  - Done when: Targeted trace shell tests pass, repository-preferred validation passes, context is current-state accurate, and temporary/debug artifacts are removed.
  - Verification notes (commands or checks): Prefer `nix flake check`; also run `nix run .#pkl-check-generated` if any config/generated-adjacent files were touched or as the repo lightweight baseline.
  - Completed: 2026-06-30
  - Files changed: `cli/src/services/trace/discovery.rs`, `cli/src/services/trace/render_list.rs`, `cli/src/services/trace/render_status_all.rs`, `cli/src/services/trace/shell.rs`, `cli/src/services/trace/stats.rs`, `cli/src/services/trace/status.rs`, `cli/src/services/trace/status_all.rs`, `context/plans/db-shell-tables-dot-command.md`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test trace::shell'` was blocked by repository bash policy (`use-nix-flake-check-over-cargo-test`); `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Final validation found trace test flakiness from retrying non-idempotent fixture setup under the shared DB retry policy; cleanup made partial-schema fixture DDL idempotent and batched part fixture inserts so full flake validation is clean. Existing ignored `context/tmp/` trace/debug artifacts were left untouched because they predated this task and are ignored scratch data.

## 5) Open questions

None. User confirmed SQLite-like table-name output and inclusion of internal tables.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test trace::shell'` -> blocked by repository bash policy `use-nix-flake-check-over-cargo-test`; no direct Cargo test executed.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> initially failed in trace DB fixture tests, then passed after cleanup; final output: `all checks passed!`.
- `nix develop -c sh -c 'cd cli && cargo fmt'` -> exit 0; applied rustfmt after cleanup edits.

### Success-criteria verification

- [x] `.tables` is accepted alongside `.help`, `.exit`, and `.quit` -> covered by `services::trace::shell::tests::shell_tables_lists_table_names_in_deterministic_order` in the passing `nix flake check` CLI test derivation.
- [x] Table names only, no row counts or schema details -> shell test asserts no table output lines contain row/table formatting markers.
- [x] Internal/system tables included -> shell test asserts `__sce_migrations` is listed.
- [x] Deterministic ordering -> shell test asserts `__sce_migrations` precedes `diff_traces`, which precedes the smoke table.
- [x] `.help` documents `.tables` -> covered by `services::trace::shell::tests::shell_renders_help_and_quit`.
- [x] SQL execution, shell startup output, and existing dot commands unchanged -> shell non-regression tests passed in `nix flake check`.
- [x] Focused tests cover `.tables`, help text, and error/non-regression behavior -> trace shell test set passed via the full flake CLI test derivation.

### Failed checks and follow-ups

- None remaining. Initial validation exposed retry-sensitive trace fixture setup; cleanup made test fixture setup deterministic under the existing DB retry policy.

### Residual risks

- None identified for this plan.
