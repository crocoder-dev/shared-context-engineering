# Agent Trace Range Content Hash Implementation Plan

## Change summary

Add `content_hash` to each Agent Trace `ranges[]` entry emitted in `agent_traces.trace_json`. The JSON Schema already allows `range.content_hash` as an optional string; this change makes the Rust Agent Trace generator compute and emit it for every generated range.

The hash is a deterministic fingerprint of the attributed range content so downstream consumers can track content independently from line-number movement.

## Success criteria

- Every range emitted by `agent_trace::build_agent_trace(...)` includes a non-empty `content_hash` string.
- Hash format is `sha256:<lowercase-hex>`.
- Hash input is deterministic and position-independent:
  - sourced from the canonical `post_commit_patch` hunk used to emit the range;
  - based on touched attributed hunk lines in patch order;
  - excludes trace ID, timestamp, file path, line numbers, VCS revision, tool metadata, contributor/model metadata, and database IDs.
- Identical attributed touched content yields the same hash across traces; different touched content yields different hashes.
- Existing Agent Trace schema validation continues to pass.
- `agent_traces.trace_json` persistence stores the enriched payload without a database schema migration.
- Golden Agent Trace fixtures and relevant tests cover the new field.

## Constraints and non-goals

- Do not change `config/schema/agent-trace.schema.json` unless implementation discovers schema drift; the current schema already accepts `content_hash`.
- Do not add a separate Agent Trace DB column or migration for `content_hash`; it remains inside the persisted JSON payload.
- Do not change top-level Agent Trace payload version semantics unless tests reveal an explicit compatibility requirement.
- Do not read working-tree files only to compute hashes; use patch data already available to the generator.
- Do not include position-dependent values such as path or line numbers in the hash input.
- Do not change OpenCode plugin `diff-trace` intake shape for this slice.

## Assumptions

- The approved hash format is `sha256:<lowercase-hex>`.
- `content_hash` is emitted per range, not per trace payload.
- Every emitted range receives a `content_hash`, including `ai`, `mixed`, and `unknown` contributor classifications.
- “Attributed content” means the `post_commit_patch` hunk's touched lines that produced the emitted range, not the full final file range including unchanged context lines.
- The implementation may define a small canonical byte serialization for hash input, for example touched lines in order with line kind plus normalized line content, while still excluding line numbers and other positional metadata.

## Task stack

- [x] T01: `Add deterministic range content hashing helper` (status:done)
  - Task ID: T01
  - Goal: Add the internal helper/API needed to compute a range `content_hash` from a `PatchHunk` without wiring it into all emitted payloads yet.
  - Boundaries (in/out of scope): In - helper in the Agent Trace generator area, canonical hash input serialization, SHA-256 formatting as `sha256:<lowercase-hex>`, focused unit tests for determinism and line-number independence. Out - fixture-wide payload updates, DB changes, schema changes, plugin changes.
  - Done when: The helper returns stable hashes for equivalent touched content, changes when touched content changes, and ignores path/line-number/trace metadata by construction.
  - Verification notes (commands or checks): Targeted Rust test for the helper through Nix, e.g. `nix develop -c sh -c 'cd cli && cargo test content_hash'`; if no exact test filter fits after implementation, run the relevant `agent_trace` test filter.
  - Completed: 2026-05-21
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test content_hash'` was blocked by repo bash policy preferring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed.
  - Notes: Added `range_content_hash(...)` using versioned, length-delimited touched-line serialization plus `sha256:<lowercase-hex>` formatting; tests cover format, content sensitivity, and line-number/model metadata independence.

- [ ] T02: `Emit content_hash on Agent Trace ranges` (status:todo)
  - Task ID: T02
  - Goal: Extend `LineRange` serialization and `build_agent_trace(...)` range construction so each emitted range includes the computed `content_hash`.
  - Boundaries (in/out of scope): In - `LineRange` model update, hunk-to-range construction update, deleted embedded patch path behavior, golden fixture updates under `cli/src/services/agent_trace/fixtures/**/golden.json`, generator tests. Out - post-commit command routing changes beyond consuming the enriched existing payload, DB migrations, schema tightening.
  - Done when: All Agent Trace golden fixtures validate against the embedded schema and include `content_hash` on every range; generated payloads preserve existing contributor/range boundaries while adding the new field.
  - Verification notes (commands or checks): Run the Agent Trace generator tests through Nix, e.g. `nix develop -c sh -c 'cd cli && cargo test agent_trace'`; ensure schema-validation tests still pass.

- [ ] T03: `Add trace_json persistence regression coverage` (status:todo)
  - Task ID: T03
  - Goal: Cover the persisted post-commit Agent Trace JSON path so `agent_traces.trace_json` stores schema-valid ranges with `content_hash`.
  - Boundaries (in/out of scope): In - narrow Rust test coverage around the post-commit Agent Trace flow or AgentTraceDb insert boundary, asserting the serialized JSON contains range-level `content_hash` after schema validation. Out - new database columns, broader hook behavior changes, CLI output text changes unless existing tests require fixture updates.
  - Done when: A regression test fails if future code drops `content_hash` before persistence and passes without changing the DB schema.
  - Verification notes (commands or checks): Run the narrow hooks/AgentTraceDb test filter selected during implementation, then the generator test filter from T02 if needed.

- [ ] T04: `Sync Agent Trace context documentation` (status:todo)
  - Task ID: T04
  - Goal: Update current-state context docs to describe range-level `content_hash` emission and its hash contract.
  - Boundaries (in/out of scope): In - focused updates to Agent Trace context files such as `context/sce/agent-trace-minimal-generator.md`, `context/sce/agent-trace-hooks-command-routing.md`, and context-map/overview/glossary entries only if current-state summaries need adjustment. Out - historical reference docs unless they explicitly claim current runtime behavior, broad prose rewrites, completed-work narration.
  - Done when: Context accurately states that built Agent Trace payloads persisted in `trace_json` include per-range `content_hash` values computed from touched post-commit hunk content.
  - Verification notes (commands or checks): Manual context consistency review against implemented code; no application-code edits in this task except documentation/context updates.

- [ ] T05: `Final validation and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Run final repository validation and remove any temporary scaffolding from the implementation.
  - Boundaries (in/out of scope): In - full validation, generated-output parity check, formatting/lint/test surface, cleanup of temporary debug artifacts, final plan status/evidence updates. Out - new feature work or additional Agent Trace payload fields.
  - Done when: Required checks pass or any failures are documented with actionable follow-up; no temporary files remain; plan evidence is updated.
  - Verification notes (commands or checks): Preferred repo validation from root: `nix run .#pkl-check-generated` and `nix flake check`.

## Open questions

- None blocking. If implementation finds that deleted-file or embedded-patch ranges require a different canonical hash input than normal hunks, stop and confirm before changing the hash contract.
