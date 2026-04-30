# CLI Service Lifecycle

`cli/src/services/lifecycle.rs` defines the current compile-safe lifecycle seam for moving service-owned setup and health behavior out of monolithic command orchestrators.

## Current contract

- `ServiceLifecycle: Send + Sync` exposes three default no-op methods:
  - `diagnose(&self, ctx: &AppContext) -> Vec<HealthProblem>`
  - `fix(&self, ctx: &AppContext, problems: &[HealthProblem]) -> Vec<FixResultRecord>`
  - `setup(&self, ctx: &AppContext) -> anyhow::Result<SetupOutcome>`
- `HealthProblem`, `HealthCategory`, `HealthSeverity`, `HealthFixability`, and `HealthProblemKind` are lifecycle-owned types that mirror the current doctor taxonomy without making the trait depend on `doctor` module types.
- `FixResultRecord` and `FixOutcome` are lifecycle-owned fix result types.
- `SetupOutcome` is a minimal lifecycle-owned carrier for current setup result shapes:
  - optional lifecycle-owned `RequiredHooksInstallOutcome`
- `LifecycleProvider` aliases boxed lifecycle providers, and `lifecycle_providers(include_hooks)` is the shared provider catalog/factory used by command orchestrators.
- Provider order is deterministic: `ConfigLifecycle` → `LocalDbLifecycle` → `AgentTraceDbLifecycle` → `HooksLifecycle` when hooks are included.

## Current boundaries

- The trait is registered through `cli/src/services/mod.rs`.
- Doctor/setup modules adapt lifecycle-owned result types at orchestration boundaries before rendering command-owned output.
- `cli/src/services/hooks/lifecycle.rs` defines `HooksLifecycle`, the hook-owned provider.
- `HooksLifecycle::diagnose` emits hook rollout/repository-targeting lifecycle health problems and compares required hook files against canonical embedded hook assets.
- `HooksLifecycle::fix` reuses the canonical required-hook setup flow for auto-fixable hook rollout problems.
- `HooksLifecycle::setup` returns lifecycle-owned `SetupOutcome.required_hooks_install` data adapted from the canonical `install_required_git_hooks` flow.
- `cli/src/services/config/lifecycle.rs` defines `ConfigLifecycle`, the config-owned provider.
- `ConfigLifecycle::diagnose` emits global/repo-local config validation lifecycle health problems.
- `ConfigLifecycle::setup` bootstraps the repo-local `.sce/config.json` through the existing canonical setup helper using `ctx.repo_root()` and returns an empty `SetupOutcome` because config bootstrap currently has no dedicated outcome carrier.
- `cli/src/services/local_db/lifecycle.rs` defines `LocalDbLifecycle`, the local-DB-owned provider.
- `LocalDbLifecycle::diagnose` emits canonical local DB path and parent-directory readiness lifecycle health problems.
- `LocalDbLifecycle::fix` bootstraps the canonical local DB parent directory for auto-fixable local DB parent readiness problems.
- `LocalDbLifecycle::setup` initializes the canonical local DB through `LocalDb::new()` and returns an empty `SetupOutcome` because DB bootstrap currently has no dedicated outcome carrier.
- `cli/src/services/agent_trace_db/lifecycle.rs` defines `AgentTraceDbLifecycle`, the Agent Trace DB-owned provider.
- `AgentTraceDbLifecycle::diagnose` emits canonical Agent Trace DB path and parent-directory readiness lifecycle health problems.
- `AgentTraceDbLifecycle::fix` bootstraps the canonical Agent Trace DB parent directory for auto-fixable DB parent readiness problems.
- `AgentTraceDbLifecycle::setup` initializes the Agent Trace DB through `AgentTraceDb::new()` and returns an empty `SetupOutcome` because DB bootstrap currently has no dedicated outcome carrier.
- `doctor` runtime execution now aggregates lifecycle providers for diagnosis and repair:
  - `cli/src/services/doctor/command.rs` passes `AppContext` into doctor execution.
  - `cli/src/services/doctor/mod.rs` resolves the repository root once, creates a repo-root-scoped `AppContext` using `with_repo_root()`, and requests the full provider catalog with hooks included.
  - `ConfigLifecycle::diagnose` and `HooksLifecycle::diagnose`/`fix` now consume `ctx.repo_root()` instead of calling `std::env::current_dir()` independently.
  - Diagnose mode collects `ServiceLifecycle::diagnose` health problems from each provider, adapts them into doctor-owned problem records, then `doctor/inspect.rs` builds the report facts and integration health around those service-owned problems.
  - Fix mode adapts doctor problem records back into lifecycle health problems for provider repair decisions, adapts lifecycle fix records into doctor-owned fix records, rebuilds the report after fixes, and keeps manual remediation reporting through `doctor/fixes.rs`.
- `setup` runtime execution now aggregates lifecycle providers for setup:
  - `cli/src/services/setup/command.rs` resolves the repository root, derives a repo-root-scoped `AppContext` from the runtime command context with `with_repo_root()`, and requests the shared provider catalog with hooks included only when `SetupRequest.install_hooks` is true.
  - Setup lifecycle providers receive the runtime logger, telemetry, filesystem capability, and git capability objects with `repo_root` populated instead of a setup-local replacement context.
  - `HooksLifecycle::setup` returns lifecycle-owned `SetupOutcome.required_hooks_install` from the canonical `install_required_git_hooks` flow, and setup command adapts that result into setup-owned hook install outcomes before rendering.
  - Config asset installation (OpenCode/Claude targets) remains handled by the setup command after lifecycle aggregation.

## Related context

- `context/cli/capability-traits.md`
- `context/cli/cli-command-surface.md`
- `context/sce/agent-trace-hook-doctor.md`
