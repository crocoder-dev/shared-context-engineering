# Plan: sce-cli-rust-idiomatic-hardening-pass

## 1) Change summary

Apply an idiomatic and safety-focused Rust hardening pass across hosted reconciliation, local DB path handling, parser/runtime ergonomics, and large test-module maintainability. Replace brittle handwritten primitives (crypto and JSON parsing), remove broad suppression patterns, and stage an incremental test split for oversized service files.

Locked clarification decisions:
- Dependency policy: add runtime crates as needed (`hmac`, `sha2`, `serde_json`) and update dependency-contract/context references.
- Test split scope: incremental extraction only (target highest-churn test slices now; full migration deferred).
- Float tie policy: use a small epsilon tie window and document deterministic behavior.

## 2) Success criteria

- Hosted signature and idempotency hashing in `cli/src/services/hosted_reconciliation.rs` use vetted crates (`hmac` + `sha2`) with no handwritten SHA-256/HMAC implementation remaining.
- Hosted webhook payload field extraction no longer uses string scanning (`find_required_json_string`); parsing uses `serde_json` value/typed access with deterministic error messages for missing/invalid fields.
- Rewrite score tie handling avoids direct `f32 == f32` comparison and applies documented epsilon-based tie semantics.
- Local DB connection and test helpers avoid lossy path conversion where possible; any required UTF-8 conversion is explicit and contextualized.
- `cli/src/services/agent_trace.rs` no longer uses crate-wide `#![allow(dead_code)]`; any remaining allow is narrowly scoped and justified by placeholder contract needs.
- Top-level argument parsing in `cli/src/app.rs` no longer clones `tail_args` just to initialize `lexopt`.
- `cli/src/services/sync.rs` uses idiomatic `OnceLock` initialization flow (`get_or_init`/`get_or_try_init` style) instead of manual get/set/get choreography.
- `cli/src/services/hooks.rs` and `cli/src/services/setup.rs` have an incremental test/runtime separation pass applied (targeted test-module extraction to smaller files/modules) with behavior preserved.

## 3) Constraints and non-goals

Constraints:
- Preserve current user-facing command behavior and error semantics unless a safety fix requires an intentional update covered by tests.
- Keep hosted reconciliation mapping and signature verification contracts stable while changing internals.
- Maintain deterministic outcomes for tie resolution and unresolved mapping reporting.
- Keep task slicing one-task/one-atomic-commit.

Non-goals (deferred):
- Full migration of all tests in `hooks.rs` and `setup.rs` into integration tests under `cli/tests/`.
- Broad architecture redesign of service boundaries beyond targeted extraction needed for maintainability.
- Functional feature expansion outside listed refactor/safety concerns.

## 4) Task stack (`T01..T09`)

- [x] T01: Replace handwritten hosted crypto with vetted crates and align dependency contract (status:done)
  - Goal: Remove manual `sha256`/`hmac_sha256` internals in hosted reconciliation and wire `hmac` + `sha2` crate usage for signature/idempotency hashing.
  - Boundaries (in): `cli/src/services/hosted_reconciliation.rs`, `cli/Cargo.toml`, `cli/src/dependency_contract.rs`, and related unit tests.
  - Boundaries (out): Changing provider signature policy semantics (GitHub/GitLab contract must stay equivalent).
  - Done when: handcrafted crypto helpers are removed/replaced; dependency contract compiles and tests validate equivalent signature/hash behavior.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::hosted_reconciliation::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T02: Replace fragile hosted JSON string scanning with structured parsing (status:done)
  - Goal: Replace `find_required_json_string` usage with `serde_json` parsing (typed/value extraction) for `before`, `after`, and provider-specific repository fields.
  - Boundaries (in): Hosted payload parse path and parse-focused tests in `cli/src/services/hosted_reconciliation.rs`; dependency additions in `cli/Cargo.toml` as needed.
  - Boundaries (out): New provider support or webhook schema expansion beyond existing GitHub/GitLab fields.
  - Done when: no manual substring search parser remains on hosted intake path; missing/invalid-field failures are deterministic and covered by tests.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::hosted_reconciliation::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T03: Introduce epsilon-based tie handling for rewrite score comparison (status:done)
  - Goal: Remove direct float equality check in candidate tie detection and apply explicit epsilon tie-window semantics.
  - Boundaries (in): Tie detection and mapping-outcome tests in `cli/src/services/hosted_reconciliation.rs`.
  - Boundaries (out): Replacing score model or threshold policy (`FUZZY_MAPPING_THRESHOLD`) beyond tie logic.
  - Done when: tie behavior is epsilon-based, deterministic, and tested for near-equal/clearly-different scores.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::hosted_reconciliation::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T04: Eliminate lossy DB path string conversion in local DB service/tests (status:done)
  - Goal: Refactor local DB target path handling to avoid `to_string_lossy()` for DB location construction, using `Path`-native or explicit fallible conversion with context.
  - Boundaries (in): `cli/src/services/local_db.rs` runtime and test helpers.
  - Boundaries (out): Turso API redesign assumptions or broader filesystem abstraction rewrite.
  - Done when: targeted lossy conversions at current call sites are removed/replaced with explicit safe handling and tests still pass.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::local_db::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T05: Remove broad dead-code suppression from agent trace module (status:done)
  - Goal: Remove `#![allow(dead_code)]` from `cli/src/services/agent_trace.rs` and apply narrow item-level handling only where required.
  - Boundaries (in): `cli/src/services/agent_trace.rs` and directly affected tests/usages.
  - Boundaries (out): Large-scale pruning of placeholder Agent Trace contracts not required to satisfy compiler hygiene.
  - Done when: crate-level dead-code allow is absent and compile/test remain green without broad suppression.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::agent_trace::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T06: Remove avoidable `tail_args` clone in top-level parser (status:done)
  - Goal: Restructure top-level parsing so `lexopt` consumes arguments without cloning `tail_args` solely for parser initialization.
  - Boundaries (in): `cli/src/app.rs` parse flow and parser tests.
  - Boundaries (out): Command-surface behavioral changes unrelated to clone removal.
  - Done when: `parse_command` no longer clones `tail_args` for `Parser::from_args`, with behavior preserved and tests passing.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml app::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T07: Simplify sync runtime initialization with idiomatic OnceLock API (status:done)
  - Goal: Replace manual get/set/get runtime init in `shared_runtime` with `OnceLock` idioms (`get_or_try_init` or equivalent safe pattern).
  - Boundaries (in): `cli/src/services/sync.rs` runtime init path and relevant tests.
  - Boundaries (out): Async architecture changes beyond runtime initialization style.
  - Done when: runtime initialization code is single-flow and atomic in style, preserving current error context and reuse behavior.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::sync::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T08: Apply incremental test/runtime separation in hooks/setup modules (status:todo)
  - Goal: Improve maintainability by extracting selected large in-file test sections from `hooks.rs` and `setup.rs` into focused sibling test modules/files while preserving current test semantics.
  - Boundaries (in): test module organization and local helper placement for `cli/src/services/hooks.rs` and `cli/src/services/setup.rs`.
  - Boundaries (out): Full integration-test migration and non-test production refactors not needed for extraction.
  - Done when: high-churn/large test slices are moved out of primary runtime files, module compiles cleanly, and affected test suites pass.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::hooks::tests services::setup::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T09: Validation and cleanup (status:todo)
  - Goal: Execute full verification sweep, confirm behavior parity for touched domains, and sync context artifacts to current state (including dependency contract references).
  - Boundaries (in): formatting/build/test checks, plan status finalization, and required context updates in `context/`.
  - Boundaries (out): New feature work beyond this hardening pass.
  - Done when: all verification checks pass, no temporary scaffolding remains, and context files reflect final behavior/contracts.
  - Verification notes: run `cargo fmt --manifest-path cli/Cargo.toml --all -- --check`, `cargo test --manifest-path cli/Cargo.toml`, `cargo build --manifest-path cli/Cargo.toml`, and repo baseline checks `nix run .#pkl-check-generated` plus `nix flake check` when context/pkl artifacts are touched.

## 5) Open questions (if any)

None. Scope, dependency direction, tie policy, and test-split depth were resolved during clarification.
