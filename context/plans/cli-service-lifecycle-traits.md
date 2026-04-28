# Plan: CLI service lifecycle traits

## Change summary

Refactor the Rust CLI service layer toward trait-based lifecycle capabilities so setup, doctor, and repair flows can call service-owned setup/diagnose/fix behavior through shared internal contracts. This plan starts with a minimal hooks lifecycle pilot, then leaves a repeatable pattern for later services without rewriting every CLI service in one pass.

## Success criteria

1. The CLI has an internal lifecycle capability contract for service-owned setup, diagnosis, repair, and action-preview behavior, with typed request/result models instead of ad hoc cross-service calls.
2. A hooks lifecycle service is the first production implementation and owns the required git-hook setup/diagnose/fix/preview behavior now shared by `sce setup --hooks` and `sce doctor --fix`.
3. A small capability registry lets command orchestration discover the hooks lifecycle capability by service ID instead of hardcoding hook-specific setup/doctor coupling at each call site.
4. Lifecycle capabilities receive explicit runtime/dependency context seams for git, filesystem/path resolution, config validation, and rendering-adjacent mapping so tests can use fakes without requiring broad filesystem/git fixtures.
5. Internal dry-run planning exists as typed preview/action-plan data, but no public `--dry-run` CLI flag or user-visible behavior change is introduced in this plan.
6. Existing `setup`, `doctor`, help, stderr, text, and JSON output contracts remain stable except for internal-only implementation changes.
7. The resulting architecture is documented in current-state `context/` files so future services can implement the same lifecycle traits one at a time.

## Constraints and non-goals

- Planning decision: create a new plan instead of reopening the completed `cli-architecture-full-refactor` plan.
- Planning decision: use a minimal hooks lifecycle pilot first; do not attempt to trait-convert every service in one change stream.
- Planning decision: include capability registry, typed reports, dependency injection, and internal dry-run/action planning.
- In scope: internal Rust refactors under `cli/src/services/**`, command call-site rewiring needed for the hooks pilot, tests for the new trait/registry/service seams, and current-state context updates.
- Out of scope: public `--dry-run` CLI flags, setup/doctor UX redesign, output wording cleanup, JSON schema changes, command additions/removals, new external dependencies, or converting the CLI into a multi-crate workspace.
- Out of scope: trait implementations for local DB, OpenCode assets, config, auth, or sync beyond extension notes/patterns that keep the hooks pilot reusable.
- Existing public functions may remain temporarily as thin compatibility wrappers when that keeps task boundaries small, but ownership should move toward the lifecycle capability seam.

## Task stack

- [x] T01: `Define lifecycle capability traits and typed models` (status:done)
  - Task ID: T01
  - Goal: Add the internal trait/model foundation for service-owned setup, diagnose, fix, and preview behavior.
  - Boundaries (in/out of scope): In - a focused internal module such as `cli/src/services/lifecycle.rs`, composable traits for lifecycle capabilities, service identity/metadata, typed setup/diagnostic/fix/action-preview result models, and dependency/context types needed by those traits. Out - hooks implementation, command rewiring, public CLI flags, output rendering changes, or broad service conversions.
  - Done when: the lifecycle module compiles; trait contracts can represent a service that supports any subset of setup/diagnose/fix/preview; result models are typed enough to map to existing setup and doctor outputs; unit tests cover basic model/trait expectations without filesystem or git dependencies.
  - Verification notes (commands or checks): Prefer `nix flake check`; targeted Rust check through Nix is acceptable while developing the new module, for example `nix develop -c sh -c 'cd cli && cargo test lifecycle'`.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/lifecycle.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test lifecycle'` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed.
  - Notes: Added the internal lifecycle trait/model foundation only; hooks implementation and setup/doctor rewiring remain deferred to later tasks.

- [x] T02: `Implement hooks lifecycle setup and preview capability` (status:done)
  - Task ID: T02
  - Goal: Introduce the hooks lifecycle service as the first capability implementation, initially covering canonical required-hook setup plus internal preview/action-plan data.
  - Boundaries (in/out of scope): In - a hooks lifecycle service/adaptor that reuses the canonical embedded hook assets and install engine, exposes setup/fix-compatible typed outcomes, and can produce an internal non-mutating action plan for missing/stale/non-executable required hooks. Out - changing `sce setup --hooks` output, moving doctor diagnosis, adding a public dry-run flag, or changing hook asset packaging semantics.
  - Done when: hooks setup behavior is available behind the lifecycle trait while existing setup helper functions can still delegate safely; preview/action-plan tests prove no mutation is required to describe intended hook actions; install outcomes still distinguish installed/updated/skipped states.
  - Verification notes (commands or checks): Targeted hooks/setup service tests through Nix during development; `nix flake check` before task completion.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/hooks_lifecycle.rs`, `cli/src/services/mod.rs`, `cli/src/services/setup.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test hooks_lifecycle'` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Added the hooks lifecycle service and non-mutating required-hook preview plan while preserving existing setup hook output/call compatibility. Classified as an important context-sync change because it adds the first concrete lifecycle capability; durable context was updated. Doctor diagnosis/repair rewiring remains deferred to later tasks.

- [x] T03: `Add capability registry and rewire setup hooks` (status:done)
  - Task ID: T03
  - Goal: Register the hooks lifecycle capability and make `sce setup --hooks` consume it through the registry instead of directly coupling command orchestration to hook install internals.
  - Boundaries (in/out of scope): In - a small static capability registry, hooks service registration, setup command/service call-site rewiring, and compatibility wrappers if needed for existing tests. Out - doctor rewiring, adding registry entries for non-hook services, changing setup config-target installation, or altering setup success text.
  - Done when: setup hook orchestration resolves the hooks capability through the registry/service ID path; existing `sce setup --hooks` behavior and output remain stable; registry tests cover successful lookup and missing-capability handling.
  - Verification notes (commands or checks): `nix flake check`; inspect setup-focused tests for preserved output and option compatibility behavior.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/lifecycle_registry.rs`, `cli/src/services/mod.rs`, `cli/src/services/setup.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test lifecycle_registry'` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Added the hook-only lifecycle registry, rewired setup hook execution through the registered hooks setup capability, and kept existing required-hook install wrappers for later doctor tasks. Classified as an important context-sync change because setup hook orchestration now consumes lifecycle capability registry lookup; durable context was updated.

- [x] T04: `Move doctor hook diagnosis through hooks lifecycle capability` (status:done)
  - Task ID: T04
  - Goal: Make doctor hook health inspection consume the hooks lifecycle diagnostic capability while preserving the existing doctor report/render contract.
  - Boundaries (in/out of scope): In - migrating hook-specific health collection/problem classification into the hooks lifecycle service or a trait-backed adapter, mapping typed lifecycle diagnostics back into existing doctor report types, and preserving stable problem category/fixability/remediation semantics. Out - doctor text/JSON output redesign, integration asset diagnosis changes, local DB diagnosis changes, or fix execution rewiring.
  - Done when: required-hook presence/executable/content checks are owned by the hooks lifecycle capability; `doctor/inspect.rs` no longer directly owns hook-domain inspection logic beyond report assembly/mapping; existing doctor output-shape tests still pass.
  - Verification notes (commands or checks): `nix flake check`; targeted doctor tests through Nix during development if needed.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/hooks_lifecycle.rs`, `cli/src/services/lifecycle_registry.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Added the hooks diagnostic lifecycle and diagnostic registry lookup, moved required-hook presence/executable/content checks into the hooks lifecycle module, and kept doctor responsible for repository targeting plus mapping lifecycle diagnostics into existing doctor report/problem types. Classified as an important context-sync change because doctor hook diagnosis ownership moved to the lifecycle capability; durable context was updated.

- [x] T05: `Move doctor hook repair through hooks lifecycle capability` (status:done)
  - Task ID: T05
  - Goal: Make `sce doctor --fix` use the registered hooks lifecycle fix capability and typed fix results instead of directly invoking setup-owned hook install functions.
  - Boundaries (in/out of scope): In - doctor fix-path rewiring for `hook_rollout` problems, mapping lifecycle fix outcomes to existing `DoctorFixResultRecord` values, preserving idempotent fixed/skipped/manual/failed vocabulary, and tests for supported hook repairs. Out - new fix classes, non-hook repair behavior, public dry-run flags, or changes to manual-only remediation.
  - Done when: doctor auto-fix obtains hook repair behavior through the lifecycle registry/capability; setup and doctor share the same hook service without importing each other's internals; fix-mode text/JSON remains stable.
  - Verification notes (commands or checks): `nix flake check`; inspect doctor fix tests/output assertions for preserved result vocabulary.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/hooks_lifecycle.rs`, `cli/src/services/lifecycle_registry.rs`, `cli/src/services/doctor/fixes.rs`, `cli/src/services/doctor/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Added the hooks fix lifecycle and fix registry lookup, moved doctor hook-rollout auto-repair through the registered hooks lifecycle capability, and mapped lifecycle fix actions back to existing doctor fix-result vocabulary/details. Classified as an important context-sync change because doctor hook repair ownership moved to the lifecycle capability; durable context was updated.

- [x] T06: `Sync lifecycle architecture context and extension guide` (status:done)
  - Task ID: T06
  - Goal: Update durable context to describe the trait-based lifecycle architecture, hooks pilot ownership, and the pattern future services should follow.
  - Boundaries (in/out of scope): In - current-state updates to relevant files such as `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/cli/cli-command-surface.md`; extension guidance for likely next candidates such as local DB and OpenCode assets. Out - historical progress narration, marking the plan as complete, or documenting unimplemented services as current runtime behavior.
  - Done when: context states that hooks are the implemented lifecycle pilot; setup/doctor ownership boundaries match code truth; future-service guidance is current-state oriented and does not overclaim unimplemented traits.
  - Verification notes (commands or checks): Review context against code truth; `nix run .#pkl-check-generated` if generated context/config surfaces are touched; otherwise note why generated-output parity is unaffected.
  - Completed: 2026-04-28
  - Files changed: `context/cli/service-lifecycle.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/overview.md`
  - Evidence: `nix run .#pkl-check-generated` passed (generated outputs unaffected by context-only changes); `nix flake check` passed (no code changes).
  - Notes: Context-only task. Updated all durable context files to reflect the implemented lifecycle architecture: trait/model foundation, hooks lifecycle service with all four facets (setup, diagnose, fix, preview), hook-only registry, setup/doctor rewiring through registry, and extension pattern for future services. No code changes, no generated output changes.

- [x] T07: `Run full validation and cleanup` (status:done)
  - Task ID: T07
  - Goal: Perform final repository validation, remove accidental temporary scaffolding, and verify code/context alignment for the lifecycle-trait pilot.
  - Boundaries (in/out of scope): In - full repo validation, cleanup of temporary shims/TODOs introduced during this plan, confirmation that no public dry-run flag or output drift slipped in, and final context-sync verification. Out - adding more lifecycle service implementations or opportunistic architecture rewrites.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no unintended temporary scaffolding remains; existing setup/doctor behavior is confirmed stable; context accurately reflects the implemented hooks lifecycle pilot.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted review that setup/doctor text+JSON contracts were not intentionally changed; verify plan task statuses/evidence are updated by the executor.
  - Completed: 2026-04-28
  - Files changed: `context/plans/cli-service-lifecycle-traits.md` (task status update only)
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks); no TODO/FIXME/HACK/TEMP scaffolding found in lifecycle modules (`lifecycle.rs`, `hooks_lifecycle.rs`, `lifecycle_registry.rs`); `#![allow(dead_code)]` in `lifecycle.rs` and `hooks_lifecycle.rs` is intentional and documented in context; no `dry_run`/`dry-run`/`DryRun` references exist in CLI source; setup/doctor consume `LifecycleRegistry`/`HOOKS_SERVICE_ID` confirmed in code; context files accurately reflect the implemented hooks lifecycle pilot.
  - Notes: Validation-only task. No code changes, no generated output changes, no scaffolding cleanup needed. All seven plan tasks are now complete.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all CLI tests, clippy, fmt, pkl-parity, integrations-install tests, JS checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (generated outputs are up to date)

### Temporary scaffolding
- No temporary scaffolding found in lifecycle modules (`lifecycle.rs`, `hooks_lifecycle.rs`, `lifecycle_registry.rs`)
- `#![allow(dead_code)]` in `lifecycle.rs` and `hooks_lifecycle.rs` is intentional and documented in context as "localized dead-code allowance for lifecycle facets whose command consumers are planned separately"

### Success-criteria verification
- [x] SC1: Internal lifecycle capability contract with typed request/result models -> confirmed: `cli/src/services/lifecycle.rs` defines `ServiceId`, `ServiceMetadata`, `LifecycleContext`, `SetupRequest`/`SetupReport`, `DiagnoseRequest`/`DiagnosticReport`, `FixRequest`/`FixReport`, `PreviewRequest`/`ActionPlan`, `LifecycleAction`, `DiagnosticRecord`, `LifecycleOutcome`, `DiagnosticSeverity`, `DiagnosticFixability`, and composable traits `SetupLifecycle`, `DiagnosticLifecycle`, `FixLifecycle`, `PreviewLifecycle` extending `LifecycleService`
- [x] SC2: Hooks lifecycle service owns setup/diagnose/fix/preview behavior -> confirmed: `cli/src/services/hooks_lifecycle.rs` implements all four lifecycle facets for the `hooks` service ID; `sce setup --hooks` consumes setup through registry; doctor consumes diagnostics and fix through registry
- [x] SC3: Capability registry for service ID discovery -> confirmed: `cli/src/services/lifecycle_registry.rs` provides `setup_lifecycle`, `diagnostic_lifecycle`, `fix_lifecycle` lookups by `ServiceId`; setup and doctor resolve hooks capability by service ID instead of direct coupling
- [x] SC4: Explicit runtime/dependency context seams -> confirmed: `LifecycleContext` carries optional repository, config, and state roots; tests use fake services without filesystem/git fixtures
- [x] SC5: Internal dry-run planning exists, no public --dry-run flag -> confirmed: `PreviewLifecycle` trait and `ActionPlan` model exist; no `dry_run`/`dry-run`/`DryRun` references in CLI source
- [x] SC6: Existing setup/doctor/help/stderr/text/JSON output contracts remain stable -> confirmed: `nix flake check` passes all tests; setup/doctor consume lifecycle through registry while preserving existing output vocabulary
- [x] SC7: Architecture documented in current-state context files -> confirmed: `context/cli/service-lifecycle.md`, `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` all reflect the implemented lifecycle architecture with extension guidance for future services

### Residual risks
- None identified.

## Open questions

- None at planning time. Resolved decisions: new plan, hooks lifecycle pilot, internal dry-run preview only, and preserve existing CLI behavior/output.
