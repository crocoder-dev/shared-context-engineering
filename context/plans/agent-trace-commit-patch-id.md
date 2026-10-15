# Plan: agent-trace-commit-patch-id

## Change summary

Extend the existing SCE CLI Agent Trace capture and synchronization pipeline to persist a stable Git patch fingerprint for each original commit. When `post-commit` creates an Agent Trace, SCE computes `git patch-id --stable` from the full committed patch, saves the result in the repository-scoped `agent_traces.commit_patch_id` column, and exports it as the optional `commitPatchId` field of the existing `agent_traces` sync stream. The fingerprint is independent of the original commit SHA and of SCE's AI-attribution intersection patch.

This extends existing `post-commit` → `RepositoryAgentTraceDb` → `AgentTraceExportReader` → `sce sync` behavior. The Agent Trace JSON, original `commit_id`, source identity, sync cursors, remote stream count, and auto-sync lifecycle remain unchanged. `insert_agent_trace` has exactly one production writer (`run_post_commit_agent_trace_flow` in `cli/src/services/hooks/commit_hooks.rs`; `post-rewrite` is a no-op), so no other capture path needs the column. Control Plane support for the incoming optional `commitPatchId` field is a release prerequisite, not an implementation task in this SCE-only plan.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: For a normal non-merge commit, the captured fingerprint equals the first column of `git patch-id --stable` applied to the commit's entire canonical binary-capable Git patch (the documented pinned invocation); changing only the commit message, or replaying an equivalent patch onto another parent, does not change the fingerprint; user Git diff configuration (prefixes, path quoting, diff algorithm, context, color, external diff/textconv) does not change it.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` using temporary Git repositories covering commit-message amend, equivalent cherry-pick/rebase, root commit, and user diff-configuration variations (guard with `-- --list` that the filter matches at least one test).
- [ ] AC2: Empty commits and multi-parent merge commits yield a `None` fingerprint; binary-only and rename-only commits produce deterministic fingerprints. A failed, malformed-output, oversized, or timed-out Git subprocess returns an error (surfaced as a sanitized warning) without panicking, leaking a child process, or blocking indefinitely, and the original Agent Trace row is still persisted with `commit_patch_id IS NULL`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` with binary, rename, empty, merge, error, malformed-output, size-limit, and timeout cases; assert `None` for empty/merge and an error/diagnostic for actual failures.
- [ ] AC3: An existing repository Agent Trace database at migration `006_mutation_trace_health_invariant` upgrades through `sce setup` to `007_commit_patch_id`, retaining existing trace rows, IDs, URLs, and `trace_json`; old rows have `commit_patch_id IS NULL`. Repeated setup is idempotent. Hook runtime never runs migrations and reports `Run 'sce setup'.` guidance for a pre-007 schema; `sce doctor` reports the schema as not ready until setup runs.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` against a pre-007 database fixture, inspecting `PRAGMA table_info(agent_traces)`, the `__sce_migrations` ledger, unchanged rows, and hook/doctor readiness results.
- [ ] AC4: Successful post-commit Agent Trace insertion persists the exact captured `commit_id` and its nullable `commit_patch_id` in the same SQL insert, without changing or reserializing the Agent Trace JSON for fingerprint purposes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` asserting local database values for successful and unavailable fingerprints and byte-identical `trace_json` versus the pre-change serialization path.
- [ ] AC5: `AgentTraceExportReader::read_agent_traces_after()` returns the stored optional patch ID unchanged; serialized export rows contain `commitPatchId` (string or `null`) and preserve existing fields, `traceJson` bytes, and `sourceRowId` ordering.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` asserting query mapping, JSON serialization, and `traceJson` byte preservation for rows with and without fingerprints.
- [ ] AC6: `sce sync` and post-commit auto-sync send `commitPatchId` on the existing `agent_traces` batch payload, without introducing a fourth remotely uploaded stream, a new cursor, or changes to `/state`, retry, or conflict behavior. Previously acknowledged rows are not replayed because of this migration.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync_uploads_only_three_streams_and_reports_zero_upload_diff_traces` plus a captured local HTTP ingestion request assertion for `rows[*].commitPatchId`, cursor advancement, and no replay of rows at or below the server cursor.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix flake check`

### Context sync

- `context/sce/agent-trace-db.md` — new nullable `agent_traces.commit_patch_id` column, additive migration `007_commit_patch_id`, setup-only migration / hook-runtime setup requirement, upgrade/downgrade note; repair existing drift (migration list in the adapter-seam paragraph stops at `005`, the `agent_traces` column list, stale `015_add_diff_traces_payload_type` reference, and the "hook runtime lazy initialization" sentence that contradicts the no-migration hook path).
- `context/sce/agent-trace-hooks-command-routing.md` — best-effort canonical fingerprint capture, exact pinned Git invocation, and failure behavior.
- `context/sce/agent-trace-export-readers.md` — optional `commitPatchId` export field.
- `context/cli/agent-trace-sync-command.md` — unchanged cursor protocol, new field on the existing `agent_traces` stream, Control Plane release dependency, and no historical backfill.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-repository/007_commit_patch_id.sql`; `cli/src/services/agent_trace_db/mod.rs` (`AgentTraceInsert`, `INSERT_AGENT_TRACE_SQL`, `insert_agent_trace_with`); `cli/src/services/hooks/commit_hooks.rs` and a narrowly scoped Git patch-ID helper module under `cli/src/services/hooks/`; `cli/src/services/agent_trace_export/mod.rs`; the `sync.rs` test module; focused migration, hook, export, and sync tests; the durable context files listed above.
- **Out of scope:** Control Plane API/migration implementation, GitHub events, reconciliation/lineage graph, historical backfill, new SCE commands, remote `diff_traces` upload, changing the Agent Trace specification, rehashing trace JSON, adding another synchronization stream, and changing the existing `capture_head_patch_from_git` / intersection capture.
- **Canonical algorithm:** version-1 fingerprint is `git patch-id --stable` over the full commit patch generated with explicitly pinned flags, starting from `git show --format= --patch --binary --no-ext-diff --no-textconv --no-renames --no-color --diff-algorithm=myers <captured-oid>`. Pin every additional option whose user configuration can change patch-id input (at minimum: source/destination prefix, `core.quotePath`, unified context, indent heuristic, `diff.relative`, `diff.orderFile`) via flags or `-c` overrides, and pass `--stable` explicitly so `patchid.stable`/`patchid.verbatim` config cannot change it. Document the final exact invocation for Control Plane's future matcher. A fingerprint is patch-equivalence evidence, never proof of unique commit identity.
- **Process safety:** no shell pipeline; invoke both Git subprocesses directly with `tokio::process::Command` (`kill_on_drop(true)`, following the precedent in `cli/src/services/mutation_trace/runtime/git_snapshot.rs`), streaming `git show` stdout into `git patch-id` stdin rather than buffering the diff, with a finite overall deadline, a byte bound on streamed patch input and on patch-id output, exit-status and output validation for both processes, and cleanup of both processes on cancellation/error. No dyn dispatch and no new generalized process framework.
- **Capture semantics:** use `post_commit_data.commit_oid` already captured for the Agent Trace, not a second HEAD lookup. Empty commits yield `None`; multi-parent merges yield `None` rather than a first-parent identity. Accept 40-hex (SHA-1) and 64-hex (SHA-256) object IDs and patch IDs; validate output as lowercase hex of the matching length. Never derive the ID from SCE's AI-contributed patch.
- **Failure policy:** fingerprint computation is best-effort. Emit a sanitized warning and persist `NULL` when unavailable, but never weaken existing Agent Trace build, validation, or persistence errors. No automatic retry and no replacement of the original trace row.
- **Migrations:** additive migration `007` rather than altering `001`. `sce setup` applies migrations; high-frequency hooks do not. Because hook readiness requires exact migration-ledger parity, an upgraded CLI fails open on all repository hook writes until `sce setup` reruns (existing precedent from migrations `003`–`006`). Older CLI binaries may reject a newer ledger on downgrade; document upgrade/downgrade behavior.
- **Release dependency:** deploy a Control Plane receiver that accepts and persists optional `commitPatchId` before releasing the SCE export, because the current server decoder may strip an unknown field while acknowledging the row, and acknowledged rows are never re-sent.
- **Non-goal:** automatic re-sync of traces the server already acknowledged. That requires an explicit later backfill/change stream, not cursor rewinding.

## Assumptions

- Fingerprints are a nullable column on `agent_traces` rather than a new normalized commits table; this is sufficient for the current single-trace-row-to-commit association and existing export protocol.
- The wire field is `commitPatchId`, a lowercase hex Git patch ID or `null`, always present on serialized `agent_traces` export rows (not skipped when `None`). A future schema may add an algorithm/version discriminator if the canonical algorithm changes.
- Verification commands use the repository convention (`nix develop -c ./scripts/run-cli-cargo.sh …`) instead of bare `cargo test --manifest-path cli/Cargo.toml`, because `cli/build.rs` requires the generated Pkl payload that the wrapper provides; the package is `shared-context-engineering`.
- No new Cargo dependency is needed: `tokio` already enables `process`, `io-util`, and `time`.
- `run_post_commit_agent_trace_flow` currently takes no logger; T03 threads the existing post-commit `Option<&L>` logger (or an equivalent warning seam) into it rather than inventing a new logging channel.
- Test cases may be grouped into table-driven tests sharing temporary repositories; every new test name contains `commit_patch_id` so the targeted filter selects them, and a zero-match filter never counts as passing.

## Task stack

- [x] T01: `Persist an optional commit patch ID in the local Agent Trace database` (status:done)
  - Task ID: T01
  - Scope: In — `007_commit_patch_id.sql` (`ALTER TABLE agent_traces ADD COLUMN commit_patch_id TEXT`), `AgentTraceInsert.commit_patch_id: Option<&str>`, `INSERT_AGENT_TRACE_SQL` and parameter binding, updating existing `AgentTraceInsert` call sites with `None`, migration/readiness tests, and the `agent-trace-db.md` context update including the listed drift repair. Out — Git computation, hook wiring beyond compiling call sites, export.
  - Dependencies: none
  - Done when: Fresh and pre-007 repository databases apply the nullable column through the setup/lifecycle migration path; existing rows and `trace_json` are retained with `commit_patch_id IS NULL`; repeated setup is idempotent; the hook-runtime open still refuses a pre-007 schema with `Run 'sce setup'.` guidance; a caller can insert `Some(id)` or `None` in the same Agent Trace insert.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` (migration/persistence cases).
  - Completed: 2026-10-15
  - Files changed: `cli/migrations/agent-trace-repository/007_commit_patch_id.sql` (new), `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/agent_trace_db/repository.rs`, `cli/src/services/agent_trace_storage/mod.rs`, `cli/src/services/hooks/commit_hooks.rs`, `cli/src/services/sync/sync.rs`, `context/sce/agent-trace-db.md`, `context/plans/agent-trace-commit-patch-id.md`
  - Result: Added additive migration `007_commit_patch_id` (`ALTER TABLE agent_traces ADD COLUMN commit_patch_id TEXT`); `AgentTraceInsert` gained `commit_patch_id: Option<&str>` bound as `?7` in `INSERT_AGENT_TRACE_SQL`; the post-commit writer and the sync test call site pass `None`. Three `commit_patch_id` tests in `agent_trace_storage` cover setup upgrade of a pre-007 fixture (test-only `DbSpec` with migrations `001`–`006`) preserving the seeded row, `trace_json`, and `source_instance_id` with `commit_patch_id IS NULL`, idempotent repeated setup with full ledger parity; hook-runtime refusal with `Run 'sce setup'.` and doctor `IncompatibleSchema` readiness without migrating; and `Some`/`None` inserts. Per user instruction, doc comments on touched and adjacent items (`INSERT_AGENT_TRACE_SQL` and neighbors, `AgentTraceInsert` and neighbors, `insert_agent_trace`, `open_repository_db_for_hook_runtime`) were removed; no new code comments were added.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id -- --list` lists 3 tests; the filtered run passed (3 passed, 0 failed). Additional: `... test agent_trace` passed (69 passed); `... test sync_uploads_only_three_streams_and_reports_zero_upload_diff_traces` passed (1 passed); `... clippy --all-targets -- -D warnings` clean.
  - Context impact: localized — `context/sce/agent-trace-db.md` updated (new column, migration `007` and upgrade/downgrade note, `AgentTraceInsert` field, drift repair for the `006` migration list, stale `015_add_diff_traces_payload_type` reference, and the hook-runtime lazy-initialization sentence).
  - Deviation: setup-path parity is tested through the private storage open functions rather than an end-to-end `sce setup` invocation.
  - Context synchronization: synced

- [ ] T02: `Compute the canonical Git patch ID with bounded subprocesses` (status:todo)
  - Task ID: T02
  - Scope: In — a small `hooks/` Git patch-ID helper module, the pinned `git show` options and config overrides, parent-count check for merges, streaming `git show` → `git patch-id --stable` with deadline, byte bounds, exit/output validation and `kill_on_drop` cleanup, the output parser, and temporary-Git-repository tests. Out — hook orchestration, persistence, sync.
  - Dependencies: none
  - Done when: The async helper takes a repository root and captured commit OID and returns `Result<Option<String>>`; normal, root, binary-only, and rename-only commits match the canonical invocation and are stable across message amend, equivalent cherry-pick, and user diff-configuration variations; empty and multi-parent merge commits return `Ok(None)`; non-zero exit, malformed output, oversized output, and timeout return bounded errors with no surviving child process.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` (helper cases).
  - Context synchronization: pending

- [ ] T03: `Attach the fingerprint to post-commit Agent Trace insertion` (status:todo)
  - Task ID: T03
  - Scope: In — `run_post_commit_agent_trace_flow` and its `run_post_commit_agent_trace_flow_with` test seam, fingerprint-to-insert wiring, best-effort sanitized warning, local storage tests, and the `agent-trace-hooks-command-routing.md` update documenting the exact pinned invocation. Out — changing Agent Trace JSON, changing intersection capture signatures, sync transport.
  - Dependencies: T01, T02
  - Done when: The already-captured `commit_oid` is fingerprinted asynchronously after the intersection flow and before the Agent Trace database insert; the stored row carries the nullable ID without any change to `trace_json`; fingerprint errors log a sanitized warning and persist `NULL`; Agent Trace build, validation, and persistence failures keep their existing behavior.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id` (hook/storage cases).
  - Context synchronization: pending

- [ ] T04: `Export commitPatchId through the existing agent_traces sync stream` (status:todo)
  - Task ID: T04
  - Scope: In — `AgentTraceAgentTraceExportRow.commit_patch_id: Option<String>`, `SELECT_AGENT_TRACES_AFTER_SQL` and `agent_trace_export_row_from_turso`, JSON serialization, captured HTTP sync assertions in the `sync.rs` tests, and the `agent-trace-export-readers.md` / `agent-trace-sync-command.md` updates (release dependency, no backfill, upgrade/downgrade). Out — sync-engine rewrite, Control Plane migration/API code, historical backfill.
  - Dependencies: T01, T03
  - Done when: The stored value is emitted as `commitPatchId` in the existing `agent_traces` ingestion batch; strings and nulls round-trip; `traceJson` remains raw; the three-stream protocol and authoritative server cursors are unchanged and pre-acknowledged rows are not resent; deployment and backfill limitations are documented.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_patch_id`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync_uploads_only_three_streams_and_reports_zero_upload_diff_traces`.
  - Context synchronization: pending

## Open questions

None. The plan limits implementation to the SCE CLI and uses the previously agreed optional `commitPatchId` contract; the Control Plane rollout dependency, setup-required upgrade window, and lack of historical backfill are explicit constraints rather than unresolved design questions.
