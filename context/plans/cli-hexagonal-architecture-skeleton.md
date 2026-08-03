# Plan: cli-hexagonal-architecture-skeleton

## Change summary

Introduce explicit internal hexagonal architecture boundaries inside the existing
single-crate `sce` CLI (`cli/`): new `domain`, `application`, `adapters`, and
`composition` modules under `cli/src/`, a `composition::run` entrypoint that
`main.rs` calls in place of `app::run`, a deterministic shell-based architecture
check (`scripts/check-cli-architecture.sh`) that enforces the permanent
dependency restrictions on `domain` and `application`, fixture-backed tests
proving the check rejects and accepts the required cases, a new root Nix check
wiring the architecture check into `nix flake check`, and a focused new section
in `context/architecture.md` documenting the four layers, their dependency
rules, and the transitional role of `services`.

This is new structure, not a migration: `app.rs`, `cli_schema.rs`,
`command_surface.rs`, and every file under `cli/src/services/` are untouched
except for `main.rs`'s one-line routing change. `composition::run` delegates to
the existing `app::run` runtime unchanged, so CLI behavior, exit codes, output,
and the public command surface do not change.

## Acceptance criteria

- [x] AC1: The CLI remains one Cargo package and exposes explicit internal
  modules for `domain`, `application`, `adapters`, and `composition`.
  - Validate: `test -f cli/src/domain/mod.rs && test -f cli/src/application/mod.rs && test -f cli/src/adapters/mod.rs && test -f cli/src/composition.rs`
- [x] AC2: `main.rs` calls `composition::run`, which delegates to the existing
  `app` runtime without changing behavior.
  - Validate: `grep -n "composition::run" cli/src/main.rs` and `cargo test --manifest-path cli/Cargo.toml`
- [x] AC3: `context/architecture.md` documents the four internal layers, the
  allowed dependency direction, the transitional role of `services`, and the
  vertical-slice migration strategy.
  - Validate: `grep -n "Hexagonal" context/architecture.md` and `grep -n "services" context/architecture.md`
- [x] AC4: The architecture check fails when domain code imports `crate::adapters`,
  `crate::application`, `crate::composition`, `crate::services`, or a forbidden
  infrastructure dependency (including `std::fs`, `std::env`, `std::process`).
  - Validate: `scripts/check-cli-architecture.sh` against the real tree, plus
    `scripts/test-check-cli-architecture.sh` negative fixtures for domain
    violations
- [x] AC5: The architecture check fails when application code imports
  `crate::adapters`, `crate::composition`, `crate::services`, or a forbidden
  infrastructure dependency (including `std::fs`, `std::process`).
  - Validate: `scripts/check-cli-architecture.sh` against the real tree, plus
    `scripts/test-check-cli-architecture.sh` negative fixtures for application
    violations
- [x] AC6: The check accepts the required positive cases: domain code using
  `std::path::PathBuf`, application code importing `crate::domain`, adapter
  code importing `crate::application`, and composition code delegating to the
  legacy `app` module.
  - Validate: `scripts/test-check-cli-architecture.sh`
- [x] AC7: Existing CLI tests, formatting, linting, and the generated-asset
  pipeline continue to pass unchanged.
  - Validate: `nix flake check`

### Full validation

- `nix flake check`
- `cargo test --manifest-path cli/Cargo.toml`
- `scripts/check-cli-architecture.sh`
- `scripts/test-check-cli-architecture.sh`

### Context sync

- `context/architecture.md` (new hexagonal architecture section; already an
  in-scope task deliverable, not a post-hoc sync)
- `context/cli/cli-command-surface.md` (note the new module-boundary layer if
  its module-boundary description would otherwise go stale)

## Constraints and non-goals

- **In scope:** `cli/src/main.rs`, new `cli/src/composition.rs`, new
  `cli/src/domain/`, `cli/src/application/`, `cli/src/adapters/` module trees,
  `scripts/check-cli-architecture.sh`, `scripts/test-check-cli-architecture.sh`,
  `flake.nix` (one new check derivation), `context/architecture.md`.
- **Out of scope:** any change to `cli/src/services/**`, `cli/src/app.rs`,
  `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/build.rs`,
  command behavior, help output, exit codes, JSON output, generated asset
  paths, `AppContext`, `ServiceLifecycle`, `CommandRegistry`.
- **Constraints:** single Cargo package, unchanged package/binary name, no new
  crate dependencies, no dynamic dispatch, no boxed service registries, new
  modules default to `mod` (crate-private) visibility with a narrow surface per
  layer, the architecture check must run without network access and require no
  tool beyond what the repository environment already has (bash/grep/find).
- **Non-goal:** migrating any command, service, or domain logic out of
  `services/**` in this phase. `application/ports` and `domain` stay
  documentation-only beyond the minimum structural types needed to compile;
  no speculative `FileSystem`/`Database`/`HttpClient`/`Clock`/`Logger` traits.
- **Non-goal:** a general-purpose architecture-rule engine. The check is a
  deterministic shell script covering exactly the permanent restrictions on
  `domain` and `application`; it does not enforce rules on `adapters` or
  `composition`, which may transitionally depend on `services`.

## Task stack

- [x] T01: `Add domain/application/adapters module skeleton and composition root` (status:done)
  - Task ID: T01
  - Goal: Create the four internal layer modules (`domain`, `application` with
    `error.rs`/`ports/`/`use_cases/`, `adapters` with `inbound/cli/` and
    `outbound/`, and `composition.rs`) as crate-private modules with
    module-level documentation only, wire them into `main.rs`, and make
    `composition::run` delegate to `app::run`. Update `main.rs` to call
    `composition::run` instead of `app::run`.
  - Boundaries (in/out of scope): In — new module files/dirs under
    `cli/src/{domain,application,adapters}/`, new `cli/src/composition.rs`,
    the `mod` declarations and one-line dispatch change in `cli/src/main.rs`.
    Out — any change to `app.rs`'s internals, `services/**`, or runtime
    initialization order.
  - Dependencies: none
  - Done when: `cargo build --manifest-path cli/Cargo.toml` succeeds with the
    new modules present and unused-code-clean (module docs only, no dead
    code warnings), `main.rs` calls `composition::run`, and
    `cargo test --manifest-path cli/Cargo.toml` passes with no behavior change.
  - Verification notes (commands or checks): `cargo build --manifest-path cli/Cargo.toml`; `cargo test --manifest-path cli/Cargo.toml`; `cargo clippy --manifest-path cli/Cargo.toml --all-targets --all-features`; `./sce --help` (via `cargo run --manifest-path cli/Cargo.toml -- --help`) output unchanged versus current `main`.
  - Implementation evidence: Added crate-private `cli/src/domain/mod.rs`,
    `cli/src/application/{mod.rs,error.rs,ports/mod.rs,use_cases/mod.rs}`,
    `cli/src/adapters/{mod.rs,inbound/mod.rs,inbound/cli/mod.rs,outbound/mod.rs}`
    (module-doc-only, no items), and `cli/src/composition.rs` with
    `pub(crate) fn run` delegating unchanged to `crate::app::run`. `main.rs`
    now declares the four new `mod` items and calls `composition::run(std::env::args())`
    in place of `app::run(...)`. `app.rs`, `cli_schema.rs`, `command_surface.rs`,
    and `services/**` are untouched.
  - Verification commands and outcomes: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`
    (clean build, no warnings); `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
    (183 passed, 0 failed); `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`
    (clean); `./scripts/run-cli-cargo.sh run --manifest-path cli/Cargo.toml -- --help`
    (banner/usage/commands output unchanged).
  - Deviations/assumptions: Used `./scripts/run-cli-cargo.sh` (the repository's
    canonical Cargo boundary, per `context/patterns.md`) in place of bare
    `cargo`, since repository builds require the `SCE_CLI_GENERATED_INPUT_DIR`
    handoff that only this wrapper (or Nix) provides; this does not change
    scope or command intent.

- [x] T02: `Add the deterministic architecture validation script` (status:done)
  - Task ID: T02
  - Goal: Add `scripts/check-cli-architecture.sh`, a network-free, tool-free
    (bash/grep/find only) script that scans `cli/src/domain/**/*.rs` and
    `cli/src/application/**/*.rs`, flags full-line-comment matches only on a
    best-effort basis, and fails with the offending file, line, and matched
    dependency for any forbidden import listed in the plan's dependency rules
    (domain: `clap`, `turso`, `reqwest`, `inquire`, `keyring_core`, `std::fs`,
    `std::env`, `std::process`, `crate::adapters`, `crate::application`,
    `crate::composition`, `crate::services`; application: the same set minus
    `std::env` and `crate::application`). It must not flag `std::path::Path`,
    `std::path::PathBuf`, `std::time::Duration`, or ordinary collection/
    formatting imports. Support an override (e.g. `CLI_ARCH_CHECK_ROOT`) so
    tests can point it at a fixture tree instead of the real repository.
  - Boundaries (in/out of scope): In — the check script only. Out — wiring it
    into Nix (T04), fixtures/tests (T03), and any rule enforcement on
    `adapters`/`composition` (explicitly not required this phase).
  - Dependencies: T01
  - Done when: `scripts/check-cli-architecture.sh` exits `0` against the real
    `cli/src/domain` and `cli/src/application` trees produced by T01, and
    exits non-zero with a deterministic offending-file/line message when run
    against a manually constructed violation (verified ad hoc during this
    task; the durable fixture proof is T03).
  - Verification notes (commands or checks): `scripts/check-cli-architecture.sh`; manual run against a scratch temp copy with an injected `use crate::services;` line inside `cli/src/domain` to confirm non-zero exit and a clear diagnostic.
  - Implementation evidence: Added `scripts/check-cli-architecture.sh` (executable).
    It resolves an optional `CLI_ARCH_CHECK_ROOT` override (defaulting to the
    real repository root), scans `cli/src/domain/**/*.rs` and
    `cli/src/application/**/*.rs` via `find ... -print0`, skips lines that are
    entirely a `//` comment on a best-effort basis, and matches each layer's
    forbidden-token list with a boundary-guarded bash regex
    (`(^|[^A-Za-z0-9_:])token([^A-Za-z0-9_]|$)`) so `std::fs`/`crate::adapters`/
    etc. match only as whole path segments and never match
    `std::path::PathBuf`, `std::path::Path`, `std::time::Duration`, or other
    unrelated imports. On a match it prints `file:line: forbidden dependency
    in <layer> layer: <token>` to stderr and exits non-zero; with no
    violations it prints a one-line pass message and exits 0.
  - Verification commands and outcomes: `./scripts/check-cli-architecture.sh`
    against the real tree (exit 0, pass message); a scratch `mktemp -d` copy of
    `cli/src/domain` and `cli/src/application` with `use crate::services;`
    appended to `domain/mod.rs`, run via `CLI_ARCH_CHECK_ROOT=<tmp>
    ./scripts/check-cli-architecture.sh` (exit 1, printed
    `.../domain/mod.rs:7: forbidden dependency in domain layer:
    crate::services`); a second scratch fixture with a domain file using
    `std::path::PathBuf`, `std::path::Path`, `std::time::Duration`,
    `std::collections::HashMap`, and comment-only mentions of
    `crate::adapters`/`crate::services`, plus an application file importing
    `crate::domain` (exit 0, no false positives).
  - Deviations/assumptions: Diagnostic wording and exit-code convention follow
    the style of existing repository check scripts (e.g.
    `config/pkl/check-generated.sh`); the script reports every violation found
    in a single run rather than stopping at the first one, which remains
    deterministic for a given tree. Neither choice changes scope or command
    intent.

- [x] T03: `Add architecture-check fixtures and test script` (status:done)
  - Task ID: T03
  - Goal: Add `scripts/test-check-cli-architecture.sh`, following the existing
    `scripts/test-check-generated.sh` pattern (temp-dir fixture repo, no
    modification of real source), that proves the check rejects: a domain
    file importing `crate::adapters`, a domain file using `std::fs`, an
    application file importing `crate::services`, and an application file
    importing `turso`; and accepts: domain code using `std::path::PathBuf`,
    application code importing `crate::domain`, adapter code importing
    `crate::application`, and composition code delegating to the legacy `app`
    module.
  - Boundaries (in/out of scope): In — the test script and its fixtures. Out —
    modifying `scripts/check-cli-architecture.sh` behavior itself except to
    fix any incorrect diagnostic found while writing the tests; out — Nix
    wiring (T04).
  - Dependencies: T02
  - Done when: `scripts/test-check-cli-architecture.sh` exits `0` and its
    output shows all eight required assertions (4 reject, 4 accept) passing.
  - Verification notes (commands or checks): `scripts/test-check-cli-architecture.sh`
  - Implementation evidence: Added `scripts/test-check-cli-architecture.sh`
    (executable), mirroring `scripts/test-check-generated.sh`'s conventions
    (`mktemp -d` root with a `trap` cleanup, no modification of real source).
    It builds 8 isolated fixture trees under the temp root — one per required
    assertion — and drives `scripts/check-cli-architecture.sh` against each via
    `CLI_ARCH_CHECK_ROOT`, using `assert_reject` (non-zero exit plus an
    expected offending-file/token substring in output) and `assert_accept`
    (zero exit) helpers. Reject fixtures: domain file importing
    `crate::adapters`; domain file using `std::fs`; application file importing
    `crate::services`; application file importing `turso`. Accept fixtures:
    domain file using `std::path::PathBuf`; application file importing
    `crate::domain`; adapter file importing `crate::application`; composition
    file delegating to `crate::app`. The script tallies pass/fail counts and
    exits non-zero if any assertion fails.
  - Verification commands and outcomes: `./scripts/test-check-cli-architecture.sh`
    (all 8 assertions printed `PASS`, script exited 0 with summary
    "all 8 assertions passed"); `./scripts/check-cli-architecture.sh` against
    the real tree (unaffected, still exits 0 with its pass message).
  - Deviations/assumptions: No incorrect diagnostic surfaced while writing the
    tests, so `scripts/check-cli-architecture.sh` required no behavior change.
    Adapter and composition fixtures are included even though the check script
    does not scan those directories, satisfying the plan's four required
    accept assertions explicitly.

- [x] T04: `Wire the architecture check into root Nix checks` (status:done)
  - Task ID: T04
  - Goal: Add a new `runCommand`-based check derivation in `flake.nix`
    (following the existing `workflow-actionlint` pattern) that runs
    `scripts/check-cli-architecture.sh` against the repository tree and
    `scripts/test-check-cli-architecture.sh`, and register it under the root
    `checks.<system>` attribute set so `nix flake check` exercises it.
  - Boundaries (in/out of scope): In — one new check derivation and its
    registration in `flake.nix`. Out — any change to existing check
    derivations, devShells, or packages.
  - Dependencies: T03
  - Done when: `nix flake check` includes and passes the new check, and
    `nix flake check --print-build-logs 2>&1 | grep -i "cli-architecture"` shows
    it ran.
  - Verification notes (commands or checks): `nix flake check`
  - Implementation evidence: Added `cliArchitectureCheckSrc`
    (`pkgs.lib.fileset.toSource`, rooted at `workspaceRoot`) covering
    `scripts/check-cli-architecture.sh`, `scripts/test-check-cli-architecture.sh`,
    `cli/src/domain`, and `cli/src/application`. Added `cliArchitectureCheck`
    (`pkgs.runCommand "cli-architecture-check"`) that copies that source into
    a writable `./repo`, `patchShebangs ./repo/scripts` (the sandbox has no
    `/usr/bin/env`, matching the existing `pklGeneratedCheck` pattern), then
    runs `bash ./scripts/check-cli-architecture.sh` and
    `bash ./scripts/test-check-cli-architecture.sh` before `mkdir -p "$out"`.
    Registered it as `cli-architecture = cliArchitectureCheck;` in the root
    `checks` attribute set alongside `workflow-actionlint` and
    `native-portability-audit`.
  - Verification commands and outcomes: `nix flake check --print-build-logs`
    (all checks passed, including `checks.x86_64-linux.cli-architecture`);
    `nix flake check --print-build-logs 2>&1 | grep -i "cli-architecture"`
    (showed `checking derivation checks.x86_64-linux.cli-architecture...` and
    the evaluated derivation path).
  - Deviations/assumptions: Nix flake evaluation only sees git-tracked (or
    staged) files, so the previously untracked T01–T03 deliverables
    (`cli/src/{domain,application,adapters}/`, `cli/src/composition.rs`,
    `scripts/check-cli-architecture.sh`, `scripts/test-check-cli-architecture.sh`)
    and this task's `flake.nix`/`cli/src/main.rs` changes were staged with
    `git add` (no commit) so `nix flake check` could see them; this is a
    prerequisite for running the required verification, not a scope change.

- [x] T05: `Document the internal hexagonal architecture in context/architecture.md` (status:done)
  - Task ID: T05
  - Goal: Add a new `## CLI internal hexagonal architecture` section to
    `context/architecture.md` covering: the CLI stays one Cargo package;
    hexagonal architecture here is about dependency direction, not crate
    count; what each of `domain`, `application`, `adapters/inbound`,
    `adapters/outbound`, and `composition` owns; that `services` is a
    temporary compatibility namespace that new domain/application code must
    never depend on; that migration proceeds through vertical slices rather
    than a big-bang rewrite; the dependency diagram from the change request;
    and that adapter implementations depend inward on application-owned
    ports.
  - Boundaries (in/out of scope): In — `context/architecture.md` only. Out —
    any other `context/**` file (leave `context/cli/cli-command-surface.md`
    etc. as a follow-up if it turns out to need updating; check it, only edit
    if a specific claim there would now be misleading).
  - Dependencies: T04
  - Done when: `grep -n "Hexagonal" context/architecture.md` and
    `grep -n "services" context/architecture.md` both match, and the section
    covers all ten points listed in the change request.
  - Verification notes (commands or checks): `grep -n "Hexagonal" context/architecture.md`; `grep -n "services" context/architecture.md`; manual read-through against the ten required points.
  - Implementation evidence: Inserted a new `## CLI internal hexagonal
    architecture` section into `context/architecture.md` (after the
    `## Placeholder SCE CLI boundary` section, before
    `## CLI install/distribution boundary`) covering: single-Cargo-package
    framing and hexagonal-as-dependency-direction-not-crate-count; per-layer
    ownership for `domain`, `application` (`error.rs`/`ports/`/`use_cases/`),
    `adapters/inbound` (`cli/`), `adapters/outbound`, and `composition`
    (`composition::run` as `main.rs`'s sole entrypoint, currently delegating
    to `app::run`); the adapters-depend-inward-on-application-owned-ports
    rule; `services` as a temporary compatibility namespace domain/application
    code must never depend on, expected to shrink over time; the
    vertical-slice migration strategy versus a big-bang rewrite; a Mermaid
    dependency-direction diagram (adapters -> application -> domain,
    composition -> adapters/services, adapters -.transitional.-> services);
    and a closing paragraph describing `scripts/check-cli-architecture.sh`'s
    enforced forbidden-import lists and its `nix flake check` wiring.
    `context/cli/cli-command-surface.md` was reviewed and left unedited: it
    documents command-surface behavior, not the new internal module
    boundaries, so no claim there is now misleading.
  - Verification commands and outcomes: `grep -n "Hexagonal"
    context/architecture.md` (matched, line 87); `grep -n "services"
    context/architecture.md` (multiple matches including the new section);
    manual read-through of the new section against the ten required points
    (all present).
  - Deviations/assumptions: None.

## Open questions

None. The change request is a fully specified task with explicit target file
layout, dependency rules, forbidden-import lists, acceptance criteria with
proof commands, and an explicit non-goals list; there is no unresolved scope,
architecture, or ordering decision left to make.

## Validation Report

**Status:** validated  
**Date:** 2026-08-04

### Commands run

- `nix flake check --print-build-logs` -> exit 0 (all checks passed, including `checks.x86_64-linux.cli-architecture`)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (183 passed, 0 failed)
- `./scripts/check-cli-architecture.sh` -> exit 0 (pass message against real tree)
- `./scripts/test-check-cli-architecture.sh` -> exit 0 (all 8 assertions passed)
- `test -f cli/src/domain/mod.rs && test -f cli/src/application/mod.rs && test -f cli/src/adapters/mod.rs && test -f cli/src/composition.rs` -> exit 0
- `grep -n "composition::run" cli/src/main.rs` -> exit 0 (matched line 17)
- `grep -n "Hexagonal" context/architecture.md` -> exit 0 (matched)
- `grep -n "services" context/architecture.md` -> exit 0 (multiple matches)

### Scaffolding removed

- None. Fixtures used by `scripts/test-check-cli-architecture.sh` are generated at runtime via `mktemp -d` and cleaned up by its own trap; no committed temporary files remain.

### Success-criteria verification

- [x] AC1: Four internal module files exist -> `test -f` checks all passed.
- [x] AC2: `main.rs` calls `composition::run` -> matched at `cli/src/main.rs:17`; `cargo test` passed (183/183).
- [x] AC3: `context/architecture.md` documents the four layers, dependency direction, `services` role, migration strategy -> both greps matched; new `## CLI internal hexagonal architecture` section present.
- [x] AC4: Architecture check fails on forbidden domain imports -> `check-cli-architecture.sh` passes real tree; `test-check-cli-architecture.sh` domain-reject fixtures (crate::adapters, std::fs) both PASS.
- [x] AC5: Architecture check fails on forbidden application imports -> `test-check-cli-architecture.sh` application-reject fixtures (crate::services, turso) both PASS.
- [x] AC6: Check accepts required positive cases -> `test-check-cli-architecture.sh` accept fixtures (domain PathBuf, application->domain, adapter->application, composition->app) all PASS.
- [x] AC7: Existing tests, formatting, linting, generated-asset pipeline pass unchanged -> `nix flake check` exit 0, all checks including `cli-fmt`, `cli-generated-input`, `pkl-generated`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
