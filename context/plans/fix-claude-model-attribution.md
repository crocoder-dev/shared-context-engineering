# Plan: Fix Claude Model Attribution

## Change summary

Restore event-local Claude model enrichment for structured diff traces by resolving direct `PostToolUse` model metadata first and then, when direct metadata is absent, looking up the matching `tool_use_id` in the event's Claude JSONL transcript. The resolved value remains nullable, is normalized with the existing `claude/` convention, and is persisted only in `diff_traces.model_id`; the retired `session_models` table and runtime remain absent.

Also repair structured diff-trace reconstruction so every touched line receives the persisted canonical row session ID while each hunk retains the persisted row model ID. This lets downstream Agent Trace generation emit both `contributor.model_id` and canonical `cc_...` related-session URLs without changing OpenCode, Pi, the Agent Trace schema, or database migrations.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: A supported Claude `PostToolUse` event uses direct model metadata when present; otherwise it resolves the model from the real Claude assistant-message JSONL envelope by matching `tool_use.id` to `tool_use_id`, normalizes the result without double-prefixing, and leaves `model_id` null when lookup cannot succeed.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model`.
- [ ] AC2: Transcript lookup is fail-open for missing or unreadable files, unmatched tool calls, missing models, and malformed unrelated JSONL lines, and direct metadata always wins over transcript-derived metadata.
  - Validate: focused transcript and resolver unit tests cover all named branches and pass under the commands in AC1.
- [ ] AC3: Reconstructing a `payload_type="structured"` diff-trace row assigns the persisted `row.model_id` to every relevant hunk and the persisted canonical `row.session_id` to every touched line, without reusing the raw unprefixed session from the stored Claude payload.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml structured_diff_trace`.
- [ ] AC4: A Claude event with no direct model, a transcript match, and persisted `cc_...` session provenance produces Agent Trace output containing both `contributor.model_id` and a related resource with `type="session"` and the canonical `https://sce.crocoder.dev/sessions/cc_...` URL; existing OpenCode and Pi attribution behavior remains unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; existing OpenCode/Pi hook and Agent Trace regression tests pass in `nix flake check`.
- [ ] AC5: Current-state Agent Trace documentation describes direct-first/transcript-second/NULL Claude attribution as event-local enrichment, explicitly excludes `session_models` from runtime design, and records canonical persisted-session propagation during structured reconstruction.
  - Validate: inspect the focused Agent Trace hook, DB, patch, and generator context documents for those statements and confirm no current-state document describes an active `session_models` runtime.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/sce/agent-trace-hooks-command-routing.md` and `context/sce/agent-trace-db.md` for event-local direct-first/transcript-second Claude attribution and the explicit absence of `session_models` runtime behavior.
- Update `context/cli/patch-service.md`, `context/cli/structured-patch-service.md`, and `context/sce/agent-trace-minimal-generator.md` for persisted canonical session propagation and Agent Trace related-session generation.
- Refresh `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` where their current-state summaries or contracts would otherwise remain direct-only or omit the structured provenance rule.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/` transcript/model resolution, Claude structured diff-trace parsing, repository Agent Trace recent-row reconstruction, focused Rust tests, and current-state Agent Trace context documentation.
- **Out of scope:** DWH changes, historical backfill or repair, Agent Trace schema changes, unrelated migrations, and changes to OpenCode or Pi attribution semantics.
- **Constraints:** direct Claude model metadata must win; transcript lookup must use only the event's `transcript_path` plus `tool_use_id`, skip malformed unrelated lines where practical, normalize through the existing `claude/` convention, remain nullable/fail-open, and use no new database abstraction or table.
- **Non-goal:** restoring `session_models`, any session-level model cache/runtime, placeholder model IDs, or raw unprefixed Claude sessions in reconstructed touched-line provenance.

## Assumptions

- The focused transcript helper will live at `cli/src/services/hooks/claude_transcript.rs` and use the existing standard-library buffered file reading plus `serde_json`; no new dependency is needed.
- The historical implementation at `18afa0ac402134b132820a37f42e600cf6639644` is behavioral reference only; its whole-transcript failure on any malformed JSONL line is intentionally tightened to skip malformed unrelated lines.
- The existing repository DB test seams and public patch/Agent Trace builder APIs make the requested end-to-end regression practical without adding production abstractions.

## Task stack

- [x] T01: `Restore event-local Claude transcript model resolution` (status:done)
  - Task ID: T01
  - Scope: In — add the focused Claude JSONL transcript helper and unit tests; refactor Claude diff-trace model resolution to direct-first/transcript-second; retain existing nested direct fields and `claude/` normalization; cover missing/unreadable/malformed/unmatched inputs and direct precedence. Out — database schema/API changes, structured-row session propagation, OpenCode/Pi parser behavior, and session-level attribution storage.
  - Dependencies: none
  - Done when: supported Claude structured diff-trace payloads persist the direct normalized model when available, otherwise use the matching assistant transcript model, and otherwise keep `model_id=None` without rejecting the hook; all requested helper/resolver tests pass and no `session_models` runtime/API is introduced.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model`.
  - Context synchronization: synced
  - Context synchronization handoff: Changed files: `cli/src/services/hooks/claude_transcript.rs`, `cli/src/services/hooks/mod.rs`; Implementation summary: Added a buffered, fail-open Claude JSONL transcript reader that recognizes real wrapped assistant-message envelopes (while retaining flat-message compatibility), matches `tool_use.id` to the event's `tool_use_id`, skips malformed unrelated records, and returns no model for missing/unreadable/unmatched/missing-model inputs. Updated structured Claude diff-trace parsing to resolve existing direct model fields first and lazily fall back to the event's `transcript_path` plus `tool_use_id`, normalizing either source through the existing `claude/` convention. Added focused helper and resolver tests for the real envelope, malformed records, inaccessible and unmatched inputs, direct precedence, nested direct metadata, normalization, and nullable fallback.; Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript` (pass: 3 passed); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model` (pass: 3 passed).; Done checks: All satisfied — supported structured Claude payloads retain direct normalized models when present, otherwise use a matching transcript model, otherwise persist nullable model attribution without rejecting the payload; no dependency, database API, schema, or `session_models` runtime was added.; Context impact: domain — update current-state Agent Trace hook and DB context for event-local direct-first/transcript-second/NULL Claude model resolution and the continued absence of `session_models`; review the five root context files for stale direct-only summaries.
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/hooks/claude_transcript.rs`, `cli/src/services/hooks/mod.rs`
  - Result: Restored event-local Claude transcript model fallback with direct metadata precedence, existing model normalization, fail-open behavior, and focused regression coverage.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript` — pass (3 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model` — pass (3 tests).
  - Context impact: domain — synchronized current-state Agent Trace hook and DB documentation plus root summaries/patterns/glossary entries that previously described direct-only Claude model attribution.

- [ ] T02: `Preserve structured diff-trace model and session provenance` (status:todo)
  - Task ID: T02
  - Scope: In — update `parse_recent_diff_trace_patch_rows` so structured-row hunks retain persisted `model_id` and every touched line receives persisted canonical `session_id`; add focused reconstruction coverage and a practical persisted-row-to-Agent-Trace regression proving model plus canonical related session; preserve patch-row behavior. Out — schema/migration changes, backfill, Agent Trace schema changes, OpenCode/Pi attribution changes, and unrelated post-commit refactors.
  - Dependencies: T01
  - Done when: a persisted Claude structured row with `cc_session-123` and a Claude model reconstructs with that model on every relevant hunk and that canonical session on every touched line, and downstream Agent Trace output emits both model attribution and the canonical related-session URL; focused and existing regressions pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml structured_diff_trace`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
  - Context synchronization: pending

## Open questions

None. The request fixes a production attribution loss with explicit precedence, failure, persistence, provenance, compatibility, documentation, and validation contracts; the existing code and historical helper provide sufficient implementation seams without an architecture decision.
