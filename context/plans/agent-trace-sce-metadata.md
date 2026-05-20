# Agent Trace SCE Metadata

## Change summary

Reimplement the previously dropped Agent Trace metadata change after git conflict resolution. Add implementation-specific SCE metadata to every built Agent Trace payload before it is serialized into `agent_traces.trace_json` by the post-commit flow.

The target payload shape is a top-level `metadata` object accepted by `config/schema/agent-trace.schema.json`:

```json
{
  "metadata": {
    "sce": {
      "version": "0.2.0"
    }
  }
}
```

Source `metadata.sce.version` from the compiled CLI package version (`env!("CARGO_PKG_VERSION")`), not by reading `.version` at runtime. In the current checkout, `cli/Cargo.toml` has `version = "0.2.0"`, and release/package parity checks keep it aligned with the repo-root `.version` authority. This keeps generated trace JSON deterministic, packaged-binary friendly, and independent from checkout-time files.

Read-only plan review found the conflict-dropped baseline currently lacks this metadata in `cli/src/services/agent_trace.rs`, while `config/schema/agent-trace.schema.json` already permits a generic top-level `metadata` object.

Optional future metadata ideas remain out of scope for this slice:

- `metadata.sce.generator`: stable identifier for the SCE trace generator path, if downstream consumers need to distinguish local-hook vs hosted ingestion.
- `metadata.sce.agent_trace_payload_version`: copy of top-level Agent Trace `version`, only if consumers need vendor metadata to be self-contained.
- `metadata.sce.commit_window_days` or similar runtime window metadata, only if trace consumers need to explain the bounded 7-day diff-trace selection policy.

## Success criteria

- Built Agent Trace structs serialize with top-level `metadata.sce.version` equal to the compiled `sce` CLI package version.
- Post-commit `agent_traces.trace_json` rows receive the enriched payload because the enrichment happens in `agent_trace::build_agent_trace(...)` before schema validation and DB insertion.
- Existing Agent Trace schema validation continues to pass using the embedded schema's generic `metadata` object allowance.
- Unit/golden fixture coverage proves the metadata field is present and stable.
- Current-state context docs describing the Agent Trace generator and DB payload are updated after implementation.

## Constraints and non-goals

- Do not add a new Agent Trace DB migration; `trace_json` is already persisted as JSON text.
- Do not change top-level Agent Trace spec `version` semantics; `AGENT_TRACE_VERSION` remains the Agent Trace payload/schema version, while `metadata.sce.version` is the SCE CLI implementation version.
- Do not add extra metadata fields in this implementation beyond `metadata.sce.version` unless explicitly approved later.
- Do not read `.version` from disk at runtime.
- Preserve deterministic serialization and existing schema-validation behavior.
- Do not broaden hook command behavior or OpenCode plugin behavior.

## Task stack

- [x] T01: `Add SCE metadata to generated Agent Trace payloads` (status:done)
  - Task ID: T01
  - Goal: Extend `cli/src/services/agent_trace.rs` so `build_agent_trace(...)` includes top-level `metadata.sce.version` sourced from the compiled CLI package version.
  - Boundaries (in/out of scope): In - Agent Trace domain structs/helpers, `AgentTrace` serialization shape, builder assignment, existing agent-trace tests, and golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json`. Out - DB schema migrations, hook command behavior changes, OpenCode plugin changes, extra metadata keys, runtime `.version` file reads, and release/version bump work.
  - Done when: Serialized Agent Trace JSON contains `metadata: { sce: { version: env!("CARGO_PKG_VERSION") } }`; existing schema validation accepts the enriched payload; fixture/golden expectations and direct assertions cover the new field.
  - Verification notes (commands or checks): Prefer targeted agent-trace Rust coverage during implementation if allowed by policy, then run repo-preferred `nix flake check` before handoff.
  - Completed: 2026-05-20
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/**/golden.json`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed. Targeted `nix develop -c sh -c 'cd cli && cargo test agent_trace'` was attempted first and blocked by repo bash policy `use-nix-flake-check-over-cargo-test`, so repo-preferred flake verification was used.
  - Context-sync classification: localized Agent Trace payload contract change; current-state Agent Trace context docs require sync, while root shared docs are expected to be verify-only unless code truth reveals broader drift.

- [x] T02: `Sync Agent Trace metadata context` (status:done)
  - Task ID: T02
  - Goal: Update current-state context to document the new metadata contract and version-source rationale after code truth changes.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-minimal-generator.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, and glossary entries if the `AgentTrace`/`build_agent_trace` current-state contract changes. Out - historical Agent Trace reference docs unless they are incorrectly marked current-state, broad root-context churn, and unrelated Agent Trace cleanup.
  - Done when: Current-state context states that generated Agent Trace payloads include `metadata.sce.version` from the compiled SCE CLI package version before schema validation and DB persistence.
  - Verification notes (commands or checks): Review context references against code truth; use verify-only handling for root `overview.md`, `architecture.md`, and `patterns.md` unless implementation changes architecture or terminology beyond this localized payload addition.
  - Completed: 2026-05-20
  - Files changed: `context/sce/agent-trace-minimal-generator.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, `context/overview.md`, `context/glossary.md`
  - Evidence: Code truth reviewed in `cli/src/services/agent_trace.rs`, `cli/src/services/hooks/mod.rs`, and `cli/src/services/agent_trace_db/mod.rs`; context grep for `metadata.sce.version` / compiled package version returned the expected current-state references; `git diff --check` passed.
  - Context-sync classification: localized Agent Trace payload documentation update; root `architecture.md` and `patterns.md` were verify-only, while `overview.md` already carries a concise current-state hook-runtime summary.

- [x] T03: `Validate and clean up Agent Trace metadata change` (status:done)
  - Task ID: T03
  - Goal: Run final validation for the reimplemented metadata change and remove any temporary scaffolding.
  - Boundaries (in/out of scope): In - full repository validation, generated-output parity check, whitespace checks if available, final plan evidence capture, and temporary/debug artifact review. Out - unrelated refactors, additional Agent Trace enrichment, release/version bump work, and new persistence migrations.
  - Done when: Required validation commands pass or failures are documented with actionable follow-up; no task-introduced temporary files or debug-only code remain; context and plan status reflect the final state.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; whitespace/diff checks as available in the implementation session.
  - Completed: 2026-05-20
  - Files changed by T03: `context/plans/agent-trace-sce-metadata.md`
  - Evidence: `git diff --check` passed before and after validation; `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."; `nix flake check` passed with all checks passed.
  - Cleanup review: Worktree review found no task-introduced tracked temporary files or debug-only code. Ignored `context/tmp/` hook/runtime artifacts existed and were left untouched because they are not T03 scaffolding.
  - Context-sync classification: final validation/status update only; no root context edits expected unless `sce-context-sync` finds drift.

## Open questions

- None blocking. Proposed additional metadata keys are deferred until explicitly requested.

## Validation Report

### Commands run

- `git diff --check` -> exit 0; no whitespace errors reported.
- `nix run .#pkl-check-generated` -> exit 0; reported `Generated outputs are up to date.`.
- `nix flake check` -> exit 0; evaluated packages, checks, apps, and dev shell; reported `all checks passed`.

### Cleanup and context verification

- No task-introduced tracked temporary files or debug-only code were found.
- Existing ignored `context/tmp/` hook/runtime artifacts were left untouched because they are runtime artifacts, not T03 scaffolding.
- Context sync classified T03 as verify-only/final-validation status work; current-state Agent Trace documentation is present and discoverable from `context/context-map.md`.

### Success-criteria verification

- [x] Built Agent Trace structs serialize with top-level `metadata.sce.version` equal to the compiled `sce` CLI package version -> confirmed by `cli/src/services/agent_trace.rs` (`SCE_METADATA_VERSION = env!("CARGO_PKG_VERSION")`) and direct test assertion in `cli/src/services/agent_trace/tests.rs`.
- [x] Post-commit `agent_traces.trace_json` rows receive the enriched payload -> confirmed by current context/code path: post-commit builds via `agent_trace::build_agent_trace(...)`, validates JSON, then inserts serialized payload into AgentTraceDb.
- [x] Existing Agent Trace schema validation continues to pass -> confirmed by `nix flake check` and agent-trace golden validation coverage in the flake check surface.
- [x] Unit/golden fixture coverage proves the metadata field is present and stable -> confirmed by updated golden fixtures and test comparison of `actual_json["metadata"]` to fixture truth.
- [x] Current-state context docs describing the Agent Trace generator and DB payload are updated -> confirmed by context sync over `context/sce/agent-trace-minimal-generator.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, `context/overview.md`, and `context/glossary.md`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
