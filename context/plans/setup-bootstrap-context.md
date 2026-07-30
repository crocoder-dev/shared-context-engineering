# Plan: setup-bootstrap-context

## Change summary

Add an additive context bootstrap mode to `sce setup`. `sce setup --bootstrap-context` creates the baseline durable-context files and working directories without entering interactive integration setup, while every normal successful setup path also ensures the same baseline exists before continuing. Existing context files and directory contents remain untouched.

This closes a documented-but-unimplemented seam: durable workflow rules already name `sce setup --bootstrap-context` as the bootstrap boundary, while the setup CLI today only bootstraps repo-local config and lifecycle DBs. Path accessors for the baseline tree already live in `cli/src/services/default_paths.rs`; the change wires create-if-missing filesystem bootstrap, request routing, and focused tests on top of that catalog.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: `sce setup --bootstrap-context` creates `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/`, `context/handovers/`, `context/decisions/`, `context/tmp/`, and `context/tmp/.gitignore` in a Git repository without prompting for an integration target or installing integration assets.
  - Validate: Run the packaged CLI in a temporary initialized Git repository with `sce setup --bootstrap-context`, then assert that every listed path exists and no `.opencode/`, `.claude/`, or `.pi/` directory was created.
- [x] AC2: Context bootstrap is additive and idempotent: rerunning it fills missing baseline paths but does not overwrite existing context documents, plans, decisions, handovers, or scratch-ignore content.
  - Validate: Run the targeted bootstrap integration coverage that seeds sentinel content, removes selected baseline paths, reruns bootstrap, and asserts sentinel content is unchanged while missing paths are restored.
- [x] AC3: Normal `sce setup` modes ensure the baseline context tree exists in addition to their existing setup behavior.
  - Validate: Run targeted setup request/orchestration coverage for a normal non-interactive setup and assert the baseline context paths are created while the selected integration setup still runs.
- [x] AC4: `sce setup --help` documents `--bootstrap-context`, and parser/request conversion routes the flag to a deterministic context-only setup request when used alone.
  - Validate: Run targeted parser/help tests and inspect `sce setup --help` for `--bootstrap-context`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/sce/context-workflow-rules.md` with the implemented additive/idempotent bootstrap semantics.
- Update `context/sce/setup-repo-local-config-bootstrap.md` and `context/cli/cli-command-surface.md` with the setup flag, context-only behavior, and automatic normal-setup bootstrap.
- Update `context/cli/default-path-catalog.md` if the implementation adds a named `context/tmp/.gitignore` path accessor.
- Update `context/context-map.md` annotations when the setup/bootstrap ownership descriptions change.

## Constraints and non-goals

- **In scope:** The setup clap schema, setup request conversion and execution, canonical context paths in `cli/src/services/default_paths.rs`, additive baseline file/directory creation, deterministic help/output, and focused Rust/integration coverage.
- **Out of scope:** Generating repository-specific architecture or application knowledge, changing `/change-to-plan` workflow behavior, or modifying integration asset contents.
- **Constraints:** Keep path ownership in `default_paths.rs`; require the existing Git-repository gate; never overwrite existing context content; use repository Nix/Cargo wrapper conventions and stable CLI diagnostics.
- **Non-goal:** This change does not populate durable context with inferred codebase facts or turn `context/tmp/` into persisted runtime storage.

## Assumptions

- Used alone, `--bootstrap-context` is a non-interactive context-only setup mode; combining it with target or hook options is unnecessary because normal setup paths bootstrap context automatically.
- Missing individual baseline files or directories are repaired additively even when `context/` already exists.
- New Markdown files use deterministic neutral headings/placeholders, `context/context-map.md` links the baseline entry points and working directories without inventing repository details, and `context/tmp/.gitignore` ignores scratch content while retaining itself.
- Existing `RepoPaths` context accessors and constants in `default_paths.rs` are reused; a named accessor for `context/tmp/.gitignore` is added only if production code needs one.

## Task stack

- [x] T01: `Add additive context bootstrap to setup` (status:done)
  - Task ID: T01
  - Goal: Implement the dedicated `sce setup --bootstrap-context` path and make normal setup ensure the same durable-context baseline without changing existing content.
  - Boundaries (in/out of scope): In — canonical context path accessors/constants, baseline templates, additive/idempotent filesystem bootstrap, setup request/parser/help wiring, context-only execution, normal setup orchestration, and focused tests. Out — generated agent assets, application-specific context generation, and workflow-command changes.
  - Dependencies: none
  - Done when: The dedicated flag creates exactly the requested baseline without prompting or installing integrations; normal setup also ensures the baseline; reruns preserve existing content and restore missing paths; parser/help and filesystem behavior have focused automated coverage.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; targeted packaged-CLI smoke run in a temporary Git repository if the integration assertions are not fully covered by the Rust test target.
  - Completed: 2026-07-28
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/default_paths.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/command_registry.rs`, `cli/src/services/setup/mod.rs`, `cli/src/services/setup/command.rs`
  - Evidence: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` -> exit 0 (14 passed). Coverage includes resolve/parser routing for `--bootstrap-context`, help documentation, additive baseline creation without integration dirs, and idempotent restore of missing paths while preserving sentinel content.
  - Notes: `--bootstrap-context` is context-only and must be used alone; every normal successful setup path calls the same additive `bootstrap_context_baseline` after the Git gate. Added `RepoPaths::context_tmp_gitignore_file` for `context/tmp/.gitignore`.

- [x] T02: `Fix rustfmt drift in setup bootstrap code` (status:done)
  - Task ID: T02
  - Goal: Resolve the `cli-fmt` failure recorded in the prior Validation Report by formatting `cli/src/services/setup/mod.rs` with the project's rustfmt configuration, with no behavior change.
  - Boundaries (in/out of scope): In — running `cargo fmt` (or equivalent) over `cli/src/services/setup/mod.rs` and any other files it touches, confirming no functional diffs. Out — any change to bootstrap behavior, task T01 scope, or other services.
  - Dependencies: T01
  - Done when: `nix develop -c sh -c 'cd cli && cargo fmt --check'` reports no diffs and `nix flake check`'s `cli-fmt` derivation passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix develop -c sh -c 'cd cli && cargo fmt --check'`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
  - Completed: 2026-07-28
  - Files changed: `cli/src/services/setup/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` applied formatting-only changes (compact `with_context` closures, import order, multi-line literal formatting) with no functional diff; `nix build .#checks.x86_64-linux.cli-fmt --no-link` -> exit 0; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` -> exit 0 (14 passed).
  - Notes: `nix flake check` still reports an unrelated `cli-clippy` failure (`convert_setup_command` has 8/7 arguments in `cli/src/services/parse/command_runtime.rs:406`), pre-existing from T01 and out of scope for this formatting-only task.

## Open questions

None. This task closes the one outstanding failed check (`cli-fmt`) from the prior validation attempt; no new scope or ambiguity is introduced.

## Validation Report

**Status:** failed  
**Date:** 2026-07-28

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed; inventory sha256 f465ad7139a66f8530581186b8db77405afa7203be9a6ce9e6de9624e238cd0b)
- `nix flake check` -> exit 1 (`checks.x86_64-linux.cli-clippy` failed: `error: this function has too many arguments (8/7)` at `cli/src/services/parse/command_runtime.rs:406` in `convert_setup_command`; `-D clippy::too-many-arguments` implied by `-D clippy::all`)
- `nix build .#checks.x86_64-linux.cli-tests --no-link` -> exit 0 (180 passed; 0 failed; includes `bootstrap_context_baseline_creates_expected_paths` and `bootstrap_context_baseline_is_additive_and_idempotent`)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: `--bootstrap-context` creates the full baseline without integration assets -> packaged CLI smoke in temp Git repo (prior run); unit test `bootstrap_context_baseline_creates_expected_paths` (rerun, passing)
- [x] AC2: additive and idempotent bootstrap -> unit test `bootstrap_context_baseline_is_additive_and_idempotent` (rerun, passing; sentinels preserved, missing paths restored)
- [x] AC3: normal setup also ensures baseline while integration still runs -> packaged CLI `setup --pi --non-interactive` created full baseline and installed Pi (prior run); request resolution tests keep normal modes non-`context_only` (rerun, passing)
- [x] AC4: help and parser route `--bootstrap-context` -> unit tests `help_documents_bootstrap_context_flag`, `parser_routes_bootstrap_context_to_context_only_request`, `resolve_setup_request_accepts_bootstrap_context_alone` (rerun, passing); packaged `setup --help` lists the flag (prior run)

### Failed checks and follow-ups

- `nix flake check` / `cli-clippy`: `convert_setup_command` in `cli/src/services/parse/command_runtime.rs:406` has 8 parameters, exceeding clippy's `too_many_arguments` threshold of 7 under `-D clippy::all`; evidence: `nix build .#checks.x86_64-linux.cli-clippy` builder log (`error: this function has too many arguments (8/7)`); required: reduce the parameter count (e.g. group the setup flags into a request/options struct) or apply a scoped `#[allow(clippy::too_many_arguments)]` if the signature is intentional, then rerun full validation. This surfaced only now because the prior `cli-fmt` failure (fixed by T02) previously stopped `nix flake check` before it reached `cli-clippy`; it is pre-existing from T01, not introduced by T02, but it still blocks required full validation and must be resolved before the plan can validate.

### Residual risks

- Normal packaged setup without a repository identity (`origin` remote or `agent_trace.repository_id`) still fails later lifecycle steps after context baseline is ensured; that is pre-existing identity gating, not a bootstrap regression, but empty-repo smoke must supply identity when asserting end-to-end normal setup.

### Retry

After repairs, rerun:

`/validate context/plans/setup-bootstrap-context.md`
