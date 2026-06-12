# Finish Static DI Migration

## Change summary

Finish the Rust CLI static dependency-injection migration by removing the remaining runtime-dispatch seams from public capability accessors, lifecycle context boundaries, and outcome rendering support.

This is a follow-up to the completed `compile-time-di-migration` plan. That plan successfully replaced boxed command dispatch with the static `RuntimeCommand` enum and replaced boxed lifecycle provider aggregation with a static `LifecycleProvider` enum, but code review identified remaining hybrid seams:

- `AppContext<L, T, F, G>` is generic, but `logger()`, `fs()`, `git()`, and `telemetry()` helpers still return `&dyn ...` trait objects.
- `HasLogger`, `HasFs`, `HasGit`, and `HasTelemetry` expose object-safe return types instead of associated concrete capability types.
- `ServiceLifecycle` and `LifecycleProvider` still use `&dyn HasRepoRoot` at the lifecycle context boundary.
- `RunOutcome` owns `Option<services::observability::Logger>`, coupling outcome rendering to the production logger type.
- The static `RuntimeCommand` enum dispatcher should be preserved as the successful part of the migration.

## Success criteria

- `HasLogger`, `HasFs`, `HasGit`, and `HasTelemetry` use associated types and return `&Self::{Capability}` instead of `&dyn ...`.
- Inherent `AppContext` helpers preserve concrete generic capability types where exposed internally, instead of erasing them back to trait objects.
- `ServiceLifecycle`, concrete lifecycle providers, `LifecycleProvider` enum dispatch, and doctor/setup lifecycle aggregation no longer take `&dyn HasRepoRoot`; they use generic `HasRepoRoot` bounds or another compile-time-typed equivalent.
- `RunOutcome` / render support no longer hardcodes `services::observability::Logger` as the only logger carrier; tests and future callers can use the logger trait boundary without production-type coupling.
- Existing CLI behavior, stdout/stderr contracts, exit-code classification, observability logging semantics, setup/doctor behavior, and command output remain unchanged.
- The static `RuntimeCommand` enum dispatcher remains in place; no boxed command dispatch is reintroduced.
- No new third-party dependencies are introduced.
- Current-state context files describing capability traits, `AppContext`, lifecycle providers, and observability/rendering are updated after code changes.
- Repository-preferred validation passes.

## Constraints and non-goals

- Planning only; this plan does not approve implementation.
- Keep the change internal to architecture and typing; do not change public CLI command names, flags, help text, stdout/stderr payloads, or error-code taxonomy.
- Do not reintroduce `Box<dyn RuntimeCommand>`, boxed lifecycle providers, or dynamic command/plugin loading.
- Do not broaden the migration into unrelated trait-object uses such as closure trait objects, external library trait objects, or intentionally object-safe boundaries outside the app/lifecycle/rendering DI path.
- Treat each executable task as one atomic commit unit.

## Assumptions

- The desired end state is a more consistently static DI architecture, not removal of every `dyn` in the codebase.
- If a narrow runtime boundary is still desirable for a specific helper, implementation should preserve it only with an explicit local justification and without weakening the associated-type accessor contract.
- `RunOutcome` may be made generic, may hold a borrowed logger, or may move error logging earlier in the lifecycle; the accepted solution is whichever keeps observable behavior stable while removing the production-logger type coupling.

## Task stack

- [x] T01: `Add associated capability types to AppContext accessors` (status:done)
  - Task ID: T01
  - Goal: Convert the narrow capability accessor traits and `AppContext` helpers from object-returning APIs to associated-type APIs that preserve concrete generic dependency types.
  - Boundaries (in/out of scope): In - `cli/src/app.rs` traits `HasLogger`, `HasTelemetry`, `HasFs`, `HasGit`; their `AppContext` implementations; direct call sites that need updated type inference or trait bounds. Out - lifecycle `HasRepoRoot` dyn usage, `RunOutcome`, command enum behavior, or service business logic changes.
  - Done when: the accessor traits define associated types (for example `type Logger: LoggerTrait`) and return `&Self::Logger`; `AppContext` inherent helpers no longer erase logger/fs/git/telemetry to `&dyn ...`; command execution and hook logging call sites compile with the new associated-type bounds.
  - Verification notes (commands or checks): Run the narrow CLI compile/test check through Nix; inspect `cli/src/app.rs` for stale `fn logger(&self) -> &dyn LoggerTrait`, `fn fs(&self) -> &dyn FsOps`, `fn git(&self) -> &dyn GitOps`, and `fn telemetry(&self) -> &dyn Telemetry` accessor signatures.
  - Completed: 2026-06-12
  - Files changed: `cli/src/app.rs`; focused context sync in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and `context/cli/capability-traits.md`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt --print-out-paths` succeeded; `nix run .#pkl-check-generated` passed; targeted stale-signature search found no matching object-returning accessor signatures in `cli/src/app.rs`.
  - Notes: `nix flake check` was attempted but currently fails in unrelated `config-lib-biome-format` TypeScript formatting output, outside the T01 Rust accessor scope.

- [ ] T02: `Remove dyn HasRepoRoot from lifecycle dispatch` (status:todo)
  - Task ID: T02
  - Goal: Make lifecycle provider methods and aggregation compile-time typed over `HasRepoRoot` instead of accepting `&dyn HasRepoRoot`.
  - Boundaries (in/out of scope): In - `cli/src/services/lifecycle.rs`, lifecycle provider implementations under `cli/src/services/*/lifecycle.rs`, and doctor/setup aggregation call sites that pass repo-root-scoped contexts. Out - changing lifecycle health taxonomy, provider order, setup outcomes, doctor rendering, or hook install behavior.
  - Done when: `ServiceLifecycle` defaults, concrete lifecycle implementations, `LifecycleProvider::{diagnose,fix,setup}`, and doctor/setup lifecycle helpers no longer require `&dyn HasRepoRoot`; deterministic provider order and existing setup/doctor behavior are preserved.
  - Verification notes (commands or checks): Run doctor/setup/lifecycle-relevant CLI tests through Nix; inspect `cli/src/services` for remaining `&dyn HasRepoRoot` in lifecycle dispatch paths and justify any non-lifecycle leftovers if present.

- [ ] T03: `Decouple RunOutcome rendering from production Logger` (status:todo)
  - Task ID: T03
  - Goal: Remove the production `services::observability::Logger` type coupling from outcome rendering while preserving classified-error logging and stdout/stderr behavior.
  - Boundaries (in/out of scope): In - `cli/src/services/app_support.rs`, `cli/src/app.rs` runtime/outcome construction, and tests that construct or render `RunOutcome`. Out - changing `ClassifiedError`, logger event semantics, command output rendering, or observability configuration resolution.
  - Done when: `RunOutcome` no longer has `logger: Option<services::observability::Logger>` as a concrete field; rendering can operate over a logger trait/associated type/borrrowed logger strategy without changing user-visible output; classified errors and stdout-write failures are still logged once when a logger is available.
  - Verification notes (commands or checks): Run app-level stdout/stderr/error-classification tests through Nix; inspect `app_support.rs` for production-logger hardcoding in `RunOutcome` and for unchanged error logging paths.

- [ ] T04: `Sync static DI context documentation` (status:todo)
  - Task ID: T04
  - Goal: Update durable context so future sessions see the completed static DI architecture rather than the hybrid associated-type/dyn boundary.
  - Boundaries (in/out of scope): In - focused updates to `context/overview.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/capability-traits.md`, `context/cli/service-lifecycle.md`, and `context/sce/cli-observability-contract.md` where code truth changed. Out - broad context rewrites unrelated to static DI, historical plan cleanup, or new decision records unless implementation uncovers an architecture decision that must be recorded.
  - Done when: context describes associated-type capability accessors, generic lifecycle `HasRepoRoot` dispatch, and production-decoupled outcome rendering accurately; stale current-state claims about object-returning AppContext helpers or `&dyn HasRepoRoot` lifecycle boundaries are removed.
  - Verification notes (commands or checks): Search context for stale phrases such as `&dyn HasRepoRoot`, object-returning accessor signatures, and `RunOutcome` production logger coupling; confirm remaining mentions are historical or explicitly scoped.

- [ ] T05: `Run final validation and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Validate the completed follow-up, remove temporary scaffolding, and record final evidence in this plan.
  - Boundaries (in/out of scope): In - repository-preferred validation, generated-output parity check, stale dynamic-dispatch search, final plan evidence/status updates, and review for accidental CLI behavior drift. Out - new static-DI refactors beyond fixes required for validation or explicitly documented follow-up recommendations.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; searches show no stale app/lifecycle/rendering DI object boundaries targeted by this plan; this plan records validation evidence and any remaining risks.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect final diff for unrelated changes and verify no boxed command dispatch or boxed lifecycle provider aggregation was reintroduced.

## Open questions

- None blocking. The implementation should preserve the static `RuntimeCommand` enum dispatcher and treat any non-target `dyn` usage as out of scope unless it directly blocks the accessor, lifecycle, or outcome-rendering success criteria above.
