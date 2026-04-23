# Plan: CLI agent-trace commit-timestamp metadata + UUIDv7

## Change summary

Update the agent-trace domain/API so `AgentTrace.timestamp` is sourced from commit metadata (not wall-clock `now`) and `AgentTrace.id` is generated as UUIDv7 from that same commit-time moment. Keep scope limited to domain/API and tests/fixtures; do not add runtime hook wiring in this plan.

User-confirmed decisions:

- Timestamp source: commit metadata time (`C`), not trace-filename time.
- Fallback strategy: commit-metadata fallback (`C`) when the preferred commit-time source is unavailable.
- API shape: explicit metadata input into `build_agent_trace(...)` from caller (`A`), no internal `context/tmp` reads.
- Scope boundary: **domain/API + tests only**, no runtime hook integration in this plan.
- Test contract: use fixed arbitrary commit-time test values (deterministic literals) instead of dynamic placeholder tokens like `<generated-uuid>` / `<generated-rfc3339-timestamp>`.

## Success criteria

1. `build_agent_trace(...)` no longer derives timestamp from `Utc::now()`; it consumes explicit commit-time metadata input.
2. `AgentTrace.timestamp` equals the resolved commit-time metadata passed into the builder (RFC 3339 contract).
3. `AgentTrace.id` is generated as UUIDv7 and derived from the same commit-time moment used for `timestamp`.
4. Existing nested payload semantics remain unchanged (`files[].path`, `conversations[]`, nested `contributor.type`, `ranges[].start_line/end_line`).
5. Agent-trace tests are updated to validate UUIDv7 + timestamp-source behavior and still protect nested payload shape.
6. Fixtures/goldens use deterministic literal metadata values for commit-time test scenarios (no `<generated-*>` placeholder tokens).
7. Final validation and cleanup are completed, and context reflects the current metadata contract and boundaries.

## Constraints and non-goals

- In scope: `cli/src/services/agent_trace.rs` domain/API changes, metadata input contract, UUIDv7 generation logic, and focused test/fixture updates.
- In scope: dependency feature updates needed for UUIDv7 support in `cli/Cargo.toml`.
- In scope: documenting metadata-source contract in agent-trace context files.
- Out of scope: wiring post-commit runtime flow to call `build_agent_trace(...)`.
- Out of scope: reading timestamp directly from `context/tmp/*-post-commit.json` inside `agent_trace` layer.
- Out of scope: changing hunk-classification, intersection semantics, or contributor taxonomy.

## Task stack

- [x] T01: `Define explicit commit-time metadata input contract for agent-trace builder` (status:done)
  - Task ID: T01
  - Goal: Refactor agent-trace builder signature/inputs so timestamp is caller-provided commit metadata instead of internally generated wall-clock time.
  - Boundaries (in/out of scope): In - `build_agent_trace(...)` metadata-input contract, RFC3339 input validation/normalization seam, removal of `Utc::now()` metadata sourcing. Out - runtime hook wiring and file-system reads from `context/tmp`.
  - Done when: `build_agent_trace(...)` requires explicit commit-time metadata input and no longer computes timestamp from process-now.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test agent_trace'`; focused assertions that output timestamp equals provided commit-time input.
  - Completed: 2026-04-23
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/file_rename_reconstruction/golden.json`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#checks.x86_64-linux.cli-fmt` (pass); `nix build .#default` (pass)
  - Notes: `build_agent_trace` now requires explicit `AgentTraceMetadataInput { commit_timestamp }`, validates commit timestamp as RFC 3339 before build, and sets `AgentTrace.timestamp` from provided commit metadata instead of wall-clock time. Added focused tests for provided timestamp pass-through and invalid RFC 3339 rejection.

- [x] T02: `Generate UUIDv7 from resolved commit-time moment` (status:done)
  - Task ID: T02
  - Goal: Replace UUIDv4 generation with UUIDv7 generation tied to the same commit-time moment used for `timestamp`.
  - Boundaries (in/out of scope): In - UUID dependency feature/config updates, UUIDv7 generation helper, version/shape checks in tests. Out - introducing non-commit-time ID entropy policies beyond UUIDv7 defaults.
  - Done when: `AgentTrace.id` parses as UUID and reports version 7 for builder outputs produced from provided commit-time metadata.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test agent_trace'`; targeted assertions for UUID version 7.
  - Completed: 2026-04-23
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/Cargo.toml`, `cli/Cargo.lock`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#checks.x86_64-linux.cli-clippy` (pass); `nix build .#checks.x86_64-linux.cli-fmt` (pass); `nix build .#default` (pass); `nix run .#pkl-check-generated` (pass); `nix flake check` (pass)
  - Notes: Replaced UUIDv4 generation with UUIDv7 derived from parsed commit-time metadata (`commit_timestamp`), while keeping `AgentTrace.timestamp` sourced from caller-provided commit metadata input and preserving payload shape semantics.

- [x] T03: `Update agent-trace golden/tests for commit-time metadata contract` (status:done)
  - Task ID: T03
  - Goal: Refresh tests/fixtures to validate commit-time-sourced timestamp behavior while preserving existing nested payload assertions.
  - Boundaries (in/out of scope): In - `cli/src/services/agent_trace/tests.rs`, related fixture goldens under `cli/src/services/agent_trace/fixtures/**`, and replacement of dynamic metadata placeholders with deterministic literals. Out - fixture expansion unrelated to metadata contract.
  - Done when: tests assert timestamp equals provided commit-time metadata, validate UUIDv7 format/version, continue matching nested payload semantics, and no fixture uses `<generated-uuid>` or `<generated-rfc3339-timestamp>` placeholders.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test agent_trace'`; `nix build .#checks.x86_64-linux.cli-tests`.
  - Completed: 2026-04-23
  - Files changed: `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/average_age_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/file_rename_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/hello_world_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/mixed_change_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/poem_write_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/text_file_lifecycle_reconstruction/golden.json`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#default` (pass)
  - Notes: Replaced dynamic `<generated-*>` fixture metadata with deterministic literals, preserved nested payload assertions via golden comparison, and kept explicit UUIDv7 + commit-timestamp assertions in tests.

- [x] T04: `Context sync for updated agent-trace metadata contract` (status:done)
  - Task ID: T04
  - Goal: Sync context docs to reflect commit-time timestamp sourcing + UUIDv7 contract and explicit non-integration boundary.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-minimal-generator.md`, `context/glossary.md`, `context/context-map.md` (if discoverability changes). Out - broad architecture rewrites unrelated to this metadata seam.
  - Done when: context clearly states timestamp source contract (commit metadata input), UUIDv7 ID behavior, and that runtime hook integration remains out of scope.
  - Verification notes (commands or checks): context drift review against code truth; link checks for updated/added context files.
  - Completed: 2026-04-23
  - Files changed: `context/sce/agent-trace-minimal-generator.md`, `context/plans/cli-agent-trace-commit-timestamp-uuidv7.md`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix flake check` (pass)
  - Notes: Synced context wording to the implemented metadata contract by keeping commit-time timestamp + UUIDv7 coupling explicit, correcting the sample payload ID to a UUIDv7 literal, and reaffirming runtime hook integration remains out of scope.

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run final validation, ensure no temporary scaffolding remains, and record evidence in the plan.
  - Boundaries (in/out of scope): In - repo validation checks, plan status/evidence updates, final context-sync verification. Out - new behavior scope.
  - Done when: validation evidence is recorded, all scoped tasks are complete, and no temporary scaffolding from this change remains.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-04-23
  - Files changed: `context/plans/cli-agent-trace-commit-timestamp-uuidv7.md`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix flake check` (pass)
  - Notes: Final validation completed with both required checks passing. No task-specific temporary scaffolding was introduced by this task.

## Open questions

- None.

## Validation Report (T05)

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)

### Temporary scaffolding cleanup

- Removed: none (this task introduced no temporary scaffolding)

### Context sync verification

- Classification: verify-only (validation/plan-state update only; no code-path or architecture change)
- Root sync pass completed for shared files (`context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`); no root edits required.
- Feature existence documentation confirmed in `context/sce/agent-trace-minimal-generator.md` and discoverable via `context/context-map.md`.

### Success-criteria verification summary

- [x] 1. `build_agent_trace(...)` consumes explicit commit-time metadata input (T01 evidence in task notes and changed files)
- [x] 2. `AgentTrace.timestamp` equals provided commit-time metadata (T01/T03 test evidence)
- [x] 3. `AgentTrace.id` is UUIDv7 derived from same commit-time moment (T02/T03 evidence)
- [x] 4. Nested payload semantics unchanged (`files[].path`, `conversations[]`, `contributor.type`, `ranges[]`) (T03 golden-shape assertions)
- [x] 5. Tests validate UUIDv7 + timestamp source behavior and payload shape (T03 evidence)
- [x] 6. Fixtures use deterministic literal metadata (T03 fixture updates)
- [x] 7. Final validation + cleanup + context-sync completed (T05 checks and context sync verification above)

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for this plan scope.
