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
- No service implements `ServiceLifecycle` yet.
- `doctor` and `setup` runtime behavior is unchanged; they still use their existing orchestration paths until later lifecycle migration tasks wire aggregation.

## Related context

- `context/cli/capability-traits.md`
- `context/cli/cli-command-surface.md`
- `context/sce/agent-trace-hook-doctor.md`
