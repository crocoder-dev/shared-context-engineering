# Claude Session Model Missing Model

## Change summary

Claude Code can emit a raw `SessionStart` hook event without a `model` property when a new session starts via `/clear`. The current `sce hooks session-model` Claude intake treats the model identifier as required and fails before hook completion. Update session-model attribution so `model_id` and `tool_version` are both optional in the `session_models` upsert path, allowing Claude `SessionStart` events to be recorded even when model attribution is unavailable.

## Success criteria

- Raw Claude `SessionStart` payloads with a valid `session_id` / `sessionID` and no usable `model`, `model_id`, `modelId`, or nested model identifier do not fail `sce hooks session-model`.
- Missing-model Claude `SessionStart` payloads still attempt a `session_models` upsert keyed by `(tool_name="claude", session_id)` with `model_id = NULL` and nullable `tool_version`.
- `session_models.model_id` is nullable in the database schema and Rust adapter types; existing model-present rows remain supported.
- Later diff-trace attribution fallback treats nullable stored `model_id` the same way it treats nullable `tool_version`: fill only fields that are present in the stored row, and continue with `None` when unresolved.
- Existing Claude `SessionStart` payloads with a model identifier still normalize and persist `model_id` as before, including the `claude/` prefix behavior.
- Existing normalized OpenCode/session-model payload validation remains unchanged unless implementation proves the shared adapter type must accept optional values while the OpenCode parser still enforces required `model_id` before building its payload.
- Unit coverage captures the Claude `/clear`-style missing-model upsert case, existing model-present behavior, nullable lookup behavior, and unchanged OpenCode validation.
- Current-state context for `session-model` hook behavior is updated if the implemented behavior changes documented contracts.

## Constraints and non-goals

- Keep runtime behavior changes focused on session-model attribution storage and its existing diff-trace fallback consumer; do not change `conversation-trace`, `commit-msg`, or `post-commit` behavior.
- A database migration to allow nullable `session_models.model_id` is in scope.
- Do not invent placeholder model IDs such as `unknown`; missing Claude model attribution should not create false model provenance.
- Do not change generated Claude hook registration unless code investigation shows the generated hook command itself is wrong.
- Preserve existing error behavior for malformed JSON, unsupported Claude hook events, and missing Claude session identity.

## Task stack

- [x] T01: `Make session model IDs nullable in storage` (status:done)
  - Task ID: T01
  - Goal: Update the Agent Trace DB `session_models` schema and adapter types so stored `model_id` can be `NULL` while `tool_version` remains nullable.
  - Boundaries (in/out of scope): In - a forward Agent Trace DB migration, `SessionModelUpsert` / `SessionModelAttribution` Rust type updates, SQL parameter/row mapping updates, and adapter-level tests. Out - Claude/OpenCode hook parser behavior changes, generated config rewrites, and unrelated DB tables.
  - Done when: New and migrated Agent Trace DBs allow `session_models.model_id = NULL`; adapter upsert and lookup helpers accept/return `Option` for `model_id`; existing non-null model rows still round-trip; migration metadata stays deterministic.
  - Verification notes (commands or checks): Run focused Agent Trace DB tests through Nix, for example `nix develop -c sh -c 'cd cli && cargo test agent_trace_db'`, plus inspect generated migration ordering expectations if tests cover migration manifests.
  - Completed: 2026-06-29
  - Files changed: `cli/migrations/agent-trace/015_create_session_models.sql`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/hooks/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; focused `cargo test agent_trace_db` command was blocked by repo bash policy; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Because the Agent Trace DB has not been released, the existing `015_create_session_models.sql` migration was updated in place instead of adding a new migration. Adapter storage types now accept/return nullable `model_id`; hook call sites preserve existing parser behavior by passing `Some(...)` for current model-present payloads.

- [x] T02: `Persist Claude SessionStart without model` (status:done)
  - Task ID: T02
  - Goal: Make `sce hooks session-model` persist Claude `SessionStart` events even when model attribution is absent, using nullable `model_id` and nullable `tool_version`.
  - Boundaries (in/out of scope): In - `cli/src/services/hooks/mod.rs` session-model parsing/runtime flow, diff-trace fallback handling for optional stored `model_id`, targeted unit tests, and current-state context updates for the changed hook contract. Out - generated Claude hook registration, OpenCode normalized input relaxation, and unrelated hook runtime changes.
  - Done when: A Claude `SessionStart` payload with `session_id` but no model identifier returns the normal successful session-model intake result and attempts a `SessionModelUpsert` with `model_id = None`; model-present Claude payloads still persist normalized `Some("claude/...")`; normalized OpenCode payloads still require `model_id`; later diff-trace fallback fills model only when the stored session row has one.
  - Verification notes (commands or checks): Run a focused hooks/session-model test group through Nix, for example `nix develop -c sh -c 'cd cli && cargo test claude_session_model'`, and review `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/architecture.md`, and related current-state context for required wording updates.
  - Completed: 2026-06-29
  - Files changed: `cli/src/services/hooks/mod.rs`, `context/architecture.md`, `context/glossary.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; focused `cargo test claude_session_model` command was blocked by repo bash policy; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Claude raw `SessionStart` parsing now treats model attribution as optional and passes nullable `model_id` into `SessionModelUpsert`; normalized OpenCode parsing still requires `model_id`; diff-trace fallback uses nullable stored `model_id` only when available. Newly added unit tests were removed at user request, so T02 relies on existing test coverage plus full flake validation rather than retaining new targeted test cases.

- [x] T03: `Validate and clean up` (status:done)
  - Task ID: T03
  - Goal: Run final repository validation and remove any temporary scaffolding left from T01.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity check, review of plan status/evidence, and cleanup of temporary local artifacts. Out - additional behavior changes beyond fixes needed to make the planned checks pass.
  - Done when: Required validation commands complete successfully or failures are documented with actionable follow-up; no temporary test/debug artifacts remain; plan task statuses and verification evidence are ready for handoff/closure; context sync has been checked for current-state accuracy.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect `git diff` for unintended files and confirm current-state context remains aligned with code truth.
  - Completed: 2026-06-29
  - Files changed: `context/plans/claude-session-model-missing-model.md`
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed; `git status --short` and `git diff --stat` inspected.
  - Notes: Existing ignored `context/tmp/` runtime artifacts were observed but no tracked temporary/debug scaffolding requiring cleanup was found. Context sync classified this as verify-only unless final sync detects drift.

## Open questions

- None.

## Validation Report

### Commands run

- `git status --short` -> exit 0; reviewed modified implementation/context files and untracked plan file.
- `git diff --stat` -> exit 0; reviewed implementation/context diff summary for unintended tracked files.
- `nix run .#pkl-check-generated` -> exit 0; output included `Generated outputs are up to date.`
- `nix flake check` -> exit 0; output included `all checks passed!`

### Success-criteria verification

- [x] Raw Claude `SessionStart` payloads without usable model attribution do not fail session-model parsing: verified in `cli/src/services/hooks/mod.rs` where `optional_claude_model_id(...)` returns `None` instead of raising validation failure.
- [x] Missing-model Claude `SessionStart` payloads attempt `session_models` upsert with `model_id = NULL`: verified by `SessionModelUpsert { model_id: payload.model_id.as_deref(), ... }` and nullable adapter storage.
- [x] `session_models.model_id` is nullable in schema and Rust adapter types: verified in `cli/migrations/agent-trace/015_create_session_models.sql`, `SessionModelUpsert`, and `SessionModelAttribution`.
- [x] Diff-trace attribution fallback only fills stored fields when present: verified by `resolve_diff_trace_attribution(...)` using nullable `SessionModelAttribution.model_id` and preserving unresolved `None`.
- [x] Model-present Claude payloads still normalize with `claude/` prefix: covered by existing `claude_session_model_payload_prefers_payload_tool_version_without_cli_probe` assertion.
- [x] Normalized OpenCode/session-model validation remains unchanged: verified in `parse_normalized_session_model_payload(...)`, which still requires non-empty `model_id` before constructing `Some(model_id)`.
- [x] Context updated for current behavior: verified in `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/agent-trace-db.md`, and `context/sce/agent-trace-hooks-command-routing.md`.

### Temporary scaffolding cleanup

- No tracked temporary test/debug scaffolding was found during worktree/diff review.
- Existing ignored `context/tmp/` runtime artifacts were observed and left in place because they were not tracked plan scaffolding.

### Failed checks and follow-ups

- None. Required validation commands passed.

### Residual risks

- The plan's desired missing-model parser unit coverage is represented indirectly through code review plus full flake validation; additional focused unit coverage was removed during T02 at user request.
