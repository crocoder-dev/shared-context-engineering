# Plan: sce-trace-cli

## Change summary

Add a new `sce trace` top-level CLI command group with three subcommands that expose Agent Trace database visibility currently absent from the operator surface:

- `sce trace db list` — scan `<state_root>/sce/agent-trace-*.db`, assign positional aliases (`agent_trace_0..N` by mtime desc), probe schema readiness, and render an alias / status / path table.
- `sce trace status` — resolve the current checkout via `services::checkout`, locate its agent-trace DB, and render counts for `diff_traces`, `messages`, `parts`, `session_models`, `agent_traces`, `post_commit_patch_intersections`, plus last activity timestamp.
- `sce trace status --all` — run the per-DB stats across every discovered DB and render an aggregate totals block plus a per-database breakdown.

All three commands support `--format text` (default) and `--format json`, matching the convention used by `sce auth`, `sce doctor`, and `sce config`. The existing `sce doctor dbs` command is removed; its discovery scan and rendering logic move into the new `services::trace` module.

## Success criteria

- `sce trace db list` prints the documented table (Alias, Status, Path, Updated at) with `agent_trace_{index}` aliases sorted by file mtime descending, `ready` for DBs whose required tables all exist, and `skipped` with a short reason (e.g. `missing table: agent_traces`) for DBs that fail schema probing.
- `sce trace status` (no `--all`) prints the documented per-checkout block for the current cwd's checkout and exits non-zero with a clear message when no DB exists for the resolved checkout id.
- `sce trace status --all` prints the documented Databases / Totals / By database blocks aggregating across every discovered DB (skipped DBs are excluded from totals but counted in the `Databases:` summary line).
- `--format json` is supported on all three subcommands and emits a stable JSON shape under `{"status":"ok","command":"trace","subcommand":...}` analogous to `sce doctor dbs --format json`.
- `sce doctor dbs` is removed from `cli_schema.rs` and the doctor service; help text, command surface, and any tests referencing it are updated. `sce doctor --fix` and `sce doctor` (report) still work.
- `nix flake check` passes (cli-tests, cli-clippy, cli-fmt) and `nix run .#pkl-check-generated` passes after every task.
- New rendering and discovery logic ships with unit tests in the new module, including: alias assignment ordering, schema-probe `ready` vs `skipped` cases, JSON shape snapshot, and a fixture-driven aggregate test.

## Constraints and non-goals

- **Do not** change the on-disk DB layout, schema, or migration set; readiness is probed read-only.
- **Do not** add new clap top-level groups beyond `trace`. Subcommand surface is exactly `trace db list`, `trace status`, `trace status --all`.
- **Do not** introduce a fallback for `sce trace status` when checkout-identity yields nothing — fail with an actionable error.
- **Do not** add backwards-compatibility shims for `sce doctor dbs` (no alias, no deprecation warning) — it is removed in one task.
- **Do not** modify `services::agent_trace_db` schema constants; the schema probe consumes existing table names.
- **Do not** expand the JSON output beyond the data shown in text; defer richer reporting (e.g. per-table size on disk) to a follow-up.
- Activity timestamp (`Last activity`) is derived from the maximum of (`max(diff_traces.time_ms)`, `max(messages.updated_at)`, `max(agent_traces.created_at)`) — pure SQL, no extra schema.

## Assumptions

- `services::checkout::resolve_git_dir` + `read_checkout_id` is the correct cwd-to-checkout resolution path; if `read_checkout_id` returns `Ok(None)`, `sce trace status` errors with guidance to run `sce setup` rather than auto-creating an id.
- The list of required tables for `ready` is exactly the six created by `cli/migrations/agent-trace/`: `diff_traces`, `post_commit_patch_intersections`, `agent_traces`, `messages`, `parts`, `session_models`. Missing any one → `skipped` with the first missing table reported.
- "By database" rows in `--all` use the same alias as `trace db list` (positional `agent_trace_N`, mtime-desc).
- `state_root` resolution stays via `resolve_state_data_root()` (same source as `doctor dbs` today).

## Task stack

- [x] T01: `Scaffold services::trace module with discovery + readiness probe` (status:done)
  - Completed: 2026-06-27
  - Files changed: `cli/src/services/mod.rs`, `cli/src/services/trace/mod.rs` (new), `cli/src/services/trace/discovery.rs` (new)
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). New unit tests `services::trace::discovery::tests::{full_schema_db_reports_ready, missing_required_table_reports_skipped_with_first_missing, aliases_assigned_in_mtime_desc_order_with_checkout_id_tiebreak}` exercise the three required scenarios.
  - Notes: Module registered but not yet wired to any command (T02 will introduce clap surface). Public items carry `#[allow(dead_code)]` / `#[allow(unused_imports)]` while consumers are absent. Readiness probe opens via `AgentTraceDb::open_for_hooks_without_migrations_at` and queries `sqlite_master` per required table in declared order.
  - Task ID: T01
  - Goal: Create `cli/src/services/trace/{mod.rs,discovery.rs}` exposing a `discover_agent_trace_dbs()` function that returns a deterministic Vec of `DiscoveredAgentTraceDb { alias, checkout_id, path, mtime, readiness }` sorted by mtime desc (ties broken by `checkout_id` asc) with `alias = format!("agent_trace_{i}")`. Readiness is computed by probing for the six required tables (`diff_traces`, `post_commit_patch_intersections`, `agent_traces`, `messages`, `parts`, `session_models`) using a read-only sqlite open; report the first missing table as the skip reason. No command wiring yet.
  - Boundaries (in/out of scope): In — new module files, discovery struct, mtime-desc sort, table-probe helper, unit tests with a tempdir fixture creating two DBs (one full schema, one missing `agent_traces`). Out — clap surface, rendering, removing `doctor dbs`, stat queries.
  - Done when: `cargo check` and `cargo test -p sce -- services::trace::discovery` pass; unit tests cover (a) alias assignment ordering by mtime, (b) `ready` for a full-schema DB, (c) `skipped` with the missing-table reason. Module is registered in `services/mod.rs` but unused by any command yet.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::discovery'`; `nix develop -c sh -c 'cd cli && cargo clippy --all-targets -- -D warnings'`.

- [x] T02: `Add trace command clap surface and registry stub` (status:done)
  - Completed: 2026-06-27
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/trace/mod.rs`, `cli/src/services/trace/command.rs` (new), `cli/src/services/command_registry.rs`, `cli/src/services/parse/command_runtime.rs`
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). `nix run .#pkl-check-generated` → "Generated outputs are up to date.".
  - Notes: Stub `TraceCommand::execute` returns `sce trace <subcommand>: not implemented` for both `db list` and `status` (and `status --all`). `default_registry_lists_all_commands_deterministically` test updated to include `"trace"`. Clippy `unnecessary_wraps` allowed on stub `execute` because T05 introduces error paths.
  - Task ID: T02
  - Goal: Add `Commands::Trace { subcommand: TraceSubcommand }` to `cli_schema.rs` with `TraceSubcommand::Db { subcommand: TraceDbSubcommand::List { format } }` and `TraceSubcommand::Status { all: bool, format }`. Add `TRACE_*` top-level metadata constants and a `TopLevelCommandMetadata` entry (`show_in_top_level_help: true`). Add a `services::trace::NAME = "trace"` constant and stub `TraceCommand` `RuntimeCommand` impl that returns `"not implemented"` for now. Wire it into `parse::command_runtime` so `sce trace db list` / `sce trace status` parse cleanly and dispatch to the stub.
  - Boundaries (in/out of scope): In — clap enums, top-level metadata entry, registry registration, stub command. Out — actual rendering, stat queries, removing `doctor dbs`.
  - Done when: `sce trace --help`, `sce trace db --help`, `sce trace db list --help`, `sce trace status --help` all render without error; `sce trace db list` prints the stub message; `cargo check` passes and `nix run .#pkl-check-generated` passes (cli surface regenerated if needed).
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- trace db list'`; `nix develop -c sh -c 'cd cli && cargo run -- trace status --help'`; `nix run .#pkl-check-generated`.

- [x] T03: `Implement sce trace db list rendering (text + json)` (status:done)
  - Completed: 2026-06-27
  - Files changed: `cli/src/services/trace/mod.rs`, `cli/src/services/trace/command.rs`, `cli/src/services/trace/render_list.rs` (new)
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). Unit tests added under `services::trace::render_list::tests` cover empty-state text+json, mixed-fixture text table with ready+skipped rows, and mixed-fixture json shape (alias, checkout_id, status, skip_reason, mtime).
  - Notes: `TraceCommand::execute` now dispatches `DbList { format }` to `discover_agent_trace_dbs()` + `render_list::render`. JSON shape is `{"status":"ok","command":"trace","subcommand":"db.list","databases":[{alias,checkout_id,path,status,skip_reason?,mtime}]}`. Text table uses dynamic column widths and the `services::style::heading` helper. Status subcommand still returns the not-implemented stub (T05/T06).
  - Task ID: T03
  - Goal: Wire `sce trace db list` to call `discover_agent_trace_dbs()` and render the documented table for `--format text` (columns: Alias, Status, Path; status `ready` or `skipped <reason>`) and the JSON shape `{"status":"ok","command":"trace","subcommand":"db.list","databases":[{alias,checkout_id,path,status,skip_reason?,mtime}]}` for `--format json`. Use the styling helpers in `services::style` for headings consistent with other commands.
  - Boundaries (in/out of scope): In — `services/trace/render_list.rs`, text and json renderers, command handler dispatch for this subcommand only, snapshot tests for both outputs against a tempdir fixture. Out — `status` subcommand, removing `doctor dbs`.
  - Done when: Running against a fixture state root with two ready DBs and one missing-table DB produces the table shown in the change request body; JSON snapshot test passes; `cargo test` and `cargo clippy` pass.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::render_list'`; manual: `XDG_DATA_HOME=$(mktemp -d) cargo run -- trace db list` returns empty-state message.

- [x] T04: `Add stat-query layer for per-checkout AgentTraceDb counts` (status:done)
  - Completed: 2026-06-28
  - Files changed: `cli/src/services/trace/mod.rs`, `cli/src/services/trace/stats.rs` (new)
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). Unit tests added under `services::trace::stats::tests`: `collect_stats_returns_counts_and_last_activity` (seeded migrated DB with diff traces, messages, parts, session model, agent trace, intersection — asserts the six counts and last_activity ≥ the latest diff trace timestamp), `collect_stats_on_empty_db_returns_zero_counts_and_no_activity`, `parse_iso_millis_handles_sqlite_strftime_output`.
  - Notes: `AgentTraceDbStats` carries `#[allow(dead_code)]` until T05 wires the renderer. `last_activity` is computed in Rust by maxing `MAX(diff_traces.time_ms)` (millis → `DateTime::from_timestamp_millis`) with parsed `MAX(messages.updated_at)` and `MAX(agent_traces.created_at)` (ISO8601 via RFC3339 with a fallback `%Y-%m-%dT%H:%M:%S%.fZ` parser). Counts use `SELECT COUNT(*)` per table.
  - Task ID: T04
  - Goal: Add `services::trace::stats` with a function `collect_agent_trace_db_stats(path: &Path) -> Result<AgentTraceDbStats>` returning the six row counts (`diff_traces`, `messages`, `parts`, `session_models`, `agent_traces`, `post_commit_patch_intersections`) and a `last_activity` `Option<DateTime<Utc>>` derived from `MAX(diff_traces.time_ms, messages.updated_at, agent_traces.created_at)` (whichever columns exist). Read-only sqlite open; errors propagate. No command wiring.
  - Boundaries (in/out of scope): In — pure stat query module, unit test that seeds a tempdir DB with the real migrations and asserts counts and last-activity. Out — rendering, command dispatch, multi-DB aggregation.
  - Done when: Unit test exercises a freshly-migrated DB, inserts a known number of rows in each table, and asserts the returned counts and timestamp match; `cargo test` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::stats'`.

- [x] T05: `Implement sce trace status (current checkout) rendering` (status:done)
  - Completed: 2026-06-28
  - Files changed: `cli/src/services/trace/mod.rs`, `cli/src/services/trace/discovery.rs`, `cli/src/services/trace/command.rs`, `cli/src/services/trace/status.rs` (new), `cli/src/services/trace/render_status.rs` (new)
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). Unit tests added: `services::trace::status::tests::{missing_git_repo_reports_not_in_git_repo, missing_checkout_id_reports_no_checkout_id, missing_db_file_reports_db_missing, ready_db_returns_stats_report, partial_schema_db_returns_skipped_status}`, `services::trace::render_status::tests::{ready_text_renders_all_counts_and_last_activity, ready_text_renders_never_when_last_activity_absent, skipped_text_renders_skip_reason, ready_json_shape_matches_contract, skipped_json_shape_matches_contract}`.
  - Notes: `probe_readiness` exposed as `pub(super)` so status resolution reuses the same schema probe used by discovery. The status pipeline is split into `status.rs` (orchestration: `resolve_current_status_in(repo_root, sce_dir)` + production wrapper `resolve_current_status(repo_root)`) and `render_status.rs` (pure text + JSON renderers). The three documented error paths return `StatusError::{NotInGitRepo, NoCheckoutId, DbMissing}` mapped to `ClassifiedError::validation` (exit 3); sqlite/IO failures stay runtime-class (exit 4). Skipped DBs (file exists but missing required tables) render `Status: skipped: missing table 'X'` and JSON `db_status: "skipped"` with `skip_reason` (no stats), exit 0 — not enumerated as an error case in the plan.
  - Task ID: T05
  - Goal: Wire `sce trace status` (no `--all`) to resolve the cwd's checkout via `services::checkout::resolve_git_dir` + `read_checkout_id`, locate `agent-trace-{id}.db`, run `collect_agent_trace_db_stats`, and render the per-checkout block shown in the change request body. Error cases: not inside a git repo → guidance to cd into one; no checkout id → guidance to run `sce setup`; DB file missing → guidance that no traces have been recorded yet (all exit non-zero with a stable message). JSON shape: `{"status":"ok","command":"trace","subcommand":"status","checkout_id":...,"db_status":"ready|skipped","stats":{...},"last_activity":...}`.
  - Boundaries (in/out of scope): In — handler for `Status { all: false }`, text + json renderers, integration test using a tempdir HOME/state_root and a fake git repo with a written checkout id. Out — `--all` aggregation, removing `doctor dbs`.
  - Done when: Running in a fixture repo with a populated DB matches the expected text block; JSON snapshot test passes; the three error paths return non-zero with the documented messages; `cargo test` and `cargo clippy` pass.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::status'`; manual end-to-end against the current repo: `cargo run -- trace status`.

- [x] T06: `Implement sce trace status --all aggregation rendering` (status:done)
  - Completed: 2026-06-28
  - Files changed: `cli/src/services/trace/mod.rs`, `cli/src/services/trace/command.rs`, `cli/src/services/trace/status_all.rs` (new), `cli/src/services/trace/render_status_all.rs` (new)
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). Unit tests added under `services::trace::status_all::tests::{empty_sce_dir_reports_zero_discovery_and_totals, mixed_fixture_aggregates_ready_and_lists_skipped}` and `services::trace::render_status_all::tests::{empty_renders_text_with_zeroed_summary_and_totals, empty_renders_json_with_zeroed_shape, mixed_fixture_renders_text_blocks_with_per_database_rows, mixed_fixture_renders_json_aggregate_and_breakdown}`.
  - Notes: `aggregate_status_all_in(sce_dir)` walks `discover_agent_trace_dbs_in`, runs `collect_agent_trace_db_stats` on every `Ready` DB and accumulates `Totals` (sum of six counts + max `last_activity`); `Skipped` DBs are excluded from totals but counted in the discovery summary line and surfaced as `Status: skipped: missing '<table>'` rows in the per-database table (placeholder `-` cells). JSON shape: `{"status":"ok","command":"trace","subcommand":"status.all","discovery":{discovered,ready,skipped},"totals":{...,last_activity},"databases":[{alias,checkout_id,path,status,...counts|skip_reason}]}`. Empty discovery prints `Databases: 0 discovered, 0 ready, 0 skipped` plus zeroed totals and omits the `By database` block (no rows).
  - Task ID: T06
  - Goal: Extend the status command handler so that `--all` calls `discover_agent_trace_dbs()`, runs `collect_agent_trace_db_stats` over every `ready` DB (skipping `skipped` ones but counting them in the discovery summary line), and renders the Databases / Totals / By database blocks shown in the change request body. JSON shape: `{"status":"ok","command":"trace","subcommand":"status.all","discovery":{"discovered":N,"ready":N,"skipped":N},"totals":{...},"databases":[{"alias":...,"traces":N,"diffs":N,"messages":N,...}]}`.
  - Boundaries (in/out of scope): In — aggregation function, text + json renderers, fixture test with three DBs (two ready, one skipped) asserting totals and per-row breakdown. Out — removing `doctor dbs`.
  - Done when: Fixture test asserts the exact text block and JSON snapshot; running against an empty state root prints `Databases: 0 discovered, 0 ready, 0 skipped` with zeroed totals and no per-database rows; `cargo test` and `cargo clippy` pass.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test services::trace::status_all'`.

- [x] T07: `Remove sce doctor dbs command and dead code` (status:done)
  - Completed: 2026-06-28
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/command_registry.rs`, `cli/src/services/checkout/mod.rs`
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). `nix run .#pkl-check-generated` → "Generated outputs are up to date.". `rg "doctor dbs|DoctorSubcommand|DoctorAction|run_doctor_dbs|DiscoveredCheckout|discover_checkouts_from_filesystem|sort_checkouts_by_last_seen_desc|render_doctor_dbs_(text|json)" cli/src` → no hits.
  - Notes: Removed `DoctorSubcommand` enum, `DoctorAction` enum, `DiscoveredCheckout`, `run_doctor_dbs`, `discover_checkouts_from_filesystem`, `sort_checkouts_by_last_seen_desc`, `render_doctor_dbs_text`, `render_doctor_dbs_json`, and the `action` field from `DoctorRequest`. `convert_doctor_command` simplified to a non-`Result` return now that the only validation branch (`--fix` + `dbs`) is gone. Stale doc-comment in `services::checkout` updated to point at `sce trace db list`. Removed `chrono::{DateTime, Utc}` and `serde_json::json` imports from `doctor/mod.rs`.
  - Task ID: T07
  - Goal: Remove `DoctorSubcommand::Dbs`, `DoctorAction::Dbs`, `run_doctor_dbs`, `discover_checkouts_from_filesystem`, `sort_checkouts_by_last_seen_desc`, `render_doctor_dbs_text`, `render_doctor_dbs_json`, and `DiscoveredCheckout` from `services::doctor`. Update `parse::command_runtime` to drop the `Dbs` arm. Update help text generation and any tests referencing `sce doctor dbs`. Confirm `sce doctor` (report mode) and `sce doctor --fix` are unaffected.
  - Boundaries (in/out of scope): In — single-purpose removal commit: schema, dispatch, helpers, tests, help. Out — moving discovery logic (already moved in T01), adding new behavior.
  - Done when: `rg "doctor dbs|DoctorSubcommand::Dbs|DoctorAction::Dbs|run_doctor_dbs|DiscoveredCheckout"` returns no hits in `cli/src`; `sce doctor --help` no longer lists `dbs`; `sce doctor` and `sce doctor --fix` still run; `nix flake check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- doctor --help'`; `nix develop -c sh -c 'cd cli && cargo test'`; `rg "doctor dbs" cli/src` returns no hits.

- [x] T08: `Validate and sync context` (status:done)
  - Completed: 2026-06-28
  - Files changed: `context/plans/sce-trace-cli.md`
  - Evidence: `nix flake check` → all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, config-lib-biome, workflow-actionlint, flatpak parity). `nix run .#pkl-check-generated` → "Generated outputs are up to date.". Manual smoke against local state: `sce trace db list` → 1 ready DB (`agent_trace_0`); `sce trace status` → checkout `019ed063-...`, Status: ready, counts (123 diff traces, 657 messages, 561 parts, 2 session models, 14 agent traces, 14 post-commit intersections), Last activity `2026-06-27T23:20:37.563+00:00`; `sce trace status --all` → `Databases: 1 discovered, 1 ready, 0 skipped` with matching totals and per-database breakdown. `rg "doctor dbs" context cli/README.md -g '!context/plans/**'` → only the accurate current-state `context/context-map.md:17` entry documenting the completed removal; no hits in `context/cli/**` or `cli/README.md`.
  - Notes: No durable context edits were required — `context/cli/trace-command.md`, `context/cli/cli-command-surface.md` (visible command list includes `trace`; doctor section documents the move), and `context/context-map.md` were already synced to current state during T01–T07. Remaining `doctor dbs` references live only in `context/plans/` execution artifacts (this plan's task descriptions plus completed plans `agent-trace-checkout-identity`, `remove-checkout-registry`, `drop-doctor-dbs-path-remote-url`), which are disposable historical records per the `sce-plan-review` contract and are out of scope to rewrite. `cli/README.md` carries no `doctor dbs` or top-level command-surface listing requiring updates.
  - Task ID: T08
  - Goal: Run the full validation suite, exercise the three new commands end-to-end against the current repo state, and update the context map / CLI docs to reflect the new `trace` group and the removal of `doctor dbs`.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, manual smoke of `sce trace db list`, `sce trace status`, `sce trace status --all`, and updates to `context/cli/`, `context/context-map.md`, and `cli/README.md` where they reference `doctor dbs` or list the top-level command surface. Out — adding new behavior beyond this plan.
  - Done when: All checks pass; manual smoke output matches the change request body shape against the developer's local state; context docs reference `sce trace` and no longer mention `sce doctor dbs`.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `sce trace db list`; `sce trace status`; `sce trace status --all`; `rg "doctor dbs" context cli/README.md`.

## Open questions

None — clarifications resolved 2026-06-27.
