# CLI Service Lifecycle

`cli/src/services/lifecycle.rs` defines the internal lifecycle capability foundation for the Rust CLI service layer. `cli/src/services/hooks_lifecycle.rs` is the first concrete capability implementation, `cli/src/services/local_db_lifecycle.rs` is the second, and `cli/src/services/lifecycle_registry.rs` is the capability registry for hooks and local DB services.

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

### Local DB lifecycle service (`local_db_lifecycle.rs`)

`LocalDbLifecycleService` is the second production lifecycle capability implementation:

- **Service ID:** `local_db`
- **Diagnostic facet:** owns local DB path resolution, parent-directory readiness (missing, not a directory, not writable), and DB file health checks. Produces `DiagnosticRecord` entries with stable kind constants (`local_db_path_unresolvable`, `local_db_parent_missing`, `local_db_parent_not_directory`, `local_db_parent_not_writable`, `local_db_health_check_failed`) that map into existing doctor problem types. Uses `default_paths::local_db_path()` for canonical path resolution and does not require filesystem or database fixtures in tests.
- **Fix facet:** owns local DB parent-directory bootstrap repair. Creates the missing canonical SCE-owned DB parent directory via `fs::create_dir_all()` when the resolved parent path matches the expected owned location (`resolve_local_db_parent_path()`). Refuses with `LifecycleOutcome::Failed` when the parent path does not match the canonical owned location, preventing directory creation in unexpected paths. Returns `LifecycleOutcome::Applied` when created, `Unchanged` when the parent already exists, and `Failed` for unresolvable paths or creation errors. Uses the `fix_missing_parent_directory()` helper for testability.
- **Setup facet:** not implemented; setup already owns `bootstrap_local_db` and that ownership boundary is preserved.
- **Preview facet:** not implemented; deferred.

The diagnostic facet resolves the canonical local DB path, checks parent-directory readiness, and verifies DB file health (existence and file-type validation). If the DB file does not exist, that is not a diagnostic error — it will be created on first use by setup/bootstrap.

The fix facet resolves the canonical DB path and canonical parent path, then delegates to `fix_missing_parent_directory()` which checks parent state and creates the missing canonical parent directory only when it is safe to do so.

### Lifecycle registry (`lifecycle_registry.rs`)

The registry resolves lifecycle capabilities by service ID:

- `LifecycleRegistry::setup_lifecycle(HOOKS_SERVICE_ID)` returns the hooks setup lifecycle.
- `LifecycleRegistry::diagnostic_lifecycle(HOOKS_SERVICE_ID)` returns the hooks diagnostic lifecycle.
- `LifecycleRegistry::fix_lifecycle(HOOKS_SERVICE_ID)` returns the hooks fix lifecycle.
- `LifecycleRegistry::diagnostic_lifecycle(LOCAL_DB_SERVICE_ID)` returns the local DB diagnostic lifecycle.
- `LifecycleRegistry::fix_lifecycle(LOCAL_DB_SERVICE_ID)` returns the local DB fix lifecycle.
- Missing or unregistered service IDs return `None`.
- `setup_lifecycle(LOCAL_DB_SERVICE_ID)` returns `None` because local DB setup is owned by `sce setup` directly.

### Command orchestration integration

- `sce setup --hooks` resolves the registered hooks setup lifecycle by service ID, then maps the typed setup report back to the existing `RequiredHooksInstallOutcome` formatter so public output remains stable.
- Doctor hook health inspection consumes hooks lifecycle diagnostics for required-hook checks and maps them back into existing doctor report/problem types.
- Doctor hook repair consumes the hooks lifecycle fix facet and maps typed lifecycle actions back into existing `DoctorFixResultRecord` outcomes/details.
- Doctor local DB health inspection consumes the local DB diagnostic lifecycle through the registry, mapping typed `DiagnosticRecord` entries into `DoctorProblem` types with a `LocalDatabase` problem category and stable kind-to-problem mappings. The `collect_local_db_health` function in `doctor/inspect.rs` resolves the lifecycle capability, invokes `diagnose()`, and derives a `LocalDbHealth` struct (with `LocalDbStatus` and `LocalDbParentStatus` enums) from the diagnostic report for both text and JSON rendering.

## Extension pattern for future services

When a new service needs lifecycle capabilities:

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

The hooks lifecycle and registry modules have pure unit-test coverage for repository-context validation, typed action mapping across install/update/skip preview outcomes, successful hooks setup/diagnostic/fix lookup, and missing-capability handling. The registry also has tests for local DB diagnostic/fix lookup, `setup_lifecycle` returning `None` for `local_db`, and `None` for unknown service IDs across all three registry methods. Doctor output-shape stability remains covered by the existing flake check surface.

The local DB lifecycle module has pure unit-test coverage for metadata, diagnostic report generation, kind constant stability, directory writability rejection, DB health rejection of directories, parent path resolution, fix report generation, canonical parent directory creation, non-canonical parent refusal, unchanged outcome when parent exists, failed outcome for path without parent, and failed outcome for creation errors.

## Implementation notes

- The module uses `anyhow::Result`, matching the surrounding service-layer error convention.
- It is intentionally internal and currently has localized dead-code allowance for lifecycle facets whose command consumers are planned separately.
- Future lifecycle work should map reports back to existing setup/doctor output types at orchestration boundaries to preserve public output contracts.

See also: [Architecture](../architecture.md), [Patterns](../patterns.md), [Context map](../context-map.md)