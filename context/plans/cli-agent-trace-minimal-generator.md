# Plan: CLI agent-trace minimal generator

## Change Summary

Add a library-only Rust seam under `cli/src/services/` that produces the minimal agent-trace JSON shape from patch data by treating `post_commit_patch` as the source of truth, computing `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`, and then comparing `intersection_patch` against `post_commit_patch` hunk by hunk.

User-confirmed decisions:

- Implementation surface is a library seam, not a user-invocable CLI command.
- Comparison flow is `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`, then compare `intersection_patch` hunks with `post_commit_patch` hunks.
- `post_commit_patch` is the canonical source of truth for file/hunk coverage.
- MVP output is the minimal JSON shape only.
- There should be the same number of hunks and conversations in the emitted result, aligned to `post_commit_patch`.

## Success Criteria

1. The CLI service layer exposes a library seam that accepts the necessary patch inputs and returns a minimal agent-trace payload matching the agreed JSON shape.
2. The generator uses `intersect_patches(constructed_patch, post_commit_patch)` as the AI candidate patch and compares the resulting `intersection_patch` against `post_commit_patch` hunk by hunk.
3. Hunk classification follows the agreed MVP rules:
   - exact same hunk content line by line => `ai`
   - same hunk slot in `post_commit_patch` but not exact line-by-line match => `mixed`
   - hunk present in `post_commit_patch` but missing from `intersection_patch` => `unknown`
4. Output remains anchored to `post_commit_patch`, with one emitted conversation per `post_commit_patch` hunk and stable line ranges derived from the corresponding `post_commit_patch` hunk.
5. Automated tests cover the exact-match, mixed, and missing-hunk cases and verify deterministic JSON-serializable output.
6. Final validation and context sync confirm the new library seam and terminology are reflected in `context/` where required.

## Constraints and Non-Goals

- In scope: Rust library/domain modeling, hunk comparison/classification logic, minimal JSON payload generation, tests, and context updates required by the new seam.
- Out of scope: adding a new `sce` command surface, hook/runtime integration, persistence, OpenCode plugin behavior, or expanding beyond the minimal JSON contract.
- Use existing `cli/src/services/patch.rs` types and `intersect_patches` behavior as the comparison foundation rather than inventing a separate patch parser/model.
- `post_commit_patch` is the canonical output shape owner; the generator must not emit fewer or more conversations than `post_commit_patch` hunks for included files.
- Keep tasks atomic: each executable task must land as one coherent commit unit.

## Task Stack

- [x] T01: `Define minimal agent-trace domain and hunk classification contract` (status:done)
  - Task ID: T01
  - Goal: Add the minimal Rust-side domain types and comparison contract for agent-trace generation, including the exact `ai` / `mixed` / `unknown` hunk classification rules anchored to `post_commit_patch`.
  - Boundaries (in/out of scope): In - minimal serializable payload structs, contributor/classification enums or equivalents, file/hunk alignment rules against `post_commit_patch`, and focused unit tests for the comparison contract. Out - generator orchestration across multiple files beyond what the contract needs, CLI command wiring, or broad context sync.
  - Done when: the codebase has a clearly defined minimal agent-trace payload model and deterministic hunk-classification helpers/tests proving exact-match, same-slot-but-different-content, and missing-hunk behavior against `post_commit_patch`.
  - Verification notes (commands or checks): add targeted Rust tests near the new seam; preferred final repo validation remains `nix flake check`, but task-level evidence should include focused assertions covering `intersect_patches(constructed_patch, post_commit_patch) -> intersection_patch` then `intersection_patch` vs `post_commit_patch` hunk classification.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `cli/src/services/agent_trace.rs`, `cli/src/services/mod.rs`
  - **Evidence:** `nix flake check` all checks passed; `nix run .#pkl-check-generated` up to date
  - **Notes:** `build_agent_trace` was included as part of T01 since the domain model and classification contract naturally required a thin orchestration entrypoint; T02 will expand on generator behavior if needed

- [x] T02: `Implement minimal agent-trace generator from intersected patch output` (status:done)
  - Task ID: T02
  - Goal: Implement the library seam that takes patch inputs, computes `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`, compares `intersection_patch` with `post_commit_patch` hunk by hunk, and emits the minimal agent-trace JSON-ready payload with one conversation per `post_commit_patch` hunk.
  - Boundaries (in/out of scope): In - generator entrypoint, per-file iteration aligned to `post_commit_patch`, hunk-slot matching, conversation emission, and deterministic range construction from `post_commit_patch` hunk metadata. Out - command dispatch, filesystem I/O, persistence, schema-version migration machinery, or non-MVP payload enrichment.
  - Done when: callers can invoke the library seam with patch inputs and receive deterministic minimal agent-trace data whose files/conversations are derived from `post_commit_patch`, with `ai`, `mixed`, and `unknown` outcomes correctly represented.
  - Verification notes (commands or checks): expand targeted Rust tests to cover multi-file and mixed classification scenarios; verify the emitted payload serializes to the agreed minimal JSON structure.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`
  - **Evidence:** `nix flake check` all checks passed; `nix run .#pkl-check-generated` up to date
  - **Notes:** Earlier focused agent-trace test references were later removed by user request during post-task follow-up, so current tracked code does not include a dedicated agent-trace test module

- [x] T03: `Validate generator behavior and sync context` (status:done)
  - Task ID: T03
  - Goal: Run final validation, remove any temporary scaffolding, and update `context/` so future sessions understand the new minimal agent-trace generator seam and its `post_commit_patch`-anchored classification rules.
  - Boundaries (in/out of scope): In - final verification, plan evidence updates, and context sync for focused CLI/agent-trace docs plus root context files if the change introduces durable terminology or service ownership. Out - new runtime features beyond the planned generator seam.
  - Done when: validation passes, no temporary scaffolding remains, and context accurately documents the minimal agent-trace generator, its use of `intersect_patches`, and the `ai` / `mixed` / `unknown` hunk semantics anchored to `post_commit_patch`.
  - Verification notes (commands or checks): `nix flake check`; verify/update `context/overview.md`, `context/context-map.md`, `context/glossary.md`, and any focused `context/cli/` or `context/sce/` files required by the implemented seam.
  - **Status:** done
  - **Completed:** 2026-04-22
  - **Files changed:** `cli/src/services/agent_trace.rs`, `context/sce/agent-trace-minimal-generator.md`, `context/plans/cli-agent-trace-minimal-generator.md`
  - **Evidence:** `nix run .#pkl-check-generated` up to date; `nix flake check` all checks passed
  - **Notes:** Context sync was verify-only for root shared files (`overview`, `architecture`, `glossary`, `patterns`, `context-map`) because the implemented seam and terminology were already aligned to code truth; focused domain context was refreshed with discoverability links and no temporary scaffolding remained; focused agent-trace tests were removed by user request during follow-up

## Open Questions

None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)

### Temporary scaffolding

- None found; none removed.

### Success-criteria verification

- [x] Library seam accepts patch inputs and returns the minimal agent-trace payload -> confirmed in `cli/src/services/agent_trace.rs` (`build_agent_trace`)
- [x] Generator uses `intersect_patches(constructed_patch, post_commit_patch)` and compares against `post_commit_patch` hunks -> confirmed in `cli/src/services/agent_trace.rs`
- [x] Hunk classification follows `ai` / `mixed` / `unknown` MVP rules -> confirmed in `classify_hunk` plus inline tests in `cli/src/services/agent_trace.rs`
- [x] Output remains anchored to `post_commit_patch` with one conversation per post-commit hunk and stable ranges -> confirmed in `build_agent_trace` plus inline tests in `cli/src/services/agent_trace.rs`
- [ ] Automated tests cover exact-match, mixed, and missing-hunk behavior with deterministic JSON-ready output -> no dedicated agent-trace tests remain in the tracked tree after user-requested removal; only repo-level validation evidence remains
- [x] Context reflects the implemented seam and terminology -> confirmed by `context/sce/agent-trace-minimal-generator.md`, `context/context-map.md`, `context/overview.md`, and `context/glossary.md`

### Failed checks and follow-ups

- Initial final-validation pass surfaced that focused agent-trace tests described by the plan were not present in the tracked tree. Inline tests were briefly added in-scope, then removed by explicit user request during follow-up, so the plan now records the reduced coverage accurately.

### Residual risks

- The minimal generator seam currently has no dedicated focused tests in the tracked tree; validation evidence is limited to repo-level checks.
