# Plan: sce-cli-rust-refactor-priority-pass

## 1) Change summary

Apply a focused Rust refactor pass for the CLI foundation that prioritizes runtime and parser correctness, removes high-noise placeholder/dead-code patterns, and improves maintainability in help rendering and shared test utilities.

Locked planning decisions from clarification gate:
- Tokio runtime strategy: one reusable global runtime (non-async app entrypoint retained).
- Dependency policy: std-only refactors; no new crates added.
- Scope boundary: top 5 suggestion set + Cargo tokio feature fix + shared test temp-dir utility extraction.

## 2) Success criteria

- `app.rs` uses `lexopt` as the primary top-level CLI parser (no manual `Vec`-driven arg parsing path for command routing).
- `sync` placeholder no longer builds a Tokio runtime per call; runtime is initialized once and reused safely.
- `command_surface::help_text()` avoids looped incremental `push_str` assembly for command rows and uses idiomatic composition.
- Duplicated top-level unknown-option error handling in `app.rs` is consolidated.
- Placeholder `#[allow(dead_code)]` usage is cleaned up in `mcp.rs`, `hooks.rs`, and `sync.rs` by either removing unused code paths or restructuring to avoid broad suppression.
- `cli/Cargo.toml` tokio features match actual async/runtime usage needed by the implementation.
- Repeated test temp-dir creation logic is extracted into a shared test helper used by both setup and local DB test suites.

## 3) Constraints and non-goals

Constraints:
- Keep placeholder command behavior/messages and command surface contract stable unless a test-updated behavior change is explicitly required by parser/runtime refactor.
- Do not introduce new dependencies (including `itertools`); std-only simplifications.
- Preserve current non-async public app entrypoint contract (`run(...) -> ExitCode`).

Non-goals (deferred):
- `setup.rs` path-component simplification (`components().all(...)`) unless touched incidentally by a required change.
- `EmbeddedAssetSelectionIter` redesign.
- Redundant generic `where`-clause style cleanup in `app.rs` beyond directly touched signatures.

## 4) Task stack (`T01..T07`)

- [x] T01: Rework top-level CLI parsing to lexopt-first flow (status:done)
  - Goal: Replace manual argv vector slicing/removal in `cli/src/app.rs` with a `lexopt::Parser`-driven command parse that still enforces current command/option/extra-arg contracts.
  - Boundaries (in): top-level parser and related parse helpers/tests in `cli/src/app.rs`.
  - Boundaries (out): setup-service option parser internals unless parser handoff contract needs adjustment.
  - Done when: parser behavior for `help`, known commands, unknown commands/options, and extra args is represented by lexopt-first logic and tests pass with equivalent user-facing outcomes.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml app::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T02: Remove per-call Tokio runtime creation in sync flow (status:done)
  - Goal: Introduce a single reusable runtime for sync placeholder smoke-check execution and eliminate runtime construction inside each `run()` call.
  - Boundaries (in): `cli/src/services/sync.rs` and any minimal shared runtime utility required for safe reuse.
  - Boundaries (out): broad async conversion of app/service dispatch (`#[tokio::main]` migration is excluded).
  - Done when: sync placeholder path reuses one runtime instance across calls and tests/assertions cover behavior stability.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::sync::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T03: Make command-surface help rendering idiomatic (status:done)
  - Goal: Refactor `cli/src/command_surface.rs` help text construction away from repeated `push_str` loops to compositional formatting (`format!`/`join`).
  - Boundaries (in): help text construction and related tests.
  - Boundaries (out): command catalog semantic changes.
  - Done when: rendered help text content remains contract-equivalent while implementation uses clearer compositional assembly.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml command_surface::tests`.

- [x] T04: Consolidate duplicate option-error handling in app parser (status:done)
  - Goal: Remove duplicated unknown-option bail branches in `cli/src/app.rs` and centralize shared error wording.
  - Boundaries (in): parser error branch structure and helper extraction local to app parser.
  - Boundaries (out): changing existing actionable error wording unless tests intentionally update contract text.
  - Done when: one canonical unknown-option handling path exists and tests validate deterministic error output.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml app::tests::parser_rejects_unknown_option`.

- [x] T05: Replace broad dead-code allowances with structured placeholder usage (status:done)
  - Goal: Remove or narrow `#[allow(dead_code)]` usage in `cli/src/services/{mcp,hooks,sync}.rs` by making placeholder domain types exercised by tests/paths or reducing unused definitions.
  - Boundaries (in): placeholder service modules and related tests.
  - Boundaries (out): implementing full non-placeholder production behavior.
  - Done when: dead-code suppressions are minimized/removed for the targeted modules without reducing placeholder contract clarity.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::mcp::tests services::hooks::tests services::sync::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T06: Align Tokio features and extract shared test temp-dir utility (status:done)
  - Goal: Update `cli/Cargo.toml` Tokio feature set to match runtime usage and extract duplicated test temp-dir setup from setup/local_db tests into a shared helper module.
  - Boundaries (in): `cli/Cargo.toml`, test-only utility module(s), and tests in `cli/src/services/setup.rs` + `cli/src/services/local_db.rs`.
  - Boundaries (out): non-test filesystem abstraction redesign.
  - Done when: Tokio features are explicitly sufficient for runtime/async usage, both test suites consume shared temp-dir helper, and test behavior remains stable.
  - Verification notes: run `cargo test --manifest-path cli/Cargo.toml services::setup::tests services::local_db::tests` and `cargo check --manifest-path cli/Cargo.toml`.

- [x] T07: Validation and cleanup (status:done)
  - Goal: Execute full verification pass, ensure no unintended command-surface regressions, and sync context docs if behavior/contracts changed.
  - Boundaries (in): formatting/lint/test/build checks for CLI slice and `context/` updates required by changed current-state behavior.
  - Boundaries (out): new feature implementation beyond refactor scope.
  - Done when: all targeted checks pass, temporary scaffolding is removed, and context updates (if needed) reflect final current-state contracts.
  - Verification notes: run `cargo fmt --manifest-path cli/Cargo.toml --all -- --check`, `cargo test --manifest-path cli/Cargo.toml`, `cargo build --manifest-path cli/Cargo.toml`, plus repo baseline `nix run .#pkl-check-generated` and `nix flake check` if context/pkl-facing artifacts were touched.

## 5) Open questions

None. Clarification-gate dependencies, architecture choice, and scope boundary are resolved for planning.

## 6) Validation report (T07)

Commands run (all from repo root):
- `cargo check --manifest-path cli/Cargo.toml` (exit 0)
  - Key output: `Finished 'dev' profile ...`
- `cargo fmt --manifest-path cli/Cargo.toml --all -- --check` (exit 0)
  - Key output: none (clean formatting)
- `cargo build --manifest-path cli/Cargo.toml` (exit 0)
  - Key output: `Finished 'dev' profile ...`
- `cargo test --manifest-path cli/Cargo.toml` (exit 0)
  - Key output: `test result: ok. 44 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
- `cargo build --manifest-path cli/Cargo.toml` (exit 0, repeated as explicit final-criteria build)
  - Key output: `Finished 'dev' profile ...`
- `nix run .#pkl-check-generated` (exit 0)
  - Key output: `Generated outputs are up to date.`
- `nix flake check` (exit 0)
  - Key output: flake checks evaluated and built successfully; only an informational omitted-systems warning was emitted.

Failed checks and follow-ups:
- None.

Success-criteria verification summary:
- `app.rs` lexopt-first parser flow: satisfied (covered by full test pass, including parser-focused tests).
- Shared sync runtime reuse: satisfied (covered by `services::sync::tests::sync_runtime_is_reused_across_calls`).
- Command-surface help composition refactor: satisfied (covered by `command_surface` tests and full suite pass).
- Duplicate unknown-option handling consolidation: satisfied (`app::tests::parser_rejects_unknown_option` passes).
- Dead-code allowance cleanup in placeholder services: satisfied (full compile/test pass with targeted modules).
- Tokio feature alignment + shared test temp-dir helper extraction: satisfied (full compile/test pass including setup/local_db suites).

Context sync and drift status:
- Mandatory sync files reviewed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`.
- No additional context edits required for T07 because this task performed validation/cleanup only and introduced no new runtime behavior/contracts.
- Residual risks: none identified for this task scope.
