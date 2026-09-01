# Plan: claude-model-attribution-state

## Change summary

Claude diff-trace attribution currently resolves `diff_traces.model_id` from direct
`PostToolUse` metadata, then from an exact `tool_use_id` lookup in the Claude JSONL
transcript, then `NULL`. Production data shows most Claude `diff_traces` rows are
`NULL` because Claude `PostToolUse` does not normally expose model identity and the
transcript is written asynchronously, so it is frequently unavailable at hook time.
The `remove-session-models-direct-claude-model-id` plan deliberately removed
session-level model state, and the `fix-claude-model-attribution` plan's
transcript fallback has not closed the gap.

This plan adds a small, Claude-specific latest-model-state register in the existing
repository-scoped Agent Trace DB (`<state-root>/sce/repos/<repository-id>/agent-trace.db`),
seeded from two Claude lifecycle signals — `SessionStart.model` and
`PostModelSwitch.from_model`/`to_model` — through a new silent hook command
`sce hooks claude-model-state`. Diff-trace persistence consults that state only when
direct and transcript attribution both fail, keeping the precedence
`direct > exact transcript > exact session/agent state > NULL`. The register is a
best-effort register in which the latest locally observed lifecycle information
wins: `observed_at_ms` is the local SCE time at which the hook observed the event,
not Claude's authoritative causal event order.
`diff_traces.model_id` stays the durable attribution result and the only model value
exported to the control plane. Claude invokes `SessionStart` synchronously relative
to Claude's execution and `PostModelSwitch` asynchronously relative to Claude's
execution. In both SCE handlers, the local database write is performed directly
before the hook process exits; SCE does not spawn, detach, background, or defer the
`claude_model_state` write.

This is a deliberate, narrower reintroduction of local model state. It does **not**
restore the generic cross-editor `session_models` table or the `sce hooks session-model`
command; the new state is Claude-specific because both the missing-attribution
problem and the lifecycle API are Claude-specific. It adds no sync stream, no
control-plane endpoint, no ClickHouse schema change, and no historical backfill. An
accompanying decision record supersedes only the earlier "no session-level cache"
constraint for Claude model attribution, leaving the historical plans unchanged.

### Claude attribution flow and ephemeral context

For Claude structured diff-trace events, the internal parsed representation carries
`agent_id: Option<String>` (or an equivalent typed ephemeral attribution context).
The value is extracted from the raw hook event when present and is used only for the
local state lookup; it is not part of the `diff_traces` schema, export payload,
Control Plane data, or persisted raw-event reparsing path. Main-session events use
`None` internally and map it to `agent_id = ""` only for state lookup/storage.

```text
Claude PostToolUse
    |
    v
parse Claude event
    |
    +-- model_id: direct -> transcript -> None
    |
    +-- agent_id: optional ephemeral context
    |
    v
open RepositoryAgentTraceDb once
    |
    +-- model already resolved? use it
    |
    +-- otherwise lookup:
        (canonical cc_session_id, exact agent_id scope)
    |
    v
DiffTraceInsert
```

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: A decision record `context/decisions/2026-09-01-claude-model-attribution-state.md` exists, records the production evidence, the transcript-timing limitation, the new `PostModelSwitch` capability, the rejected generic `session_models` restoration, that `observed_at_ms` is local SCE observation time rather than Claude's causal event order, the best-effort latest-locally-observed register semantics, the distinct rapid-consecutive-switch and post-switch visibility races, the absence of an upstream sequence/timestamp or synchronization barrier in this hook contract, the local-only/no-export scope, and the specific one-turn fallback-chain model substitution limitation, and explicitly supersedes the earlier "no session-level cache" constraint for Claude model attribution without editing the historical plans.
  - Validate: inspect the file for each listed element and a `Supersedes` reference to the prior constraint; verify the historical plans are unchanged relative to the PR base:
    ```sh
    BASE="$(git merge-base HEAD origin/main)"

    git diff --exit-code "$BASE"..HEAD -- \
      context/plans/remove-session-models-direct-claude-model-id.md \
      context/plans/fix-claude-model-attribution.md
    ```
- [ ] AC2: Additive migration `003_claude_model_state.sql` creates the `claude_model_state` table with primary key `(session_id, agent_id)`, `observation_kind` constrained to `session_start | post_model_switch`, and `observed_at_ms >= 0`; `001_repository_schema.sql` and `002_repository_source_instance_id.sql` are byte-unchanged; `001 -> 002 -> 003` applies cleanly on a fresh DB and on a pre-`003` DB.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `git diff --stat` shows no change to `001`/`002`.
- [ ] AC3: The repository adapter exposes typed guarded read/write helpers for `claude_model_state`, and no export/sync module (`agent_trace_export`, `sync`) references the table or a model-state cursor.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `rg -n "claude_model_state" cli/src/services/agent_trace_export cli/src/services/sync` returns nothing.
- [ ] AC4: `SessionStart` with a model persists normalized `(cc_<id>, "", claude/<model>, session_start, <source>, <observed_at_ms>)`; `SessionStart` without a model (or empty/null) is a silent no-op that never deletes, nulls, or replaces existing state; canonical `cc_` identity is idempotent; the main conversation uses `agent_id = ""`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`.
- [ ] AC5: `PostModelSwitch` persists `to_model` (normalized) as current state with `observation_kind = post_model_switch`; `from_model`/`to_model` are validated/normalized, with `from_model` used only as validation/diagnostic information and never as a compare-and-swap precondition; sources `command`, `picker`, `sdk`, `auto`, `resume` are all accepted; `observed_at_ms` is local SCE observation time, not Claude's causal event timestamp; a strictly older local observation never overwrites a newer local observation, making this a best-effort latest-locally-observed register rather than a correctness guarantee about switch order; replayed identical observations are idempotent; equal-timestamp conflicts resolve deterministically (PostModelSwitch beats SessionStart, with a stable non-arrival-order tie-break for same-kind observations); the ordering/clock helper is injectable in tests.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model`.
- [ ] AC6: `sce hooks claude-model-state` writes zero bytes to stdout on every branch (successful write, no-op, malformed input, DB-open failure, DB-write failure), fails open, never returns exit code 2 or denies Claude activity, performs only local DB work with no network access or auto-sync, and routes all diagnostics through the existing logger. Claude invokes `SessionStart` synchronously relative to Claude's execution and `PostModelSwitch` asynchronously relative to Claude's execution; in both SCE handlers, the local database write is performed directly before the hook process exits. `SessionStart` remains minimal and fast so initial state is persisted before subsequent tool activity where practical; `PostModelSwitch` handling respects Claude's lifecycle semantics, including possible overlapping hook executions, without requiring SCE to serialize them or prove their causal order.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state` (stdout-byte assertions per branch); inspect the handler for any `println!`/stdout writer and any exit-code-2 path.
- [ ] AC7: `parse_claude_diff_trace_payload` performs no database access and keeps its `direct -> transcript -> None` result; for Claude structured events, the internal parsed diff-trace representation carries `agent_id: Option<String>` (or an equivalent ephemeral typed context) extracted from the raw event when present, without adding it to the external payload or persisted schema; unsupported/no-op Claude events reach no DB access; a valid Claude diff trace whose parser model is unresolved performs exactly one `claude_model_state` lookup after the repository DB is already open, and the resolved value flows into `DiffTraceInsert.model_id`; direct and transcript attribution still win over state.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; inspect `parse_claude_diff_trace_payload` for storage calls; confirm the diff-trace path opens the repository DB once.
- [ ] AC8: State lookup for a diff trace uses the canonical `(cc_<session_id>, exact agent_id scope)` from the event; `agent_id` is used only for resolving `claude_model_state`, with main-session `None` mapped to `""` for lookup/storage. A subagent (`agent_id != ""`) with no state for that exact pair resolves to `NULL` and never falls back to `(cc_<session_id>, "")`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
- [ ] AC9: Canonical Pkl generates `SessionStart` and `PostModelSwitch` registrations for `sce hooks claude-model-state` in `.claude/settings.json` while preserving the existing `PreToolUse Bash`, `PostToolUse` diff-trace/conversation-trace, `UserPromptSubmit`, and `Stop` registrations; `sce setup` merge adds both SCE lifecycle hooks, preserves user-owned hooks on those same events and unrelated settings, replaces stale SCE-owned model-state commands, and does not duplicate identical SCE registrations; `sce doctor` reports an installed Claude config missing either SCE lifecycle hook as drift and `sce doctor --fix` repairs it through the existing config-merge path.
  - Validate: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_merge`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`.
- [ ] AC10: Migration `003` is applied only through `sce setup` / lifecycle setup; no hook runtime path (including `sce hooks claude-model-state`) runs any migration; a pre-`003` repository DB surfaces the existing schema-not-ready `Run 'sce setup'.` guidance from hook paths rather than migrating; `sce doctor` diagnoses the incomplete schema; an upgrade note tells Claude-attribution repositories to rerun `sce setup`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`; inspect the hook handler and `resolve_agent_trace_storage_for_hook_runtime` usage for no migration call; confirm the upgrade note lands in `context/`.
- [ ] AC11: End-to-end, with direct and transcript attribution unavailable: `SessionStart(A)` then a model-less Claude `PostToolUse` persists `diff_traces.model_id = claude/A`; after the `PostModelSwitch(A->B)` local DB write has completed and state `B` is persisted, a model-less `PostToolUse` persists `claude/B`; a direct model `C` persists `claude/C` regardless of state `B`; a transcript model `C` persists `claude/C` regardless of state `B`; no direct, no transcript, no state persists `NULL`; a subagent `PostToolUse` with no subagent state does not receive the parent's `B`; OpenCode, Pi, and Codex diff-trace attribution behavior is unchanged. The test/design notes must not claim to prove Claude's real asynchronous scheduling order or that the first post-switch `PostToolUse` sees `B` before the state write completes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; existing OpenCode/Pi/Codex hook and Agent Trace regression tests pass under `nix flake check`.
- [ ] AC12: `AgentTraceExportReader` still exposes exactly the four existing streams (messages, parts, diff_traces, agent_traces); sync state/API types gain no model-state cursor; `diff_traces.model_id` export carries the resolved value with no control-plane protocol change.
  - Validate: `nix flake check`; `rg -n "claude_model_state|model.?state.?cursor" cli/src/services/agent_trace_export cli/src/services/sync` returns nothing.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

Claude Code compatibility smoke (release-level, in addition to repository tests):
a Claude Code build that supports `PostModelSwitch` captures state on a real model
switch, and one immediately older supported Claude Code build does not lose the
existing SCE hooks or the whole `.claude/settings.json` when it encounters the
unknown `PostModelSwitch` registration.

### Context sync

- `context/sce/agent-trace-db.md` — add `claude_model_state` table, migration `003`, and the typed guarded read/write helpers; state it is not exported.
- `context/sce/agent-trace-hooks-command-routing.md` — add `sce hooks claude-model-state`, Claude's synchronous `SessionStart` and asynchronous `PostModelSwitch` invocation semantics, the direct local database write before either SCE hook process exits, the zero-stdout / fail-open / no-exit-2 / local-only contract, the post-switch visibility race and no-convergence-wait behavior, and the diff-trace `direct > transcript > state > NULL` precedence with ephemeral agent context and subagent isolation.
- `context/sce/claude-raw-hook-capture.md` — note that `SessionStart` is registered again (for model state only, not raw capture) and `PostModelSwitch` is newly registered.
- `context/sce/agent-trace-hook-doctor.md` — add the missing SCE lifecycle-hook drift check and its `--fix` path.
- `context/sce/agent-trace-export-readers.md` — state `claude_model_state` is outside the export boundary.
- `context/architecture.md`, `context/overview.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md` — update the Claude model-attribution summary from direct/transcript-only to include the local state fallback; add glossary entries for `claude_model_state` and `sce hooks claude-model-state`; add the migration-`003` upgrade note (rerun `sce setup`).
- Add `context/decisions/2026-09-01-claude-model-attribution-state.md`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-repository/003_claude_model_state.sql`; `cli/src/services/agent_trace_db/` (state structs, guarded update/query helpers, repository adapter delegation); `cli/src/services/hooks/` (new `claude-model-state` subcommand, CLI schema, `HookSubcommand`, parse/runtime conversion, injectable clock seam, diff-trace persistence-boundary lookup); `cli/src/cli_schema.rs` / `cli/src/services/parse/command_runtime.rs`; `config/pkl/renderers/claude-content.pkl` and regenerated Claude settings; `cli/src/services/setup/config_merge.rs` and doctor inspect/lifecycle only where the new events need coverage; focused Rust tests; the listed current-state context files and the new decision record.
- **Out of scope:** control-plane / ClickHouse schema, sync worker or cursor changes, `AgentTraceExportReader` streams, historical `model_id` backfill, OpenCode / Pi / Codex attribution behavior, the retired generic `session_models` table and `sce hooks session-model` command, any TypeScript Claude translation runtime.
- **Constraints:** hooks never run migrations (`agent_trace_hook_no_hot_path_migrations`); Claude parsing must not depend on database access; `sce hooks claude-model-state` must write zero stdout bytes, fail open, never exit 2 or deny Claude activity, perform only local DB work, and perform no network access or auto-sync; Claude invokes `SessionStart` synchronously relative to Claude's execution and `PostModelSwitch` asynchronously relative to Claude's execution, and both SCE handlers perform the local database write directly before the hook process exits; SCE must not spawn, detach, background, or defer the `claude_model_state` write; state is a per-`(session_id, agent_id)` latest-value register guarded by `observed_at_ms`, not an event log, where `observed_at_ms` is local SCE observation time and the register is only a best-effort latest-locally-observed result, never a proof of Claude causal event order; strictly older local observations are rejected and equal-time outcomes use the PostModelSwitch-over-SessionStart rule plus a stable non-arrival-order tie-break for same-kind observations; `model_id` normalizes through the existing `claude/` convention; no fabricated `unknown` model; transcript lookup stays ahead of state and is not removed; a model-less `SessionStart` never clears state; Claude structured diff traces carry `agent_id: Option<String>` only as ephemeral context for exact state lookup, mapping main-session `None` to stored `""`; do not add `agent_id` to `diff_traces`, exports, or Control Plane, and do not reparse stored raw Claude JSON during persistence; OpenCode / Pi / Codex payload behavior remains unchanged; new deps pinned exactly, newest Node runtime for any new JS/TS work (`feedback_deps.md`) — though this plan expects no JS/TS changes.
- **Non-goal:** claiming the state fallback cannot detect a one-turn fallback-chain model substitution when Claude exposes that substitution neither through lifecycle state nor through the exact transcript lookup; generalizing the register into a cross-editor session-attribution architecture; using parent-session state as subagent state.

## Assumptions

- Claude Code emits `SessionStart` with an optional `model` field and `PostModelSwitch` with `from_model`, `to_model`, and `source` fields, and includes `agent_id` when a hook fires inside a subagent. `PostModelSwitch` is available from Claude Code 2.1.251. These are taken from the change request; the exact minimum-version policy is settled by the compatibility smoke in T05. Claude invokes `SessionStart` synchronously relative to Claude's execution and `PostModelSwitch` asynchronously relative to Claude's execution, so hook executions can overlap; in both SCE handlers, the local database write is performed directly before the hook process exits. Claude does not currently expose an authoritative sequence number or event timestamp through this hook contract.
- The decision record is authored as T01 because it is the explicit premise of this change (superseding a recorded constraint); if the `/next-task` synchronization decision gate is the preferred mechanism, it reuses the active record rather than writing a second one.
- `agent_id = ""` is the canonical scope for the main Claude conversation; the empty string (not `NULL`) is stored so the primary key stays total.
- The existing `config_merge` per-event SCE-owned filtering (commands containing `run-sce-or-show-install-guidance.sh`) covers `SessionStart`/`PostModelSwitch` without new merge logic; T05 adds coverage, not a new mechanism.
- `observed_at_ms` is captured in Rust immediately after hook STDIN is read, via an injectable clock seam, and means local SCE observation time. Tests control the guarded local-observation comparison; it cannot establish Claude's causal switch order.

## Task stack

- [x] T01: `Record the Claude attribution-state decision` (status:done)
  - Task ID: T01
  - Scope: In — write `context/decisions/2026-09-01-claude-model-attribution-state.md` capturing production evidence, transcript-timing limitation, the `PostModelSwitch` capability, rejected generic `session_models` restoration, local SCE observation-time semantics (not Claude causal ordering), best-effort latest-locally-observed behavior, the distinct rapid-consecutive-switch and post-switch visibility races, the absence of a synchronization barrier or upstream ordering metadata, local-only/no-export scope, and the specific one-turn fallback-chain model substitution limitation; state that it supersedes only the earlier "no session-level cache" constraint for Claude model attribution. Out — any runtime code, migration, or edits to the historical plan files.
  - Dependencies: none
  - Done when: the decision file exists in the ADR format, distinguishes Claude-specific current state from the retired generic `session_models` abstraction, names its `Supersedes` target, and the two historical plans are untouched.
  - Verify: inspect the file against AC1; `git status --short` allows only `context/decisions/2026-09-01-claude-model-attribution-state.md` and `context/plans/claude-model-attribution-state.md` (the active plan may record the task/context-sync transition), and confirms `context/plans/remove-session-models-direct-claude-model-id.md` and `context/plans/fix-claude-model-attribution.md` are untouched.
  - Completed: 2026-09-01
  - Files changed: `context/decisions/2026-09-01-claude-model-attribution-state.md`
  - Result: Added the accepted Claude-specific attribution-state decision, preserving direct and transcript precedence while defining local observation-time, best-effort register semantics and its timing limitations.
  - Verify: ADR inspection passed against AC1; the historical plans were verified unchanged relative to the PR base with:
    ```sh
    BASE="$(git merge-base HEAD origin/main)"

    git diff --exit-code "$BASE"..HEAD -- \
      context/plans/remove-session-models-direct-claude-model-id.md \
      context/plans/fix-claude-model-attribution.md
    ```
    Baseline-relative comparison found only the new ADR changed before this plan record.
  - Done checks: Decision file exists in repository ADR format; distinguishes Claude-specific state from retired generic `session_models`; names the superseded no-session-level-cache constraint; records all required production evidence, lifecycle, race, ordering, scope, and fallback limitations; historical plans remain untouched.
  - Context impact: cross-cutting decision — establishes the bounded Claude-specific local-state exception to the prior attribution constraint; synchronization must reconcile the decision record and inspect the mandatory root context files before another task starts.
  - Context synchronization: synced

- [ ] T02: `Add claude_model_state table and typed guarded helpers` (status:todo)
  - Task ID: T02
  - Scope: In — `003_claude_model_state.sql` additive migration; `RepositoryAgentTraceDbSpec` migration list picks it up; state structs (`ClaudeModelStateObservation`, `ObservationKind`); guarded upsert using local `observed_at_ms` (best-effort latest-locally-observed guard, PostModelSwitch-over-SessionStart plus a stable same-kind equal-time tie-break, replay idempotence) and exact-`(session_id, agent_id)` query helpers on `RepositoryAgentTraceDb`; migration/concurrency tests including `001 -> 002 -> 003` on fresh and pre-`003` DBs. Out — hook routing, diff-trace lookup, generated settings, export/sync wiring.
  - Dependencies: T01
  - Done when: setup-created and upgraded repository DBs persist and read Claude latest-model state deterministically; `001`/`002` are byte-unchanged; no export/sync module references the table.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `git diff --stat cli/migrations/agent-trace-repository/`.
  - Context synchronization: pending

- [ ] T03: `Add silent sce hooks claude-model-state intake` (status:todo)
  - Task ID: T03
  - Scope: In — `claude-model-state` in `cli_schema.rs`, `command_runtime.rs`, `HookSubcommand`, and `run_hooks_subcommand`; raw `SessionStart`/`PostModelSwitch` parsing, validation, `cc_`/`claude/` normalization, optional ephemeral `agent_id` handling; injectable clock seam for local `observed_at_ms`; fail-open logging through the existing logger; zero stdout bytes, no exit 2, no denial, local-only/no-network/no-sync behavior on every branch; Claude invokes `SessionStart` synchronously relative to Claude's execution and `PostModelSwitch` asynchronously relative to Claude's execution, and both SCE handlers perform the local database write directly before the hook process exits; keep the work minimal/fast and respect Claude's `PostModelSwitch` lifecycle semantics, including overlapping hook executions; state becomes visible when the PostModelSwitch local DB write completes, not when Claude's model switch occurs, and no causal ordering is asserted; SCE must not spawn, detach, background, or defer the write; focused tests for SessionStart (model / missing model / cleared-state protection), PostModelSwitch (to_model wins, `from_model` validation/diagnostics but no strict CAS, source variants, local-observation guard, replay idempotence, equal-time), and stdout-byte assertions. Out — diff-trace persistence lookup, generated settings, doctor/setup merge.
  - Dependencies: T02
  - Done when: Claude invokes `SessionStart` synchronously and `PostModelSwitch` asynchronously relative to Claude's execution, both lifecycle events perform direct guarded state writes before their SCE hook processes exit with no stdout bytes, PostModelSwitch respects Claude's lifecycle semantics, and a pre-`003` DB surfaces `Run 'sce setup'.` guidance without migrating.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model`.
  - Context synchronization: pending

- [ ] T04: `Consult claude_model_state at diff-trace persistence` (status:todo)
  - Task ID: T04
  - Scope: In — after `parse_claude_diff_trace_payload` returns a valid Claude diff trace with unresolved model and after the repository DB is already open, perform exactly one exact `(canonical cc_<session_id>, agent_id)` `claude_model_state` lookup and pass any result into `DiffTraceInsert.model_id`; extend the internal Claude parsed representation with optional ephemeral `agent_id`, extracting it from the raw event, mapping main-session `None` to `""`, and never serializing/storing/exporting it; preserve `direct -> transcript -> None` in the parser with no DB access; keep unsupported/no-op Claude events DB-free; do not poll, sleep, retry waiting for model state, or wait for another hook process; do not reparse stored raw Claude JSON; preserve subagent isolation (no fallback to `agent_id = ""`); persisted-row and precedence tests, plus OpenCode/Pi/Codex regression coverage. Out — schema/export changes, backfill, changes to direct/transcript resolution.
  - Dependencies: T02, T03
  - Done when: `direct > transcript > state > NULL` precedence and subagent isolation are proven on persisted rows, the parser still queries no storage, and one diff trace opens the repository DB once.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
  - Context synchronization: pending

- [ ] T05: `Register and merge Claude lifecycle hooks safely` (status:todo)
  - Task ID: T05
  - Scope: In — `config/pkl/renderers/claude-content.pkl` adds `SessionStart` and `PostModelSwitch` command hooks routing to `sce hooks claude-model-state`; regenerate canonical outputs; config-merge tests (adds both, preserves user hooks/settings, replaces stale SCE model-state commands, no duplicate SCE registrations); doctor drift + `--fix` tests for a config missing either SCE lifecycle hook; run the pre/post-`PostModelSwitch` Claude Code compatibility smoke and, per its result, either document a raised minimum Claude Code version or capability-gate `PostModelSwitch` installation. Out — OpenCode/Pi/Codex generation, diff-trace lookup, migration/state code.
  - Dependencies: T03
  - Done when: `nix run .#pkl-check-generated` passes with the two new registrations and the existing five preserved, setup merge is non-destructive, doctor flags and fixes the drift, and the compatibility policy is explicitly recorded (not assumed).
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_merge`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`.
  - Context synchronization: pending

- [ ] T06: `Add end-to-end attribution regression and upgrade note` (status:todo)
  - Task ID: T06
  - Scope: In — real-shaped fixtures and a lifecycle-then-`PostToolUse` regression covering every AC11 scenario (SessionStart-seeded, PostModelSwitch-updated after its local write completes, direct-wins, transcript-wins, all-absent NULL, subagent isolation) asserting persisted `diff_traces.model_id`; test the stable state-after-persistence contract without simulating an unsupported global ordering guarantee; confirm existing Agent Trace / OpenCode / Pi / Codex attribution regressions still pass; add the migration-`003` upgrade note ("rerun `sce setup`") to a durable context file. Out — new product behavior, export/sync changes.
  - Dependencies: T04, T05
  - Done when: all AC11 scenarios pass as persisted-row assertions, unrelated producer behavior is unchanged, and the upgrade note is in `context/`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; `nix flake check`.
  - Context synchronization: pending

## Residual risks

- **Post-switch visibility race:** Claude invokes `PostModelSwitch` asynchronously relative to Claude's execution, while SCE performs the local database write directly before the hook process exits. Claude may continue before the asynchronous PostModelSwitch hook process has completed its local write, because Claude does not wait for that process. A `PostToolUse` may therefore arrive while the corresponding lifecycle hook is still running. If direct and transcript attribution are unavailable, that event may use stale previous state or `NULL`; subsequent events use the new state after the lifecycle hook finishes. SCE will not spawn, detach, background, or defer the `claude_model_state` write, poll, sleep, retry waiting for model-state convergence, or delay diff-trace persistence waiting for another hook process. This is an upstream lifecycle-timing limitation; `observed_at_ms` does not solve it and the current hook contract cannot eliminate it.
- **Rapid consecutive switch ordering:** Rapid consecutive switches such as `A -> B` followed by `B -> C` can launch overlapping `PostModelSwitch` hook executions. `observed_at_ms` records when SCE locally observes each event (before persistence), so the guarded register can reject an obviously older local observation and resolve equal times deterministically, but it cannot establish Claude's causal switch order. Perfect ordering requires an upstream sequence number or authoritative event timestamp, which Claude does not currently expose through this hook contract.
- A supported Claude Code build that does not emit `PostModelSwitch` leaves the register dependent on `SessionStart` and therefore stale after an in-session switch; the compatibility policy must state this degraded mode.
- The state fallback is best-effort and cannot detect a one-turn fallback-chain model substitution when Claude exposes that substitution neither through lifecycle state nor through the exact transcript lookup.

## Open questions

- Claude Code forward-compatibility with an unknown `PostModelSwitch` hook registration is unverified. If an immediately pre-2.1.251 build treats the unknown event as fatal to `.claude/settings.json`, T05 must land strategy A (document and raise SCE's supported Claude Code floor to 2.1.251) or strategy B (capability-gate the `PostModelSwitch` registration) rather than unconditional registration. This does not block planning — both branches are scoped into T05 — but it can turn into a user-facing minimum-version policy change.
- This is the third plan to churn Claude diff-trace model attribution in roughly two months (`remove-session-models-direct-claude-model-id`, then `fix-claude-model-attribution`, now this). The premise is genuinely different — production `NULL` evidence plus a lifecycle event that did not exist before — and the plan deliberately avoids restoring the generic abstraction. Flagged rather than doubted: if `PostModelSwitch` adoption is slow, main-session state is seeded only by `SessionStart` and goes stale after an in-session `/model` switch on older clients, which is the degraded mode the compatibility note must state plainly.
