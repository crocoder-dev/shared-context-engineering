# CLI Service Lifecycle

`cli/src/services/lifecycle.rs` defines the current compile-safe lifecycle seam for moving service-owned setup and health behavior out of monolithic command orchestrators.

## Current contract

- `ServiceLifecycle: Send + Sync` exposes three default no-op methods:
  - `diagnose(&self, ctx: &AppContext) -> Vec<HealthProblem>`
  - `fix(&self, ctx: &AppContext, problems: &[HealthProblem]) -> Vec<DoctorFixResultRecord>`
  - `setup(&self, ctx: &AppContext) -> anyhow::Result<SetupOutcome>`
- `HealthProblem` aliases the existing doctor `DoctorProblem` type so lifecycle services reuse the current doctor taxonomy rather than introducing a parallel problem model.
- `DoctorFixResultRecord` is reused for fix outcomes.
- `SetupOutcome` is a minimal carrier for current setup result shapes:
  - optional `SetupInstallOutcome`
  - optional `RequiredHooksInstallOutcome`

## Current boundaries

- The trait is registered through `cli/src/services/mod.rs`.
- Doctor problem/fix types and relevant taxonomy enums are crate-visible so future service lifecycle implementations can construct existing doctor records.
- `cli/src/services/hooks/lifecycle.rs` defines `HooksLifecycle`, the hook-owned provider.
- `HooksLifecycle::diagnose` emits hook rollout/repository-targeting problems with the existing doctor taxonomy and compares required hook files against canonical embedded hook assets.
- `HooksLifecycle::fix` reuses the canonical required-hook setup flow for auto-fixable hook rollout problems.
- `HooksLifecycle::setup` returns `SetupOutcome.required_hooks_install` from the canonical `install_required_git_hooks` flow.
- `cli/src/services/config/lifecycle.rs` defines `ConfigLifecycle`, the config-owned provider.
- `ConfigLifecycle::diagnose` emits global/repo-local config validation problems with the existing doctor taxonomy.
- `ConfigLifecycle::setup` bootstraps the repo-local `.sce/config.json` through the existing canonical setup helper using `ctx.repo_root()` and returns an empty `SetupOutcome` because config bootstrap currently has no dedicated outcome carrier.
- `cli/src/services/local_db/lifecycle.rs` defines `LocalDbLifecycle`, the local-DB-owned provider.
- `LocalDbLifecycle::diagnose` emits canonical local DB path and parent-directory readiness problems with the existing doctor taxonomy.
- `LocalDbLifecycle::fix` bootstraps the canonical local DB parent directory for auto-fixable local DB parent readiness problems.
- `LocalDbLifecycle::setup` initializes the canonical local DB through `LocalDb::new()` and returns an empty `SetupOutcome` because DB bootstrap currently has no dedicated outcome carrier.
- `doctor` runtime execution now aggregates lifecycle providers for diagnosis and repair:
  - `cli/src/services/doctor/command.rs` passes `AppContext` into doctor execution.
  - `cli/src/services/doctor/mod.rs` builds the current provider list (`ConfigLifecycle`, `LocalDbLifecycle`, `HooksLifecycle`).
  - Diagnose mode collects `ServiceLifecycle::diagnose` problems from each provider, then `doctor/inspect.rs` builds the report facts and integration health around those service-owned problems.
  - Fix mode calls `ServiceLifecycle::fix` on each provider, rebuilds the report after fixes, and keeps manual remediation reporting through `doctor/fixes.rs`.
- `setup` runtime execution now aggregates lifecycle providers for setup:
  - `cli/src/services/setup/command.rs` resolves the repository root, builds an `AppContext` with the resolved root, and calls `ServiceLifecycle::setup` on each provider in order (config → local_db → hooks when requested).
  - `HooksLifecycle::setup` returns `SetupOutcome.required_hooks_install` from the canonical `install_required_git_hooks` flow.
  - Config asset installation (OpenCode/Claude targets) remains handled by the setup command after lifecycle aggregation.

## Related context

- `context/cli/capability-traits.md`
- `context/cli/cli-command-surface.md`
- `context/sce/agent-trace-hook-doctor.md`
