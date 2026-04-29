# Plan: cli-service-lifecycle

## Change summary

Move ownership of environment health checks (`diagnose`), repairs (`fix`), and bootstrapping (`setup`) from the monolithic `doctor` and `setup` commands into the individual services that own each concern. Introduce a `ServiceLifecycle` trait with `diagnose`, `fix`, and `setup` methods. `doctor` becomes an aggregator that calls `diagnose` across all registered services; `setup` becomes an aggregator that calls `setup` across all registered services.

## Success criteria

- `ServiceLifecycle` trait exists in `services/lifecycle.rs`.
- `hooks`, `config`, and `local_db` services implement `ServiceLifecycle` with their own health checks and setup steps.
- `doctor` aggregates results from all lifecycle services instead of directly inspecting hooks/config/DB.
- `setup` aggregates setup steps from all lifecycle services instead of directly calling `bootstrap_local_db`, `bootstrap_repo_local_config`, etc.
- `nix flake check` passes.
- `sce doctor` and `sce doctor --fix` output is unchanged.
- `sce setup` behavior is unchanged.

## Constraints and non-goals

- **Do not** add new CLI commands or flags.
- **Do not** change the `doctor` problem taxonomy (`ProblemKind`, `ProblemCategory`, etc.); services produce the same types.
- **Do not** change setup asset packaging or embedded asset iteration.
- The `doctor` and `setup` command modules remain as orchestrators; they do not disappear.

## Assumptions

- `AppContext` from `cli-observability-di` is available as the shared context passed to lifecycle methods. If not yet implemented, we will use a minimal `&dyn Logger` parameter as a stand-in and migrate later.
- The existing `DoctorDependencies` pattern (function-pointer injection) in `doctor/mod.rs` is the local precedent for abstraction; this plan replaces it with `&AppContext` plus `FsOps`/`GitOps` capabilities.
- `local_db` already has a health-check seam (`LocalDb::new()`); we will expose it through `ServiceLifecycle`.
- `context/decisions/cli-refactor-decisions.md` is the decision record for this plan; it chooses a single `ServiceLifecycle` trait with default no-op methods.

## Task stack

- [x] T01: Define `ServiceLifecycle` trait against `AppContext` (status:done)
  - Task ID: T01
  - Goal: Create `services/lifecycle.rs` with a single `ServiceLifecycle` trait containing default no-op `diagnose`, `fix`, and `setup` methods that accept `&AppContext`. Define shared result types (`HealthProblem`, `FixResult`, `SetupOutcome`) if they don't already exist in `doctor/types.rs`.
  - Boundaries (in/out of scope): In - trait definition, shared result types. Out - implementing the trait for any service.
  - Done when: `services/lifecycle.rs` compiles, trait is usable, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/lifecycle.rs`, `cli/src/services/mod.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/types.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed.
  - Notes: Added a default no-op `ServiceLifecycle` trait against `AppContext`, a minimal `SetupOutcome`, and crate-visible doctor result/taxonomy types for future lifecycle implementations without changing runtime behavior.

- [x] T02: Extract `hooks` health checks and setup into `ServiceLifecycle` impl (status:done)
  - Task ID: T02
  - Goal: Move hook-specific doctor inspection logic (from `doctor/inspect.rs`: `inspect_repository_hooks`, `collect_hook_health`, hook content state checks) and hook-specific setup logic (`install_required_git_hooks`) into `services/hooks/lifecycle.rs` as a `ServiceLifecycle` implementation.
  - Boundaries (in/out of scope): In - moving hook health checks and setup into `hooks/lifecycle.rs`, producing `DoctorProblem` / `SetupInstallOutcome` compatible results. Out - removing them from `doctor/inspect.rs` yet (that happens in T04).
  - Done when: `hooks/lifecycle.rs` compiles, implements `ServiceLifecycle`, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/hooks/lifecycle.rs`, `cli/src/services/hooks/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed.
  - Notes: Added a hook-owned `HooksLifecycle` provider with diagnose/fix/setup methods that reuse hook rollout problem taxonomy and canonical required-hook installation while leaving current doctor/setup orchestration unchanged for later tasks.

- [ ] T03: Extract `config` health checks and setup into `ServiceLifecycle` impl (status:todo)
  - Task ID: T03
  - Goal: Move config-specific doctor inspection logic (global config validation, local config validation) and config-specific setup logic (`bootstrap_repo_local_config`) into `services/config/lifecycle.rs` as a `ServiceLifecycle` implementation.
  - Boundaries (in/out of scope): In - moving config health checks and setup into `config/lifecycle.rs`. Out - removing them from `doctor/inspect.rs` yet.
  - Done when: `config/lifecycle.rs` compiles, implements `ServiceLifecycle`, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T04: Extract `local_db` health checks and setup into `ServiceLifecycle` impl (status:todo)
  - Task ID: T04
  - Goal: Move local-DB-specific doctor inspection logic (DB path health, parent directory readiness) and DB setup logic (`bootstrap_local_db`) into `services/local_db/lifecycle.rs` as a `ServiceLifecycle` implementation.
  - Boundaries (in/out of scope): In - moving DB health checks and setup into `local_db/lifecycle.rs`. Out - removing them from `doctor/inspect.rs` and `setup.rs` yet.
  - Done when: `local_db/lifecycle.rs` compiles, implements `ServiceLifecycle`, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T05: Refactor `doctor` to aggregate `ServiceLifecycle::diagnose` and `fix` (status:todo)
  - Task ID: T05
  - Goal: Replace direct inspection logic in `doctor/mod.rs` and `doctor/inspect.rs` with aggregation over a list of `&dyn ServiceLifecycle` providers. `doctor` calls `diagnose` on each service, collects problems, and for `--fix` calls `fix` on each service.
  - Boundaries (in/out of scope): In - refactoring doctor to use lifecycle aggregation, removing duplicated inspection logic from `doctor/inspect.rs`. Out - changing doctor output format or problem taxonomy.
  - Done when: `doctor` compiles, `cargo test` doctor tests pass, and `cargo check` is clean.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test doctor'` and `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T06: Refactor `setup` to aggregate `ServiceLifecycle::setup` (status:todo)
  - Task ID: T06
  - Goal: Replace direct setup steps in `setup.rs` with aggregation over a list of `&dyn ServiceLifecycle` providers. `setup` calls `setup` on each service in order (config → local_db → hooks → integrations) and collects results.
  - Boundaries (in/out of scope): In - refactoring setup command to use lifecycle aggregation, removing direct calls to `bootstrap_repo_local_config`, `bootstrap_local_db`, etc. from `setup.rs`. Out - changing setup target selection or interactive prompt flow.
  - Done when: `setup` compiles, `cargo test` setup tests pass, and `cargo check` is clean.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test setup'` and `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T07: Validate full behavior parity and sync context (status:todo)
  - Task ID: T07
  - Goal: Run full test suite, verify `sce doctor` and `sce setup` behavior is unchanged, and update relevant context files (`context/cli/cli-command-surface.md`, `context/sce/agent-trace-hook-doctor.md`) to document the lifecycle ownership shift.
  - Boundaries (in/out of scope): In - `nix flake check`, manual CLI smoke tests, context sync. Out - adding new features.
  - Done when: `nix flake check` passes, manual `sce doctor` and `sce setup --help` spot-checks are clean, and context files reflect the new service lifecycle boundaries.
  - Verification notes (commands or checks): `nix flake check`, manual CLI smoke tests.

## Open questions

None — assumptions recorded above.
