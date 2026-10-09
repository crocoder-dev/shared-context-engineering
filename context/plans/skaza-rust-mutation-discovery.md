# Plan: skaza-rust-mutation-discovery

## Change summary

Adds `skaza`, a new standalone Rust package (library + binary) at `crates/skaza/`, that discovers Rust mutation candidates without executing them. It extracts the minimum useful Rust-only pieces of the local Ooze repository (`../ooze`): source-file discovery and identity (`src/source_path.rs`, `src/lang/mod.rs`), six mutation operators and their Tree-sitter queries (`src/lang/rust.rs`, `queries/rust/*.scm`), candidate discovery and edit deduplication (`src/mutate/mod.rs`), and Rust skip rules (`src/skip/mod.rs`). The code is copied and simplified, never depended upon: Skaza builds with `../ooze` absent.

The CLI exposes exactly `--src` (repeatable, default `.`), `--limit` (positive, default `10`), and `--format` (`text`|`json`, default `text`), and prints a discovery-only report of deterministic, deduplicated candidates. This is new behavior; the existing `sce` CLI source, dependencies, and release packaging stay unchanged apart from narrowing the CLI's Nix source fileset so Skaza edits do not invalidate CLI derivations. The root flake gains independent Skaza build/test/clippy/fmt checks so `nix flake check` validates Skaza. Mutation application and execution are deferred to PR 2.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `crates/skaza` is one Cargo package named `skaza` (edition 2021) with its own `Cargo.lock`, a library and a binary, no root Cargo workspace, no dependency on `cli/` or `../ooze`, and `crates/skaza/target/` is git-ignored.
  - Validate: inspect `crates/skaza/Cargo.toml` (no `path =` dependencies, `edition = "2021"`, `[lib]`/`[[bin]]` or default targets), confirm no root `Cargo.toml` exists, `git check-ignore crates/skaza/target/x` succeeds, and `nix build .#checks.<system>.skaza-tests` passes in the Nix sandbox where `../ooze` is unavailable.
- [ ] AC2: The `sce` CLI's `cli/Cargo.toml`, `cli/Cargo.lock`, `packages.sce`, `packages.sce-release`, and release apps are unaffected by Skaza; editing a file under `crates/skaza/` does not change the `sce` package derivation.
  - Validate: `git diff main -- cli/Cargo.toml cli/Cargo.lock` is empty; `nix eval --raw .#packages.<system>.sce.drvPath` is identical before and after touching a file under `crates/skaza/src/`; `packages` and release apps do not reference Skaza.
- [ ] AC3: `skaza --help` lists exactly `--src`, `--limit`, `--format` (plus clap's help/version), has no subcommands, `--limit 0` exits non-zero with an actionable error on stderr, and an invalid `--format` value is rejected.
  - Validate: `nix develop -c cargo run --manifest-path crates/skaza/Cargo.toml -- --help`; `... -- --limit 0` exit code and stderr; covered by integration tests in `crates/skaza/tests/`.
- [ ] AC4: Repeated `--src` accepts files and directories, deduplicates overlapping/duplicate inputs by canonical identity, respects `.gitignore`, skips `.git`, `target`, generated files, test-only paths, and directory symlinks, and errors clearly on nonexistent or non-`.rs` explicit paths.
  - Validate: `source.rs` unit tests and binary integration tests covering duplicate/overlapping `--src`, invalid paths, and order independence pass under `nix flake check`.
- [ ] AC5: The six operators `swap_boolean`, `negate_equality`, `comparison_boundary`, `swap_logical`, `remove_not`, `swap_predicate_method` each produce exact expected candidates (byte range, line/column, original, replacement) and never match inside comments or string literals.
  - Validate: per-operator unit tests with exact expected candidates in `crates/skaza/src/rust.rs` pass under `nix flake check`.
- [ ] AC6: No candidates are produced inside `#[cfg(test)]` modules (including inline modules), `#[test]` functions, assertion macros, panic/unreachable/todo/unimplemented macros, or generated files.
  - Validate: exclusion unit tests with exact expected candidate lists pass under `nix flake check`.
- [ ] AC7: Candidates are deduplicated by identical edit, keep distinct edits at the same location, are deterministically sorted before `--limit` is applied globally, and carry IDs stable for the same source tree and invocation root.
  - Validate: `mutation.rs` unit tests plus an integration test asserting byte-identical JSON across two runs and across reversed `--src` order.
- [ ] AC8: Text output shows total vs. selected counts, file:line:column, operator, original and replacement, and is labeled discovery-only; JSON output is valid, stable, carries the same information, and stdout contains only the report while diagnostics go to stderr; empty results succeed with zero counts.
  - Validate: `report.rs` serialization tests and binary integration tests for both formats and for an empty tree.
- [ ] AC9: Source files are never modified by any invocation.
  - Validate: integration test comparing fixture bytes before and after running both formats; `git status --porcelain cli/` is empty after the dogfood run.
- [ ] AC10: Dogfooding against SCE discovers up to ten distinct valid candidates from production code and the JSON output is identical across two runs.
  - Validate: run `nix develop -c cargo run --manifest-path crates/skaza/Cargo.toml -- --src cli/src/services/repository_identity/mod.rs --limit 10 --format json` twice, diff the outputs, confirm `selected` ≤ 10 and > 0, no candidate line falls at or after the `#[cfg(test)]` module, and `git status --porcelain` is clean.
- [ ] AC11: `nix flake check` builds, tests, lints, and format-checks Skaza through dedicated checks, not only the existing CLI checks.
  - Validate: `nix flake show` lists `skaza-tests`, `skaza-clippy` (`--all-targets -- -D warnings`), and `skaza-fmt` checks; `nix flake check` passes.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix develop -c cargo clippy --manifest-path crates/skaza/Cargo.toml --all-targets -- -D warnings`

### Context sync

- New `context/skaza/skaza-mutation-discovery.md` describing the crate boundary, CLI contract, operators, exclusion rules, candidate model/ID scheme, and report shapes.
- `context/context-map.md` entry for the new Skaza context file.
- `context/sce/flake-build-performance.md` and the flake paragraph in `context/overview.md`: the independent Skaza checks and the narrowed CLI source fileset.
- `context/glossary.md`: `Skaza` and `mutation candidate`.
- `AGENTS.md` repository shape: add `crates/skaza/` and its validation commands.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `crates/skaza/**`, root `.gitignore`, root `flake.nix` (new Skaza checks and narrowing the CLI `workspaceSrc` fileset), and the durable context files listed above.
- **Out of scope:** mutation application, compilation/test execution, worktree management, source backups and recovery, parallel execution, coverage, mutation scoring, agent orchestration, multi-language support, TOML/config files, extra CLI flags or subcommands, report persistence, SARIF, historical reporting, killed/survived outcomes, function complexity/branch-counting metadata, and any change to `../ooze`.
- **Constraints:** Rust edition 2021; Rust 1.95 toolchain and Crane from the existing flake; dependencies limited to clap, tree-sitter, tree-sitter-rust, ignore, serde, serde_json, and anyhow/thiserror (dev-dependencies such as `tempfile` allowed for tests); separate `crates/skaza/Cargo.lock`; no root Cargo workspace; `.scm` queries loaded via `include_str!` must be present in the Nix check source; Skaza stays out of `packages`, release apps, and release artifacts; stdout carries only the report; all non-coreutils tools run through Nix.
- **Non-goal:** a generic multi-language operator registry, operator plugin system, or reuse of Ooze's `mutators!` macro, `LanguageSpec`, config, or operator metadata.

## Assumptions

- Operator replacement behavior is copied from Ooze's Rust `mutators!` entries: `swap_boolean` (`true`↔`false`), `negate_equality` (`==`↔`!=`), `comparison_boundary` (`<`↔`<=`, `>`↔`>=`), `swap_logical` (`&&`↔`||`), `remove_not` (strip leading `!` from a unary `!` expression; skip if nothing remains), `swap_predicate_method` (`is_some`↔`is_none`, `is_ok`↔`is_err`; other method names produce nothing). Ooze's `describe` strings are not carried over.
- Operators are a `#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)] enum Operator` with `match`-based query selection and replacement (static dispatch); the six queries compile once into a struct reused for every file in a run.
- Test-only path detection keeps Ooze's Rust-relevant rules (path segments `tests`, `benches`; stems `tests`, `*_test`, `*_tests`, `test_*`) and generated-file detection keeps Ooze's rule (`@generated` or `DO NOT EDIT` in a leading comment within the first 15 lines). Directories under `spec/` are not treated as test-only, since SCE's top-level `spec/` holds Quint models, not Rust tests.
- `#[cfg(test)]` / `#[test]` / macro exclusions are implemented with Tree-sitter ancestry checks (attribute items preceding `mod_item`/`function_item`, `macro_invocation` names) rather than Ooze's byte scanner. Comments and string literals are excluded by construction because queries match only syntax nodes, verified by tests.
- An explicit `--src` naming a test-only or generated `.rs` file is not an error; it contributes zero candidates and a note is written to stderr. A non-`.rs` explicit file is an error.
- Report paths are rendered relative to the invocation directory with `/` separators when the file lies under it, otherwise as the canonical absolute path.
- Candidate IDs are a fixed-width hex digest of `(report path, start byte, end byte, operator, replacement)` computed with an in-crate deterministic hash (for example FNV-1a 64), because `std`'s `DefaultHasher` is not stable across Rust releases and no hashing crate is in the allowed dependency set.
- Deterministic sort order is `(report path, start byte, end byte, operator, replacement)`; deduplication key is `(canonical file, start byte, end byte, replacement)`, keeping the lexicographically first operator name.
- Line and column are 1-based; column counts UTF-8 bytes from the start of the line, matching Tree-sitter's `Point`.
- The root `flake.nix` CLI `workspaceSrc` currently unions `craneLib.fileset.commonCargoSources workspaceRoot`, which would pull `crates/skaza` Rust sources into every CLI derivation; it is narrowed to exclude `crates/` so Skaza remains independent of CLI packaging. This is a fileset-only change with no effect on CLI build inputs.
- The request's `nix develop -c cargo test --manifest-path crates/skaza/Cargo.toml` may be rejected by the repository's `cargo test` bash policy; the `skaza-tests` flake check is the canonical test path, with the direct command used only where policy allows.

## Task stack

- [ ] T01: `Scaffold the standalone skaza crate and CLI contract` (status:todo)
  - Task ID: T01
  - Scope: In — `crates/skaza/Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/main.rs`, empty-but-compiling `src/source.rs`, `src/mutation.rs`, `src/rust.rs`, `src/report.rs`, `queries/rust/`, `tests/` directories; clap derive `Args` with exactly `--src` (repeatable `Vec<PathBuf>`, default `.`), `--limit` (`NonZeroUsize`/validated, default `10`, actionable error on `0`), `--format` (`text`|`json` value enum, default `text`); root `.gitignore` entry `crates/skaza/target/`. Out — discovery, operators, reporting, Nix wiring.
  - Dependencies: none
  - Done when: crate builds with the minimal dependency set and its own lockfile; argument-parsing unit tests cover defaults, repeated `--src`, zero limit rejection, and invalid format; no root workspace is introduced and `cli/` is untouched.
  - Verify: `nix develop -c cargo build --manifest-path crates/skaza/Cargo.toml`; `nix develop -c cargo run --manifest-path crates/skaza/Cargo.toml -- --help`; `... -- --limit 0` exits non-zero; `git check-ignore crates/skaza/target/x`.
  - Context synchronization: pending

- [ ] T02: `Add independent Skaza checks to the Nix flake` (status:todo)
  - Task ID: T02
  - Scope: In — root `flake.nix`: a Skaza source fileset (Cargo sources plus `crates/skaza/queries/**/*.scm`), a Skaza `buildDepsOnly` artifact on the existing `craneLib`/Rust 1.95 toolchain, and `checks.skaza-tests`, `checks.skaza-clippy` (`--all-targets -- -D warnings`), `checks.skaza-fmt`; narrow the CLI `workspaceSrc` so `crates/` is excluded. Out — adding Skaza to `packages`, apps, release derivations, or CI workflow files; any other CLI derivation change.
  - Dependencies: T01
  - Done when: `nix flake check` runs the three Skaza checks; the `sce` package derivation path is unchanged by edits under `crates/skaza/`; Skaza appears in no package, app, or release output.
  - Verify: `nix flake show` lists the new checks; `nix build .#checks.<system>.skaza-tests .#checks.<system>.skaza-clippy .#checks.<system>.skaza-fmt`; compare `nix eval --raw .#packages.<system>.sce.drvPath` before/after touching `crates/skaza/src/lib.rs`.
  - Context synchronization: pending

- [ ] T03: `Implement Rust source discovery` (status:todo)
  - Task ID: T03
  - Scope: In — `src/source.rs`: resolve each `--src` against the invocation directory; explicit file must exist and end in `.rs`; directories walked with `ignore::WalkBuilder` (gitignore respected, symlinks not followed, `.git`/`target` skipped); test-only path and generated-file filtering; canonical-identity dedup across duplicate/overlapping inputs; deterministic sorted output carrying canonical path, report path, and source text; clear errors for nonexistent/unsupported paths; unit tests. Out — parsing, operators, exclusions inside files.
  - Dependencies: T01
  - Done when: unit tests over temp trees prove recursion, `.gitignore`, `target`/`.git` skipping, symlinked-directory non-following, generated and test-only file exclusion, duplicate and overlapping `--src` dedup, stable report paths, and exact error messages for missing and non-`.rs` paths.
  - Verify: `nix build .#checks.<system>.skaza-tests` (or `nix develop -c cargo test --manifest-path crates/skaza/Cargo.toml source` where policy allows).
  - Context synchronization: pending

- [ ] T04: `Implement the mutation candidate model` (status:todo)
  - Task ID: T04
  - Scope: In — `src/mutation.rs`: `Operator` enum with stable snake_case names, `MutationCandidate` (id, report path, line, column, start/end byte, operator, original, replacement) with serde serialization; constructor that verifies the byte range is in bounds and on UTF-8 char boundaries and that the original text matches the source slice, rejecting unchanged replacements; deduplication of identical edits while keeping distinct edits at the same range; deterministic sort; stable ID derivation; global limit selection returning total and selected counts; unit tests. Out — Tree-sitter queries, file discovery, rendering.
  - Dependencies: T01
  - Done when: unit tests with exact expected values prove range/UTF-8 validation, unchanged-replacement rejection, identical-edit dedup, same-location distinct-edit retention, stable sort and IDs independent of input order, and limit behavior (below, at, above total).
  - Verify: `nix build .#checks.<system>.skaza-tests`.
  - Context synchronization: pending

- [ ] T05: `Port the six Rust mutation operators and queries` (status:todo)
  - Task ID: T05
  - Scope: In — copy `swap_boolean.scm`, `negate_equality.scm`, `comparison_boundary.scm`, `swap_logical.scm`, `remove_not.scm`, `swap_predicate_method.scm` from `../ooze/queries/rust/` into `crates/skaza/queries/rust/`; `src/rust.rs`: compile the six queries once into a reusable struct via `include_str!`, parse each file once with `tree-sitter-rust`, run queries over the tree, map captures to `MutationCandidate`s through `match`-based replacement; unit tests for query compilation, every operator with exact candidates and byte ranges, and no matches in comments or strings. Out — test/macro/generated exclusions (T06), rendering.
  - Dependencies: T03, T04
  - Done when: each operator's fixture yields exactly the expected candidates; non-pair predicate methods yield nothing; queries compile in a dedicated test; comment and string fixtures yield zero candidates.
  - Verify: `nix build .#checks.<system>.skaza-tests .#checks.<system>.skaza-clippy`.
  - Context synchronization: pending

- [ ] T06: `Exclude test, assertion, panic, and generated code from discovery` (status:todo)
  - Task ID: T06
  - Scope: In — Tree-sitter-based exclusion in `src/rust.rs` (or a private submodule): skip captures inside `mod` items annotated `#[cfg(test)]` (inline and nested), functions annotated `#[test]`, invocations of `assert*`/`debug_assert*` and `panic`/`unreachable`/`todo`/`unimplemented` macros; confirm generated files never reach operators; unit tests with exact expected candidate lists for each rule including a production file containing an inline `#[cfg(test)] mod tests`. Out — any configuration flag for exclusions; non-Rust rules.
  - Dependencies: T05
  - Done when: every listed exclusion has a fixture proving production candidates remain while excluded-region candidates are absent.
  - Verify: `nix build .#checks.<system>.skaza-tests`.
  - Context synchronization: pending

- [ ] T07: `Render text and JSON discovery reports from the CLI` (status:todo)
  - Task ID: T07
  - Scope: In — `src/report.rs`: stable JSON document (discovery-only marker, `total`, `selected`, `limit`, ordered candidates) and text rendering (discovery-only header, total vs. selected, `path:line:column`, operator, `original -> replacement`); `src/lib.rs` discovery entrypoint and `src/main.rs` wiring that writes the report to stdout and diagnostics to stderr with non-zero exit on errors; empty-result handling; unit tests for JSON serialization and exact text output. Out — persistence, SARIF, history.
  - Dependencies: T06
  - Done when: both formats render exact expected output for fixture candidates and for zero candidates; JSON round-trips through `serde_json`; nothing but the report reaches stdout.
  - Verify: `nix build .#checks.<system>.skaza-tests`; `nix develop -c cargo run --manifest-path crates/skaza/Cargo.toml -- --src crates/skaza/src --format json` parses as JSON via `nix shell nixpkgs#jq -c jq .`.
  - Context synchronization: pending

- [ ] T08: `Add binary integration tests and dogfood against repository_identity` (status:todo)
  - Task ID: T08
  - Scope: In — `crates/skaza/tests/` tests that run the built `skaza` executable (`CARGO_BIN_EXE_skaza`) over a temporary Rust tree: repeated `--src`, text and JSON modes, identical JSON across two runs, identical selection across reversed `--src` order, `--limit` truncation with correct total/selected, byte-for-byte unchanged sources, zero-limit and invalid-path errors on stderr; fix any defects the dogfood run against `cli/src/services/repository_identity/mod.rs` exposes in Skaza code. Out — changes to any file under `cli/`.
  - Dependencies: T07
  - Done when: integration tests pass in the Nix sandbox; the dogfood command yields 1–10 candidates, all outside the `#[cfg(test)]` module, with identical JSON across two runs and a clean `git status`.
  - Verify: `nix build .#checks.<system>.skaza-tests`; run `nix develop -c cargo run --manifest-path crates/skaza/Cargo.toml -- --src cli/src/services/repository_identity/mod.rs --limit 10 --format json` twice and diff; `git status --porcelain`.
  - Context synchronization: pending

## Open questions

- Should `nix develop -c cargo test --manifest-path crates/skaza/Cargo.toml` be exempted from the repository's `cargo test` bash policy (for example via `satisfied_by` or a Skaza-specific rule), or is the `skaza-tests` flake check enough? The plan treats the flake check as canonical and does not change `.sce/config.json`.
