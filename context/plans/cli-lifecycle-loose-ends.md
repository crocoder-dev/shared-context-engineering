# Plan: cli-lifecycle-loose-ends

## Change summary

Resolve the lifecycle/AppContext loose ends captured in `loose.md` after the CLI service-lifecycle refactor. This is a follow-up plan to the completed `cli-service-lifecycle` plan, not a reopening of that completed plan. The goal is to make lifecycle orchestration use one runtime `AppContext`, resolve repository roots once per command path, remove redundant lifecycle provider construction, narrow dead-code suppression, and reduce `ServiceLifecycle` coupling to doctor/setup-owned types while preserving current `sce doctor` and `sce setup` behavior.

## Success criteria

- `doctor` and lifecycle providers no longer independently resolve the current directory for the same command path; repo-scoped lifecycle behavior uses a resolved repository root carried through `AppContext` when available.
- `setup` reuses the runtime command `AppContext` and a scoped repo-root variant instead of constructing an isolated `AppContext` with `NoopLogger` / `NullTelemetry`.
- `AppContext` capability fields (`fs`, `git`) and `repo_root` have real consumers or narrowly justified local allowances; broad dead-code suppressions are removed from lifecycle implementation files where no longer needed.
- Doctor and setup share one lifecycle provider catalog/factory instead of duplicating provider-list construction logic.
- `ServiceLifecycle` exposes lifecycle-owned result types or adapters so the trait is not directly anchored to doctor/setup modules as its public contract.
- Existing user-facing behavior remains stable for `sce doctor`, `sce doctor --fix`, and `sce setup` flows.
- Repository validation passes with the preferred checks: `nix flake check`; generated-output parity remains clean with `nix run .#pkl-check-generated`.

## Constraints and non-goals

- Do not add new CLI commands, flags, output modes, or user-facing behavior.
- Do not change doctor text/JSON output shape, problem taxonomy semantics, exit-code classes, or setup result wording except where tests require deterministic refactor-safe adjustments.
- Do not introduce new third-party dependencies.
- Do not remove lifecycle providers or collapse service-owned lifecycle modules back into command modules.
- Keep tasks as one-task/one-atomic-commit units; any task that uncovers independent behavior changes must stop and split before implementation.
- Prefer `nix flake check` over direct Cargo validation unless a narrow targeted check is needed during implementation.

## Task stack

- [x] T01: Share the lifecycle provider catalog (status:done)
  - Task ID: T01
  - Goal: Move lifecycle provider-list construction into one shared service-owned catalog/factory consumed by both `doctor` and `setup` orchestration.
  - Boundaries (in/out of scope): In - a shared function or small provider catalog type in the lifecycle/service layer, preserving provider order (`config`, `local_db`, and `hooks` where applicable). Out - changing provider behavior, changing doctor/setup output, or changing trait result types.
  - Done when: `doctor` and `setup` no longer maintain separate provider construction lists, provider ordering stays deterministic, and existing lifecycle aggregation still compiles.
  - Verification notes (commands or checks): Prefer `nix flake check`; if needed during development, run a narrow compile/check through the Nix dev shell.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/lifecycle.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/setup/command.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo check'`; `nix run .#pkl-check-generated`; `nix flake check` (all checks passed)
  - Notes: Lifecycle provider construction is centralized in `services::lifecycle::lifecycle_providers(include_hooks)`, preserving config → local_db → hooks ordering while letting setup omit hooks when not requested.

- [x] T02: Add scoped `AppContext` reuse helpers (status:done)
  - Task ID: T02
  - Goal: Add the minimal `AppContext` API needed for command paths to reuse runtime logger/telemetry/capability dependencies while attaching a resolved repository root.
  - Boundaries (in/out of scope): In - helper/accessor methods such as scoped repo-root cloning and capability accessors if needed by later tasks. Out - migrating setup/doctor/provider call sites and out - changing observability behavior.
  - Done when: command code can derive a repo-root-scoped context from the runtime context without constructing replacement logger/telemetry/capability objects, and unused-field allowances are narrowed where possible.
  - Verification notes (commands or checks): Prefer `nix flake check`; inspect compile warnings for dead-code allowances before proceeding to later tasks.
  - Completed: 2026-04-30
  - Files changed: `cli/src/app.rs`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/capability-traits.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo check'`; `nix run .#pkl-check-generated`; `nix flake check` (all checks passed)
  - Notes: `AppContext::with_repo_root(...)` now clones the runtime logger, telemetry, filesystem capability, and git capability while attaching a resolved repository root. Capability fields now have narrow method-level dead-code allowances on temporary accessors instead of field-level allowances.

- [x] T03: Route doctor lifecycle execution through a repo-root-scoped context (status:done)
  - Task ID: T03
  - Goal: Make the doctor command resolve the repository/current working context once and pass a repo-root-aware `AppContext` into lifecycle diagnosis/fix providers.
  - Boundaries (in/out of scope): In - doctor orchestration and provider calls needed so `ConfigLifecycle` / `HooksLifecycle` consume `ctx.repo_root()` for doctor diagnosis/fix paths. Out - setup command changes and out - doctor output/taxonomy changes.
  - Done when: doctor no longer causes config/hooks lifecycle providers to call `std::env::current_dir()` independently for the same diagnosis/fix pass, and outside-repo/bare-repo behavior remains reported through the existing taxonomy.
  - Verification notes (commands or checks): Prefer `nix flake check`; include targeted doctor smoke checks during implementation if behavior could drift (`sce doctor`, `sce doctor --fix --format json` where safe).
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/doctor/mod.rs`, `cli/src/services/config/lifecycle.rs`, `cli/src/services/hooks/lifecycle.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'`; `nix flake check` (all checks passed)
  - Notes: Doctor now resolves repository root once and passes a repo-root-scoped `AppContext` to lifecycle providers via `AppContext::with_repo_root()`. `ConfigLifecycle::diagnose()` and `HooksLifecycle::diagnose()`/`fix()` now use `ctx.repo_root()` instead of calling `std::env::current_dir()` independently.

- [x] T04: Reuse runtime `AppContext` in setup orchestration (status:done)
  - Task ID: T04
  - Goal: Replace setup's isolated `AppContext` construction with a repo-root-scoped context derived from the runtime command context.
  - Boundaries (in/out of scope): In - `SetupCommand::execute`, removal of `NoopLogger` / `NullTelemetry` setup-only scaffolding, and use of the shared provider catalog from T01. Out - setup target-selection behavior, install engine behavior, and hook/config asset changes.
  - Done when: setup lifecycle aggregation receives the runtime logger/telemetry/capability dependencies with `repo_root` populated, and setup output remains unchanged.
  - Verification notes (commands or checks): Prefer `nix flake check`; include a setup help/non-mutating smoke check where practical.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/setup/command.rs`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/service-lifecycle.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo check'`; `nix develop -c sh -c 'cd cli && cargo run -- setup --help'`; `nix run .#pkl-check-generated`; `nix flake check` (all passed)
  - Notes: `SetupCommand::execute()` now derives a repo-root-scoped `AppContext` from the runtime context via `AppContext::with_repo_root(...)`, preserving runtime logger/telemetry/capability dependencies for lifecycle setup providers and removing the setup-only `NoopLogger`/`NullTelemetry` scaffolding.

- [x] T05: Decouple `ServiceLifecycle` from doctor/setup-owned public result types (status:done)
  - Task ID: T05
  - Goal: Move the lifecycle trait contract to lifecycle-owned result types or explicit adapters so lifecycle providers are not publicly defined in terms of `doctor` and `setup` module types.
  - Boundaries (in/out of scope): In - lifecycle-owned health/fix/setup result types, adapter/conversion seams used by doctor/setup, and provider migration to the new contract. Out - changing doctor taxonomy semantics, setup outcome semantics, or public CLI output.
  - Done when: `services/lifecycle.rs` no longer imports doctor/setup modules for its trait method signatures, providers compile against the lifecycle-owned contract, and doctor/setup adapt results at their orchestration boundaries.
  - Verification notes (commands or checks): Prefer `nix flake check`; pay special attention to exact doctor JSON/text output stability and setup result rendering.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/lifecycle.rs`, `cli/src/services/config/lifecycle.rs`, `cli/src/services/hooks/lifecycle.rs`, `cli/src/services/local_db/lifecycle.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/setup/command.rs`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/service-lifecycle.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo check'`; `nix develop -c sh -c 'cd cli && cargo run -- doctor --format json'`; `nix develop -c sh -c 'cd cli && cargo run -- setup --help'`; `nix run .#pkl-check-generated`; `nix flake check` (all checks passed)
  - Notes: `ServiceLifecycle` now exposes lifecycle-owned health, fix, and setup result types. Config/hooks/local_db lifecycle providers no longer import doctor result types for trait implementations; doctor and setup convert lifecycle results at orchestration boundaries while preserving existing output semantics.

- [x] T06: Remove broad lifecycle dead-code suppression (status:done)
  - Task ID: T06
  - Goal: Remove file-level `#![allow(dead_code)]` from lifecycle implementation files and replace any still-required allowances with narrow item-level justification or real consumers.
  - Boundaries (in/out of scope): In - lifecycle files for config/hooks/local_db and related trait/type allowances made obsolete by T01-T05. Out - unrelated dead-code cleanup outside the lifecycle/AppContext surface.
  - Done when: lifecycle implementation files compile without broad file-level dead-code suppression, remaining allowances are item-local and justified by current extension seams, and no warnings are introduced.
  - Verification notes (commands or checks): Prefer `nix flake check`; if formatting changes are needed, use the repo's Nix-managed Rust formatting flow.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/config/lifecycle.rs`, `cli/src/services/hooks/lifecycle.rs`, `cli/src/services/local_db/lifecycle.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'`; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix run .#pkl-check-generated`; `nix flake check` (all checks passed)
  - Notes: Removed broad file-level dead-code suppression from lifecycle implementation files. The lifecycle files compile without replacement allowances or behavior changes.

- [x] T07: Validate behavior and sync context (status:done)
  - Task ID: T07
  - Goal: Run final validation, remove temporary scaffolding, and update durable context to reflect the cleaned-up lifecycle/AppContext architecture.
  - Boundaries (in/out of scope): In - `nix flake check`, `nix run .#pkl-check-generated`, relevant non-mutating CLI smoke checks, and context updates for current-state architecture/patterns/glossary/domain files. Out - new feature work or additional cleanup not required by this plan.
  - Done when: full validation passes, context files no longer describe stale lifecycle/AppContext behavior, and this plan records final evidence for all success criteria.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; non-mutating smoke checks for `sce doctor` / `sce setup --help` where practical.
  - Completed: 2026-04-30
  - Files changed: `context/plans/cli-lifecycle-loose-ends.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo run -- doctor --format json'`; `nix develop -c sh -c 'cd cli && cargo run -- setup --help'`; `nix run .#pkl-check-generated`; `nix flake check` (all checks passed)
  - Notes: Final validation completed without code changes. Durable context already reflects the cleaned-up lifecycle/AppContext architecture: lifecycle providers use lifecycle-owned result types, the shared provider catalog, and repo-root-scoped `AppContext` reuse for doctor/setup orchestration.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo run -- doctor --format json'` -> exit 0; JSON rendered successfully with `status: "ok"` and existing environment readiness details.
- `nix develop -c sh -c 'cd cli && cargo run -- setup --help'` -> exit 0; setup help rendered successfully.
- `nix run .#pkl-check-generated` -> exit 0; `Generated outputs are up to date.`
- `nix flake check` -> exit 0; `all checks passed!`

### Temporary scaffolding

- None introduced by this task; none removed.

### Success-criteria verification

- [x] `doctor` and lifecycle providers use repo-root-scoped `AppContext` where available -> confirmed by completed T03/T04 evidence and current context/code contracts.
- [x] `setup` reuses runtime command `AppContext` -> confirmed by completed T04 evidence and final validation.
- [x] AppContext/lifecycle loose-end cleanup is reflected in durable context -> confirmed by context sync verify-only pass over root and lifecycle domain context.
- [x] Doctor/setup share one lifecycle provider catalog -> confirmed by completed T01 evidence and current `context/cli/service-lifecycle.md` contract.
- [x] `ServiceLifecycle` exposes lifecycle-owned result types -> confirmed by completed T05 evidence and current lifecycle context.
- [x] Existing user-facing behavior remains stable for `sce doctor`, `sce doctor --fix`, and `sce setup` flows -> confirmed by non-mutating doctor/setup smoke checks plus `nix flake check`.
- [x] Preferred validation passes -> `nix run .#pkl-check-generated` and `nix flake check` both passed.

### Residual risks

- The local `sce doctor --format json` smoke output reports an environment-specific repo asset mismatch for `.opencode/opencode.json`; the command itself succeeds and this did not affect flake validation. No task-owned code or context drift remains.

## Open questions

None. The input `loose.md` is treated as the accepted loose-end inventory, and this plan covers all six listed items while preserving current CLI behavior.
