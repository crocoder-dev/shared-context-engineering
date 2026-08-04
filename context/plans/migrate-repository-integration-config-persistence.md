# Plan: migrate-repository-integration-config-persistence

## Change summary

Extract repository-scoped integration configuration persistence from
`services::setup` into the CLI's internal hexagonal layers without moving
`run_setup_for_mode`. The slice adds a narrow `IntegrationConfigRepository`
port, three application use cases, and a filesystem outbound adapter that owns
`.sce/config.json` lifecycle, JSON parsing/merge behavior, and serialization.

The existing public setup functions remain compatibility facades. Their
callers, error context, setup ordering, optional-workflow best-effort behavior,
and successful-install-before-persistence contract remain unchanged. The
adapter will preserve the current repository-config compatibility semantics
without deserializing the full document into a strict replacement struct.

## Acceptance criteria

- [x] AC1: Repository configuration operations are represented by the narrow
  `IntegrationConfigRepository` port, and application code depends only on the
  port and existing domain types; no application module depends on
  `services`, filesystem APIs, or JSON APIs.
  - Validate: `nix develop -c ./scripts/check-cli-architecture.sh`; inspect the
    port and use-case imports and run the architecture flake check through
    `nix flake check`.
- [x] AC2: `IntegrationTarget::config_id()` returns `opencode`, `claude`, and
  `pi`, while the persistence port accepts only concrete
  `IntegrationTarget` values; `All` is expanded in the record-installation use
  case in `OpenCode`, `Claude`, `Pi` order before the repository is called.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml integration_config_repository` and
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml record_integration_installation`.
- [x] AC3: `FilesystemIntegrationConfigRepository::ensure_exists` creates a
  missing `.sce/config.json` with the canonical schema-only payload and final
  newline, but never overwrites an existing file.
  - Validate: adapter tests for missing and existing configuration pass through
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::integration_config_repository`.
- [x] AC4: The filesystem adapter preserves the current JSON compatibility
  semantics: unrelated top-level fields and unknown existing integration target
  strings survive recording; existing target order is retained; newly selected
  concrete target IDs are appended and deduplicated; and
  `optional_workflows` is replaced by the current selection.
  - Validate: the adapter merge tests in
    `cli/src/adapters/outbound/filesystem/integration_config_repository.rs`
    cover unrelated fields, order, deduplication, unknown strings, and
    workflow replacement.
- [x] AC5: Invalid JSON and non-object top-level JSON values return the current
  stable errors from the repository adapter, while serialized successful output
  is pretty JSON with exactly one final newline.
  - Validate: adapter tests assert the existing error text and inspect the
    written bytes for the final-newline contract.
- [x] AC6: `EnsureRepoConfig` delegates to `ensure_exists`,
  `LoadPersistedOptionalWorkflows` returns repository errors unchanged, and
  `RecordIntegrationInstallation` forwards the workflow slice unchanged while
  propagating repository errors.
  - Validate: focused use-case tests with fake repositories prove delegation,
    `All` expansion, single-target behavior, unchanged workflow forwarding, and
    error propagation.
- [x] AC7: The public legacy facades
  `bootstrap_repo_local_config`, `persisted_optional_workflows`, and
  `persist_integration_targets` construct and invoke the new use cases while
  preserving legacy error context and best-effort behavior. `services::setup`
  no longer performs repository config I/O, JSON mutation, or uses the legacy
  config-schema parser.
  - Validate: facade tests prove missing/unreadable/invalid workflow config
    returns an empty selection, target recording preserves unrelated fields,
    `SetupTarget::All` records `opencode`, `claude`, and `pi`, and the source
    inspection shows no config-schema/JSON persistence implementation remains
    in `services::setup`.
- [x] AC8: `run_setup_for_mode` keeps its existing call-site sequence and
  persists integration state only after asset installation succeeds; normal
  setup and `ConfigLifecycle` continue using the compatibility facades without
  composition-root wiring or orchestration migration.
  - Validate: setup facade/orchestration tests and source inspection confirm the
    order `load workflows -> install assets -> persist configuration -> render`
    and that an installation failure leaves no newly recorded target.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`

### Context sync

- `context/architecture.md` — document the repository integration-configuration
  vertical slice, its port/use-case/adapter ownership, and the continued
  compatibility-facade boundary.
- `context/sce/setup-repo-local-config-bootstrap.md` — update implementation
  ownership from `services::setup` JSON persistence to the application port and
  filesystem adapter while retaining setup ordering and compatibility behavior.
- `context/glossary.md` — add the repository integration-configuration port,
  adapter, and use-case terminology if the new boundary is part of current
  architecture vocabulary.
- `context/context-map.md` — update annotations only if the ownership changes
  make an existing setup/config context entry materially incomplete.

## Constraints and non-goals

- **In scope:** `IntegrationTarget::config_id`; `cli/src/application/ports/integration_config_repository.rs`; `cli/src/application/use_cases/{load_persisted_optional_workflows,record_integration_installation,ensure_repo_config}.rs`; `cli/src/adapters/outbound/filesystem/integration_config_repository.rs`; module registrations; the three public setup facades and their focused tests.
- **Out of scope:** moving `run_setup_for_mode`; setup rendering; `SetupCommand`; prompts; workflow validation; Git repository discovery; hooks; `ConfigLifecycle`; general/global configuration loading or precedence; JSON Schema validation; composition-root wiring; asset installation migration changes.
- **Constraints:** Preserve current public function signatures, call-site order, output/error context, canonical bootstrap payload, optional-workflow best-effort fallback, unknown integration target strings, target ordering, pretty serialization, and final-newline behavior. Use no new crate dependencies. Keep all JSON-document merge logic in the outbound adapter rather than introducing a strict whole-document domain struct.
- **Non-goal:** Generalizing this boundary into `ConfigStore` or making it responsible for global configuration, environment precedence, observability, database, or unrelated config domains.
- **Precondition:** The in-flight Nix CI and release-validation jobs for the preceding slice must finish before implementation begins. The stale Clippy-warning sentence in the completed `context/plans/install-integration-assets.md` validation record must be removed before the repository-config slice starts.

## Assumptions

- `IntegrationConfigRepository::record_installation` receives a concrete slice of `IntegrationTarget`; the use case owns expansion of `IntegrationTargetSelection::All`.
- The adapter continues to represent the document as `serde_json::Value`, preserving unrelated top-level fields and unknown string target IDs instead of round-tripping through a strict typed config model.
- `LoadPersistedOptionalWorkflows` delegates errors to its caller, while the legacy `persisted_optional_workflows` facade alone applies `unwrap_or_default()` for missing, unreadable, or invalid configuration.
- Existing adapter tests may use the repository's established filesystem-fixture style; all repository verification still runs through the Nix/Cargo wrapper boundary.

## Task stack

- [x] T01: `Clear the stale prior-slice validation warning before migration` (status:done)
  - Task ID: T01
  - Goal: Remove the stale Clippy-warning follow-up sentence from the completed `install-integration-assets` plan after the preceding Nix CI and release-validation jobs have concluded, leaving its validated evidence internally consistent.
  - Boundaries (in/out of scope): In — the single stale warning in `context/plans/install-integration-assets.md` and recording the completed CI/release outcomes in the task evidence. Out — changing completed implementation tasks, revalidating the new repository-config slice, or modifying application code.
  - Dependencies: none
  - Done when: The completed plan no longer claims that Clippy validation remains incomplete, and the prerequisite CI/release-validation outcomes are known before T02 begins.
  - Verification notes (commands or checks): Inspect the relevant `Validation Report` and `Residual risks` text in `context/plans/install-integration-assets.md`; record the completed CI/release-validation status without changing historical task evidence.
  - Evidence: Confirmed via the GitHub check-runs API for the `hexagonal` branch head commit (`8e34b20`) that all 8 checks — including `Nix CI (ubuntu-latest)`, `Nix CI (macos-latest)`, `Release validation (ubuntu-latest)`, and `Release validation (macos-latest)` — are `completed` with `conclusion: success`. Removed the stale sentence "Full validation remains incomplete until the Clippy diagnostics are repaired." from the `Residual risks` section of `context/plans/install-integration-assets.md`; that plan's own Validation Report already recorded a passing `nix flake check` (which includes Clippy) and an empty "Failed checks and follow-ups" section, so the sentence contradicted its own recorded evidence. No historical task evidence was changed.
  - Verification: `git diff --check -- context/plans/install-integration-assets.md` — passed, no whitespace errors. Manually re-read the `Residual risks` section — no remaining reference to incomplete Clippy validation.

- [x] T02: `Define the integration configuration port and concrete target identifiers` (status:done)
  - Task ID: T02
  - Goal: Add `IntegrationTarget::config_id()` and the application-owned `IntegrationConfigRepository` trait, then register the new port module.
  - Boundaries (in/out of scope): In — `cli/src/domain/integration/target.rs`, `cli/src/application/ports/integration_config_repository.rs`, and module registration. Out — use-case behavior, adapter I/O, facade changes, and `run_setup_for_mode`.
  - Dependencies: T01
  - Done when: The port exposes `ensure_exists`, `load_optional_workflows`, and `record_installation` with the requested signatures; target IDs are canonical; and application/domain architecture checks pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/check-cli-architecture.sh`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_id`.
  - Evidence: Added `IntegrationTarget::config_id(self) -> &'static str` in `cli/src/domain/integration/target.rs`, returning `"opencode"`, `"claude"`, `"pi"` for the three concrete targets (mirrors the existing `services::setup::integration_target_id_str`, which T06 retires). Added `cli/src/application/ports/integration_config_repository.rs` defining `pub(crate) trait IntegrationConfigRepository` with `type Error;` and `ensure_exists(&self, repository_root: &Path) -> Result<(), Self::Error>`, `load_optional_workflows(&self, repository_root: &Path) -> Result<Vec<String>, Self::Error>`, and `record_installation(&self, repository_root: &Path, targets: &[IntegrationTarget], optional_workflows: &[String]) -> Result<(), Self::Error>`. Registered `pub(crate) mod integration_config_repository;` in `cli/src/application/ports/mod.rs`. Added `config_id_maps_each_concrete_target` unit test. Both new items are temporarily `#[allow(dead_code)]`, consumed starting with T03/T04 (port) and T05 (`config_id`).
  - Verification: `nix develop -c ./scripts/check-cli-architecture.sh` — passed, no forbidden dependencies in domain or application layers. `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_id` — passed, 1 test (`config_id_maps_each_concrete_target`). `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed clean (also fixed a `clippy::trivially_copy_pass_by_ref` finding by taking `self` by value).

- [x] T03: `Add repository-config bootstrap and workflow-loading use cases` (status:done)
  - Task ID: T03
  - Goal: Implement `EnsureRepoConfig<R>` and `LoadPersistedOptionalWorkflows<R>` as thin repository-delegating use cases with focused fake-repository tests.
  - Boundaries (in/out of scope): In — the two use-case files, use-case module registration, delegation tests, and repository-error propagation for loading. Out — concrete filesystem behavior, record-installation expansion, and compatibility facade wiring.
  - Dependencies: T02
  - Done when: `EnsureRepoConfig::execute` delegates to `ensure_exists`, the loader returns the repository's `Vec<String>` unchanged, and both use cases preserve repository errors without depending on `services` or infrastructure APIs.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml load_persisted_optional_workflows`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ensure_repo_config`; `nix develop -c ./scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/application/use_cases/ensure_repo_config.rs` with `EnsureRepoConfigRequest { repository_root: PathBuf }` and `EnsureRepoConfig<R: IntegrationConfigRepository>::execute` delegating to `repository.ensure_exists(&request.repository_root)`. Added `cli/src/application/use_cases/load_persisted_optional_workflows.rs` with `LoadPersistedOptionalWorkflowsRequest { repository_root: PathBuf }` and `LoadPersistedOptionalWorkflows<R: IntegrationConfigRepository>::execute` returning `repository.load_optional_workflows(&request.repository_root)` unchanged, including its `Result<Vec<String>, R::Error>` errors. Registered both modules in `cli/src/application/use_cases/mod.rs`. Both use cases depend only on `crate::application::ports::integration_config_repository` and `std::path::PathBuf`, with no `services` or infrastructure dependency. Each file has focused fake-repository tests proving delegation and error propagation (`execute_delegates_to_ensure_exists_with_the_resolved_root`, `execute_propagates_repository_errors`, `execute_returns_the_repositorys_workflows_unchanged`, `execute_propagates_repository_errors_unchanged`). Both new types are temporarily `#[allow(dead_code)]`, consumed starting with the compatibility facades (T06), matching the existing T02 convention.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml load_persisted_optional_workflows` — passed, 2 tests. `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ensure_repo_config` — passed, 2 tests. `nix develop -c ./scripts/check-cli-architecture.sh` — passed, no forbidden dependencies in domain or application layers. `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed clean (fixed two `clippy::needless_pass_by_value` findings by taking `request` by reference).

- [x] T04: `Add the record-installation use case with All expansion` (status:done)
  - Task ID: T04
  - Goal: Implement `RecordIntegrationInstallation<R>` and its request type so target selections are expanded before the repository port is called.
  - Boundaries (in/out of scope): In — `record_integration_installation.rs`, request/use-case module registration, fake-repository tests for `All`, one target, workflow forwarding, and error propagation. Out — JSON mutation, filesystem access, facade wiring, and orchestration migration.
  - Dependencies: T02
  - Done when: `All` invokes the repository once with concrete targets in `OpenCode`, `Claude`, `Pi` order; a single target invokes one concrete target; the optional-workflow slice is forwarded unchanged; and repository errors are returned.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml record_integration_installation`; `nix develop -c ./scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/application/use_cases/record_integration_installation.rs` with `RecordIntegrationInstallationRequest { repository_root: PathBuf, selection: IntegrationTargetSelection, optional_workflows: Vec<String> }` and `RecordIntegrationInstallation<R: IntegrationConfigRepository>::execute`, which calls `request.selection.targets()` (the existing `IntegrationTargetSelection::targets()` expansion) and forwards the concrete slice plus `optional_workflows` unchanged to a single `repository.record_installation` call, returning `R::Error` unchanged. Registered `pub(crate) mod record_integration_installation;` in `cli/src/application/use_cases/mod.rs`. The use case depends only on `crate::application::ports::integration_config_repository` and `crate::domain::integration::IntegrationTargetSelection`, with no `services`, filesystem, or JSON dependency. Both new types are temporarily `#[allow(dead_code)]`, consumed starting with the compatibility facades (T06), matching the T02/T03 convention. Added focused fake-repository tests: `one_selection_records_a_single_concrete_target`, `all_selection_records_every_target_in_order` (asserts `OpenCode`, `Claude`, `Pi` order via one repository call), `execute_forwards_the_optional_workflow_slice_unchanged`, and `execute_propagates_repository_errors`.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml record_integration_installation` — passed, 4 tests. `nix develop -c ./scripts/check-cli-architecture.sh` — passed, no forbidden dependencies in domain or application layers. `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed clean (fixed a `clippy::type_complexity` finding in the test fixture by introducing a `RecordInstallationCall` type alias).

- [x] T05: `Implement the filesystem integration-configuration repository` (status:done)
  - Task ID: T05
  - Goal: Move `.sce/config.json` lifecycle, JSON parsing/merge, compatibility-preserving target/workflow persistence, and pretty serialization into `FilesystemIntegrationConfigRepository`.
  - Boundaries (in/out of scope): In — `cli/src/adapters/outbound/filesystem/integration_config_repository.rs`, filesystem module registration, canonical bootstrap payload, path resolution, adapter error context, and all required adapter tests. Out — use-case orchestration, setup facades, `ConfigLifecycle`, global config, schema validation, and strict typed document deserialization.
  - Dependencies: T03, T04
  - Done when: Adapter tests prove missing bootstrap, non-overwrite, unrelated-field preservation, target order, append/deduplication, unknown target preservation, workflow replacement, invalid JSON/non-object errors, and exactly one final newline; all three port operations compile and use the existing error wording/context.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::integration_config_repository`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml integration_config_repository`.
  - Evidence: Added `cli/src/adapters/outbound/filesystem/integration_config_repository.rs` implementing `IntegrationConfigRepository` for `FilesystemIntegrationConfigRepository` (`type Error = anyhow::Error`), porting the exact behavior and error wording previously inline in `services::setup::{bootstrap_repo_local_config, persisted_optional_workflows, persist_integration_targets}`: `ensure_exists` creates `.sce/config.json` with the canonical `$schema` bootstrap payload (reusing `services::agent_trace::SCE_WEB_BASE_URL` and `services::default_paths::RepoPaths`) only when missing, never overwriting an existing file; `load_optional_workflows` reads the document via `serde_json::Value` and returns `integrations.optional_workflows` as `Vec<String>` (empty when absent), propagating read/parse/shape errors to the caller instead of defaulting (per the plan's `LoadPersistedOptionalWorkflows` assumption — only the legacy facade applies `unwrap_or_default()`); `record_installation` bootstraps a missing file first, then merges concrete `IntegrationTarget::config_id()` values into `integrations.target` (preserving existing order, unknown strings, and deduplicating), replaces `integrations.optional_workflows` with the current selection, and writes pretty JSON plus exactly one trailing newline. Registered `pub(crate) mod integration_config_repository;` in `cli/src/adapters/outbound/filesystem/mod.rs`. Kept all JSON-document merge logic on `serde_json::Value` in the adapter — no strict typed document struct — using no new crate dependencies (`serde_json` and `anyhow` were already CLI dependencies). Added 14 focused adapter tests covering: missing-config bootstrap and canonical payload, non-overwrite of an existing file, bootstrap-before-record on a missing file, unrelated top-level field preservation, existing target order preserved with new-target append/dedup, unknown existing target-string preservation, optional-workflow replacement, invalid-JSON and non-object top-level errors for both read and record paths, exactly-one-final-newline on write, recorded-workflow loading, empty-workflow default, and missing-file/invalid-JSON error propagation from `load_optional_workflows`.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::integration_config_repository` — passed, 14 tests. `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml integration_config_repository` — passed, the same 14 tests (no additional matches in the port module, which has no `#[cfg(test)]` tests of its own). `nix develop -c ./scripts/check-cli-architecture.sh` — passed, no forbidden dependencies in domain or application layers (the adapter layer is outside this check's scope and may depend on `services`, matching the existing `FilesystemIntegrationInstaller` precedent). `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed clean.

- [x] T06: `Replace setup configuration persistence with compatibility facades` (status:done)
  - Task ID: T06
  - Goal: Rewire `bootstrap_repo_local_config`, `persisted_optional_workflows`, and `persist_integration_targets` to the new use cases and adapter, remove legacy config-schema/JSON mutation and `integration_target_id_str`, and add facade/orchestration compatibility tests.
  - Boundaries (in/out of scope): In — `cli/src/services/setup/mod.rs` facade bodies/import cleanup, setup tests proving best-effort workflow reads, target persistence/`All`, preserved unrelated fields, and persistence-after-install ordering. Out — changing `run_setup_for_mode` call sites or sequence, migrating setup rendering/prompts/hooks, changing `ConfigLifecycle`, and composition wiring.
  - Dependencies: T04, T05
  - Done when: The three public functions retain their signatures and behavior; `run_setup_for_mode` still visibly performs workflow load, asset install, config persistence, then success rendering; `ConfigLifecycle` still calls `bootstrap_repo_local_config`; and required facade tests pass through the legacy entrypoints.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; source inspection for `integration_target_id_str`, `parse_file_config`, and JSON mutation removal from the setup config path; `nix develop -c ./scripts/check-cli-architecture.sh`.
  - Evidence: Rewired all three public facades in `cli/src/services/setup/mod.rs` to construct and invoke the T02-T05 use cases against `FilesystemIntegrationConfigRepository`, preserving each function's existing signature: `bootstrap_repo_local_config` now delegates to `EnsureRepoConfig::execute`; `persisted_optional_workflows` delegates to `LoadPersistedOptionalWorkflows::execute` and applies `.unwrap_or_default()` (the only place that defaults) so a missing/unreadable/invalid config still yields an empty selection; `persist_integration_targets` delegates to `RecordIntegrationInstallation::execute`, converting `SetupTarget` to `IntegrationTargetSelection` via a new private `integration_target_selection_for` helper (also reused by `install_embedded_setup_assets`, replacing its duplicate inline match). Removed the inline `repo_local_config_bootstrap_payload` helper, the `integration_target_id_str` function and its unit test, and the `use serde_json::json;` import — no config-schema parsing, `serde_json` value mutation, or JSON I/O remains in production code in `cli/src/services/setup/mod.rs` (confirmed by inspection; the crate's only remaining `serde_json`/`parse_file_config` references in that file are in the new facade tests, which assert against written JSON). `run_setup_for_mode`'s call-site body and order (`persisted_optional_workflows` fallback, then `install_embedded_setup_assets`, then `persist_integration_targets`, then success rendering) and `ConfigLifecycle::setup`'s call to `bootstrap_repo_local_config` were left untouched. Removed the now-unconsumed `#[allow(dead_code)]` markers on `IntegrationConfigRepository`, `IntegrationTarget`/`IntegrationTargetSelection`/`config_id`/`targets`, and the three T03/T04 use cases and their request types, since T06 is their real consumer. Added 8 new facade/orchestration tests in `cli/src/services/setup/mod.rs`: `bootstrap_repo_local_config_facade_creates_a_missing_config`; `persisted_optional_workflows_facade_returns_the_recorded_selection`; `persisted_optional_workflows_facade_returns_an_empty_selection_when_config_is_missing`; `persisted_optional_workflows_facade_returns_an_empty_selection_when_config_is_invalid`; `persist_integration_targets_facade_preserves_unrelated_fields_and_records_all_targets` (also proves `SetupTarget::All` records `opencode`, `claude`, `pi`); `run_setup_for_mode_installs_then_persists_configuration_after_a_successful_install` (proves install-then-persist ordering and success rendering); `run_setup_for_mode_leaves_no_recorded_target_when_installation_fails` (a non-directory repository root fails the installer's writability preflight before any config write, proving no config file, and therefore no newly recorded target, is created on installation failure).
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — passed, 23 tests. `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (full crate) — passed, 236 tests, 0 failed. `nix develop -c ./scripts/check-cli-architecture.sh` — passed, no forbidden dependencies in domain or application layers. `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed clean. Source inspection: `grep -n "parse_file_config\|serde_json\|integration_target_id_str" cli/src/services/setup/mod.rs` shows matches only inside the new `#[cfg(test)]` facade tests (asserting against JSON the adapter wrote), confirming no config-schema parsing or JSON mutation implementation remains in `services::setup` production code.

- [x] T07: `Record the repository-config architecture and compatibility ownership` (status:done)
  - Task ID: T07
  - Goal: Update current-state context to describe the landed repository integration-configuration port/use-case/adapter slice and its retained setup compatibility facade.
  - Boundaries (in/out of scope): In — the context files listed under Context sync, updated only where code truth changed. Out — implementation changes, plan validation, unrelated config architecture, and cleanup beyond the requested stale prior-plan warning.
  - Dependencies: T06
  - Done when: Architecture and setup-bootstrap context identify the filesystem adapter as the owner of repository config I/O/JSON merge and identify the three setup functions as compatibility facades; glossary/map changes are made only when needed for discoverability.
  - Verification notes (commands or checks): Read the updated context claims against the final source layout; run `git diff --check`.
  - Evidence: Updated `context/architecture.md`'s "CLI internal hexagonal architecture" section with a new "third landed slice" paragraph naming `IntegrationTarget::config_id`, the `IntegrationConfigRepository` port, the three use cases (`EnsureRepoConfig`, `LoadPersistedOptionalWorkflows`, `RecordIntegrationInstallation`), `FilesystemIntegrationConfigRepository`'s `serde_json::Value`-based merge/compatibility behavior, and the three `services::setup` compatibility facades, matching the corresponding `context/sce/setup-repo-local-config-bootstrap.md` cross-reference used by the prior two slices; also extended the existing `cli/src/services/setup/mod.rs` bullet to name the moved-out config responsibilities and their facades. Rewrote the "Implementation" section of `context/sce/setup-repo-local-config-bootstrap.md` to point at the vertical slice (port, three use cases, `FilesystemIntegrationConfigRepository`) instead of the retired inline JSON mutation, named `persisted_optional_workflows` as the sole error-defaulting facade, and moved the bootstrap-payload/JSON-merge ownership sentence to the adapter while keeping the context-baseline-bootstrap sentence (a separate, earlier-landed slice) intact. Added four `context/glossary.md` entries: `repository integration-configuration vertical slice`, `IntegrationConfigRepository`, `EnsureRepoConfig` / `LoadPersistedOptionalWorkflows` / `RecordIntegrationInstallation`, and `FilesystemIntegrationConfigRepository`. Verified `context/context-map.md`'s existing `setup-repo-local-config-bootstrap.md` entry describes external behavior (bootstrap, persistence, precedence) rather than implementation ownership, so it remains accurate and was left unedited, matching the plan's "update only if materially incomplete" instruction. Ran the mandatory root pass over `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md`: `overview.md` and `patterns.md` contain no reference to this subsystem's functions or the hexagonal-slice vocabulary and are not contradicted by the completed implementation, so neither required an edit.
  - Verification: `git diff --check -- context/architecture.md context/glossary.md context/sce/setup-repo-local-config-bootstrap.md` — passed, no whitespace errors. Manually re-read the edited context claims against `cli/src/adapters/outbound/filesystem/integration_config_repository.rs`, `cli/src/application/ports/integration_config_repository.rs`, `cli/src/application/use_cases/{ensure_repo_config,load_persisted_optional_workflows,record_integration_installation}.rs`, and the three facades (`bootstrap_repo_local_config`, `persisted_optional_workflows`, `persist_integration_targets`) in `cli/src/services/setup/mod.rs` — every claim (port method names/signatures, `serde_json::Value` document representation, error-defaulting confined to `persisted_optional_workflows`, facade-over-use-case wiring) matches the final source layout.

## Open questions

None. The port, use-case responsibilities, adapter compatibility semantics,
facade boundaries, test obligations, and explicit non-goals are specified. The
only operational prerequisite is completion of the already-running validation
jobs, followed by removal of the stale prior-plan warning before T02 begins.

## Validation Report

**Status:** validated
**Date:** 2026-08-04

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (71 files generated, inventory hash matched)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (236 passed, 0 failed)
- `nix develop -c ./scripts/check-cli-architecture.sh` -> exit 0 (no forbidden dependencies in domain or application layers; AC1)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml integration_config_repository` -> exit 0 (14 passed; AC2/AC3/AC4/AC5)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml record_integration_installation` -> exit 0 (4 passed; AC2/AC6)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_id` -> exit 0 (1 passed; AC2)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` -> exit 0 (23 passed; AC6/AC7/AC8)
- `grep -n "parse_file_config\|serde_json\|integration_target_id_str" cli/src/services/setup/mod.rs` (source inspection) -> matches only inside the `mod tests` block starting at line 1350 (AC7)
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml` -> exit 0 (repair: applied rustfmt to the 5 files flagged by the prior `failed` run)
- `nix flake check` (retry) -> exit 0 (`all checks passed!` — `cli-architecture`, `cli-clippy`, `cli-fmt`, `cli-tests` all green)
- `nix run .#pkl-check-generated` (retry) -> exit 0 (71 files, same inventory hash)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (retry) -> exit 0 (236 passed, 0 failed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: `nix develop -c ./scripts/check-cli-architecture.sh` passed with no forbidden dependencies; the `cli-architecture` flake attribute also passed as part of the fully green `nix flake check` retry.
- [x] AC2: `config_id` test (1 passed) confirms `opencode`/`claude`/`pi`; `record_integration_installation` tests (4 passed) confirm `All` expansion in `OpenCode`, `Claude`, `Pi` order via a single repository call.
- [x] AC3: `integration_config_repository` adapter tests confirm bootstrap-on-missing and non-overwrite-on-existing behavior.
- [x] AC4: `integration_config_repository` adapter tests confirm unrelated-field preservation, target order, append/dedup, unknown-string preservation, and workflow replacement.
- [x] AC5: `integration_config_repository` adapter tests confirm stable invalid-JSON/non-object errors and the exactly-one-final-newline write contract.
- [x] AC6: `record_integration_installation`, `ensure_repo_config`/`load_persisted_optional_workflows` (covered by the full 236-test run), and `setup::` facade tests confirm delegation, `All` expansion, unchanged workflow forwarding, and error propagation.
- [x] AC7: `setup::` facade tests (23 passed) confirm facade behavior; source inspection confirms no `serde_json`/config-schema-parsing production code remains in `cli/src/services/setup/mod.rs` outside its test module.
- [x] AC8: `run_setup_for_mode_installs_then_persists_configuration_after_a_successful_install` and `run_setup_for_mode_leaves_no_recorded_target_when_installation_fails` (both in the `setup::` run) confirm the load/install/persist/render order and that a failed install leaves no recorded target.

### Failed checks and follow-ups

- None.

### Residual risks

- The five new/edited source files were untracked at the start of this validation run (`git status` showed `??` for the four new files); `nix flake check` builds from the git-tracked tree, so they were staged with `git add` (not committed) before running the check. No file content was changed by staging. The subsequent `cargo fmt` repair was also staged the same way for the retry.
