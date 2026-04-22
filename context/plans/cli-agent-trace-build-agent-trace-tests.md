# Plan: CLI agent-trace build_agent_trace tests

## Change summary

Add focused Rust test coverage for `build_agent_trace` in `cli/src/services/agent_trace.rs` by reusing the fixture style already established in `cli/src/services/patch/tests.rs`. The follow-up plan stays tests-only: it adds an agent-trace-specific fixture/golden set and verifies that `build_agent_trace` produces the expected minimal payload without changing production behavior.

## Success criteria

1. The codebase has dedicated tests for `build_agent_trace` that exercise real patch fixtures rather than only hand-built inline cases.
2. The new tests reuse the existing patch-fixture approach from `cli/src/services/patch/tests.rs` where practical: incremental patch inputs, a post-commit patch input, parsed patch setup, and golden-output assertions.
3. The expected output for at least one agent-trace scenario is captured as an agent-trace-specific golden artifact rather than being reconstructed ad hoc inside the assertion.
4. Test scope remains limited to `build_agent_trace` behavior and expected `AgentTrace` output shape/classification (`ai` / `mixed` / `unknown`); no production logic changes are required.
5. Existing patch fixtures and production code remain unchanged unless a minimal test-only helper extraction is strictly necessary.
6. Final validation confirms the repository checks still pass and no additional context sync is required beyond recording plan progress.

## Constraints and non-goals

- In scope: test-only changes for `cli/src/services/agent_trace.rs`, agent-trace test fixture/golden files, and minimal test helpers needed to parse/reuse existing patch fixtures cleanly.
- In scope: introducing a new golden artifact for expected `AgentTrace` output.
- In scope: reusing existing reconstruction fixture data or fixture patterns from `cli/src/services/patch/tests.rs`.
- Out of scope: modifying `build_agent_trace`, `classify_hunk`, `intersect_patches`, or other production behavior unless a test exposes a real defect that must be planned separately.
- Out of scope: changing the existing patch test scenarios themselves.
- Out of scope: adding CLI/runtime integration, hook wiring, persistence, or context docs for unchanged behavior.
- Non-goal: broad test-framework refactors or cross-module cleanup unrelated to `build_agent_trace` coverage.
- Assumption: an existing reconstruction scenario from `cli/src/services/patch/tests.rs` can be reused as the source data for at least one deterministic `build_agent_trace` golden test.
- Assumption: a tests-only follow-up should land as atomic commits split by fixture preparation, test wiring, and validation.

## Task stack

- [x] T01: `Add agent-trace fixture and golden assets` (status:done)
  - Task ID: T01
  - Goal: Create agent-trace-specific test fixture inputs and at least one golden expected-output artifact derived from an existing patch reconstruction scenario so `build_agent_trace` tests can assert against stable serialized expectations.
  - Boundaries (in/out of scope): In - new test fixture/golden files under the relevant `agent_trace` test area, reuse of existing patch scenario data, and minimal test-only parsing helpers if needed. Out - production code changes, edits to the existing patch scenario fixtures, or adding multiple unrelated scenarios in one task.
  - Done when: the repository contains a deterministic `build_agent_trace` test scenario with clearly named patch inputs plus a checked-in golden expected `AgentTrace` payload ready for consumption by tests.
  - Verification notes (commands or checks): confirm fixture paths and serialized golden structure match the current `AgentTrace` schema; targeted Rust test execution will be added in T02.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/incremental_01.patch`, `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/incremental_02.patch`, `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/post_commit.patch`, `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json`, `context/plans/cli-agent-trace-build-agent-trace-tests.md`
  - **Evidence:** fixture paths added under `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction`; `jq empty cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json` passed
  - **Notes:** Reused the existing poem-edit reconstruction scenario as agent-trace-specific inputs because it deterministically covers all three contributor classifications (`mixed`, `unknown`, `ai`) without changing production code; targeted Rust test wiring remains scoped to T02

- [x] T02: `Add focused build_agent_trace golden test coverage` (status:done)
  - Task ID: T02
  - Goal: Add focused Rust tests for `build_agent_trace` that parse the chosen fixture inputs, invoke the library seam, and assert that the returned `AgentTrace` matches the checked-in golden output.
  - Boundaries (in/out of scope): In - test module/code for `agent_trace`, fixture loading, golden deserialization/assertions, and targeted coverage of contributor classification/output ordering guaranteed by the selected scenario. Out - production behavior changes, broad helper refactors, or adding unrelated patch-service assertions already covered elsewhere.
  - Done when: `build_agent_trace` has dedicated focused tests backed by the new fixtures/golden assets, and those tests prove deterministic payload construction from real patch inputs.
  - Verification notes (commands or checks): run the narrowest Rust test command covering the new `agent_trace` tests; verify assertions cover the full `AgentTrace` payload rather than partial spot checks only.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `cli/src/services/agent_trace.rs`, `flake.nix`, `context/plans/cli-agent-trace-build-agent-trace-tests.md`
  - **Evidence:** Added fixture-backed golden test `poem_edit_reconstruction_matches_golden_agent_trace`; `nix run .#pkl-check-generated` passed; `nix flake check` passed after adding `cli/src/services/agent_trace/fixtures` to the flake source set so the new test assets are present during sandboxed validation
  - **Notes:** Direct targeted `cargo test` execution was blocked by the repo's bash-policy preference for `nix flake check`, so verification used the required repo-level validation path instead

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run the repo validation baseline, verify the success criteria against code truth, and confirm whether context sync is verify-only.
  - Boundaries (in/out of scope): In - final validation, plan status/evidence updates, and context-sync verification. Out - new feature work or opportunistic test expansion.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, success criteria are checked against the implemented tests, and any required context follow-up is explicitly recorded.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `context/plans/cli-agent-trace-build-agent-trace-tests.md`
  - **Evidence:** `nix run .#pkl-check-generated` passed; `nix flake check` passed; `cli/src/services/agent_trace.rs` contains focused fixture-backed golden coverage via `poem_edit_reconstruction_matches_golden_agent_trace`; `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json` captures the expected `AgentTrace` payload with `mixed`, `unknown`, and `ai` contributor classifications
  - **Notes:** Success criteria confirmed against code truth: dedicated `build_agent_trace` tests now exercise real fixtures, reuse the existing patch-fixture pattern, assert against a checked-in agent-trace golden artifact, keep scope limited to expected `AgentTrace` output shape/classification, and require no production logic changes. Context sync remains verify-only; no additional durable context edits are required beyond plan progress.

## Open questions

- None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)

### Failed checks and follow-ups

- None.

### Success-criteria verification

- [x] Dedicated `build_agent_trace` tests exercise real patch fixtures rather than only inline cases -> confirmed by `cli/src/services/agent_trace.rs` test `poem_edit_reconstruction_matches_golden_agent_trace`
- [x] Tests reuse the existing patch-fixture approach (`incremental` inputs, `post_commit` input, parsed patch setup, golden assertions) -> confirmed by the `AgentTraceScenario` helper and `combine_patches`/`parse_patch` usage in `cli/src/services/agent_trace.rs`
- [x] Expected output for at least one scenario is captured as an agent-trace-specific golden artifact -> confirmed by `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json`
- [x] Scope remains limited to `build_agent_trace` behavior and expected `AgentTrace` output shape/classification with no production logic changes -> confirmed by the final diff scope (`cli/src/services/agent_trace.rs` test module only, fixture assets, `flake.nix` source inclusion for sandbox visibility, and plan updates)
- [x] Existing patch fixtures and production code remain unchanged unless minimal test-only extraction was necessary -> confirmed by reuse of new agent-trace fixture files without modifying `cli/src/services/patch/tests.rs` or production agent-trace logic
- [x] Final validation confirms repository checks still pass and no additional context sync is required beyond recording plan progress -> confirmed by the successful validation commands above and verify-only context-sync result

### Residual risks

- None identified.
