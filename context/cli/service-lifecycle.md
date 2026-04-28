# CLI Service Lifecycle

`cli/src/services/lifecycle.rs` defines the internal lifecycle capability foundation for the Rust CLI service layer. `cli/src/services/hooks_lifecycle.rs` is the first concrete capability implementation, and `cli/src/services/lifecycle_registry.rs` is the current hook-only capability registry.

## Current contract

### Trait and model foundation (`lifecycle.rs`)

- `ServiceId` and `ServiceMetadata` identify lifecycle-capable services without coupling command orchestration to concrete modules.
- `LifecycleContext` carries optional repository, config, and state roots as dependency-path seams for service implementations that need filesystem or git context without requiring tests to touch real filesystem/git state.
- `LifecycleOperation` names the supported operation families: setup, diagnose, fix, and preview.
- Request types (`SetupRequest`, `DiagnoseRequest`, `FixRequest`, `PreviewRequest`) wrap lifecycle context plus operation-specific inputs. `FixRequest` carries `problem_kinds` to scope which problems a fix should address. `PreviewRequest` carries an `operation` field so preview consumers can request action plans for any lifecycle operation.
- Report/model types (`SetupReport`, `DiagnosticReport`, `FixReport`, `ActionPlan`, `LifecycleAction`, `DiagnosticRecord`) are typed enough to map into existing setup and doctor result/render surfaces.
- `LifecycleOutcome` provides a shared outcome vocabulary: `Applied`, `Updated`, `Unchanged`, `Skipped`, `Failed`.
- `DiagnosticSeverity` and `DiagnosticFixability` classify diagnostic records for doctor report mapping.
- Traits are composable by facet: `SetupLifecycle`, `DiagnosticLifecycle`, `FixLifecycle`, and `PreviewLifecycle` all extend `LifecycleService`, allowing a service to implement only the lifecycle behavior it supports.

### Hooks lifecycle service (`hooks_lifecycle.rs`)

`HooksLifecycleService` is the first production lifecycle capability implementation:

- **Service ID:** `hooks`
- **Setup facet:** delegates to `setup::install_required_git_hooks(...)` and maps required-hook `Installed`/`Updated`/`Skipped` outcomes to typed lifecycle actions.
- **Diagnostic facet:** owns required-hook diagnosis for hooks-directory presence/type, required-hook presence, executable-bit, content-drift, and read-failure checks. Produces `DiagnosticRecord` entries with stable kind constants (`hooks_directory_missing`, `hooks_path_not_directory`, `required_hook_missing`, `hook_not_executable`, `hook_content_stale`, `hook_read_failed`) that map into existing doctor problem types.
- **Fix facet:** delegates to `setup::install_required_git_hooks(...)` and maps required-hook repair outcomes to typed lifecycle actions for doctor fix-result mapping.
- **Preview facet:** delegates to `setup::preview_required_git_hooks(...)`, which resolves the same repository/hooks path as installation and returns intended hook actions without staging or writing hook files.

The diagnostic facet also produces a `HooksDiagnosticReport` containing both structured `RequiredHookHealth` entries (per-hook name, path, exists, executable, content state) and the standard `DiagnosticReport` for doctor consumption.

### Lifecycle registry (`lifecycle_registry.rs`)

The current registry is intentionally small and hook-only:

- `LifecycleRegistry::setup_lifecycle(HOOKS_SERVICE_ID)` returns the hooks setup lifecycle.
- `LifecycleRegistry::diagnostic_lifecycle(HOOKS_SERVICE_ID)` returns the hooks diagnostic lifecycle.
- `LifecycleRegistry::fix_lifecycle(HOOKS_SERVICE_ID)` returns the hooks fix lifecycle.
- Missing or unregistered service IDs return `None`.
- No non-hook services are registered yet.

### Command orchestration integration

- `sce setup --hooks` resolves the registered hooks setup lifecycle by service ID, then maps the typed setup report back to the existing `RequiredHooksInstallOutcome` formatter so public output remains stable.
- Doctor hook health inspection consumes hooks lifecycle diagnostics for required-hook checks and maps them back into existing doctor report/problem types.
- Doctor hook repair consumes the hooks lifecycle fix facet and maps typed lifecycle actions back into existing `DoctorFixResultRecord` outcomes/details.

## Extension pattern for future services

When a new service (for example local DB or OpenCode assets) needs lifecycle capabilities:

1. Define a `ServiceId` constant and implement the needed lifecycle traits (`SetupLifecycle`, `DiagnosticLifecycle`, `FixLifecycle`, `PreviewLifecycle`) in a dedicated module under `cli/src/services/`.
2. Register the new service ID and its implemented facets in `LifecycleRegistry`.
3. Wire command orchestration (setup, doctor, or other commands) to resolve the capability through the registry and map typed lifecycle reports into existing command-specific output types.
4. Keep mutations delegated to canonical service-owned helpers while exposing non-mutating preview/action-plan data separately.
5. Add registry lookup tests for the new service ID and missing-capability handling.

This pattern keeps command orchestration decoupled from service internals, preserves existing output contracts, and allows services to implement only the lifecycle facets they support.

## Test coverage

The lifecycle foundation has pure unit-test coverage with fake services to prove:

- dependency-path context is carried without filesystem/git fixtures;
- setup capabilities can return typed actions;
- distinct lifecycle traits allow partial service capability support.

The hooks lifecycle and registry modules have pure unit-test coverage for repository-context validation, typed action mapping across install/update/skip preview outcomes, successful hooks setup/diagnostic/fix lookup, and missing-capability handling. Doctor output-shape stability remains covered by the existing flake check surface.

## Implementation notes

- The module uses `anyhow::Result`, matching the surrounding service-layer error convention.
- It is intentionally internal and currently has localized dead-code allowance for lifecycle facets whose command consumers are planned separately.
- Future lifecycle work should map reports back to existing setup/doctor output types at orchestration boundaries to preserve public output contracts.

See also: [Architecture](../architecture.md), [Patterns](../patterns.md), [Context map](../context-map.md)