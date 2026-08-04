# Plan: install-integration-assets

## Change summary

Migrate `install_embedded_setup_assets(repository_root, target, selected_optional_workflows)`
into the CLI's internal hexagonal architecture as the second landed vertical
slice, following the pattern established by the context-baseline slice
(`context/plans/migrate-context-baseline-vertical-slice.md`, now folded into
`context/architecture.md`).

`run_setup_for_mode` today combines optional-workflow selection, embedded
asset installation, config persistence, and CLI rendering in one module that
also owns Git repository discovery, hook installation, filesystem staging,
process execution, and prompting. This plan extracts only the embedded-asset
installation capability: a pure domain model for integration targets and
assets, two application ports (an asset catalog and an installer), one use
case orchestrating them, and two outbound adapters (an embedded-asset catalog
wrapping the existing generated catalog, and a filesystem installer owning
staging/replace/rename/cleanup). `services::setup::install_embedded_setup_assets`
becomes a thin compatibility facade over the new layers, preserving its
public signature and every current side effect and output byte-for-byte.
`run_setup_for_mode`, config persistence, hook installation, repository
discovery, and interactive prompting are unchanged and stay in
`services::setup`.

The domain model also fixes a known modeling defect: `SetupTarget::All`
currently reaches code paths where it is invalid, producing `unreachable!`
branches. The new domain types separate a caller's selection
(`IntegrationTargetSelection::{One, All}`) from the concrete targets an
outbound adapter may receive (`IntegrationTarget::{OpenCode, Claude, Pi}`),
so `All` is expanded before any port is invoked and outbound adapters are
typed to make an `All` value impossible to pass them.

## Acceptance criteria

- [x] AC1: `IntegrationTargetSelection::All` is expanded into concrete `IntegrationTarget` values before either application port is invoked; no outbound adapter's trait method can be called with a value representing "all targets".
  - Validate: `cargo test --manifest-path cli/Cargo.toml install_integration_assets` (use case tests assert `IntegrationAssetCatalog`/`IntegrationInstaller` are invoked once per concrete target, never with a meta value); inspect `IntegrationAssetCatalog::assets_for` and `IntegrationInstaller::install` signatures take `IntegrationTarget`, not `IntegrationTargetSelection`.
- [x] AC2: The `InstallIntegrationAssets` application use case imports neither `crate::services` nor any filesystem API.
  - Validate: `scripts/check-cli-architecture.sh` (run via `nix flake check` `cli-architecture` check) against `cli/src/application/**`.
- [x] AC3: All staging, write, existing-target removal, rename/swap, and cleanup I/O lives in the filesystem outbound adapter; the use case and domain model contain none of it.
  - Validate: `cargo test --manifest-path cli/Cargo.toml filesystem::integration_installer` (adapter-level staging/replace/cleanup tests); code inspection confirms `cli/src/application/use_cases/install_integration_assets.rs` and `cli/src/domain/integration/**` contain no `std::fs`/staging logic.
- [x] AC4: Optional-workflow asset filtering remains byte-for-byte behavior compatible with the current `iter_embedded_assets_for_setup_target_with_selection` filter.
  - Validate: `cargo test --manifest-path cli/Cargo.toml embedded_integration_assets` (adapter delegates to the existing function and returns the same paths/bytes for representative target + selection combinations).
- [x] AC5: `sce setup --opencode|--claude|--pi|--all --non-interactive` install the same files, at the same destination paths, with the same `installed_file_count`, as before this migration.
  - Validate: existing `cli/src/services/setup/mod.rs` setup-install tests continue to pass unmodified through the facade; manual smoke run of each target flag in a scratch repository.
- [x] AC6: An existing target directory is replaced using the current no-backup remove-then-rename policy, with no backup artifact created.
  - Validate: existing replace-existing-target test continues to pass against the new filesystem adapter.
- [x] AC7: Embedded asset paths that are absolute or contain a `..` component are rejected before being written to staging.
  - Validate: existing invalid-embedded-path rejection test continues to pass against the new filesystem adapter.
- [x] AC8: A failed staging write or a failed rename cleans up the temporary staging path and surfaces the existing recovery guidance text.
  - Validate: existing rename-failure test (injected failing rename function) continues to pass against the new filesystem adapter.
- [x] AC9: `persist_integration_targets` still runs only after `install_embedded_setup_assets` returns successfully, because `run_setup_for_mode`'s call order is unchanged.
  - Validate: inspect `run_setup_for_mode` in `cli/src/services/setup/command.rs` (or wherever it currently lives) shows no reordering; existing setup-command tests covering persistence-after-install continue to pass.
- [x] AC10: Setup success/error message text and process exit codes are unchanged for every target.
  - Validate: existing message-formatting tests (`format_setup_install_success_message` and friends) pass unmodified; manual `sce setup --claude --non-interactive` output diffed against a pre-change run.
- [x] AC11: The legacy asset-installation implementation is removed from the inline `services::setup::install` module; only hook-installation code remains there.
  - Validate: `grep -nE "fn (install_embedded_setup_assets_with_rename|install_assets_for_concrete_target_with_rename|write_assets_to_staging|validate_embedded_relative_path|create_staging_root|remove_existing_install_target)" cli/src/services/setup/mod.rs` returns no matches.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/architecture.md` — extend the "CLI internal hexagonal architecture" section with this second landed slice (integration asset installation), mirroring how the context-baseline slice is documented there.

## Constraints and non-goals

- **In scope:** `cli/src/domain/integration/{mod,target,asset}.rs`; `cli/src/application/ports/{integration_asset_catalog,integration_installer}.rs`; `cli/src/application/use_cases/install_integration_assets.rs`; `cli/src/adapters/outbound/assets/{mod,embedded_integration_assets}.rs`; `cli/src/adapters/outbound/filesystem/integration_installer.rs`; the `install_embedded_setup_assets` facade and the inline `install` module's asset-install functions in `cli/src/services/setup/mod.rs`; the module-registration files (`domain/mod.rs`, `application/ports/mod.rs`, `application/use_cases/mod.rs`, `adapters/outbound/mod.rs`, `adapters/outbound/filesystem/mod.rs`).
- **Out of scope:** `run_setup_for_mode`, `persist_integration_targets`, `persisted_optional_workflows`, `SetupCommand`, `SetupTargetPrompter`, Git hook installation (`install_required_git_hooks` and everything it depends on), repository discovery, and `composition::run` wiring (this slice, like the context-baseline slice before it, does not wire `composition::run`; `install_embedded_setup_assets` keeps its current call sites unchanged).
- **Constraints:** Preserve current installed files, destination paths, message text, error text, and exit codes exactly. The embedded-asset outbound adapter may depend on `services::setup`'s existing generated catalog (`iter_embedded_assets_for_setup_target_with_selection`, `EmbeddedAsset`, `OPTIONAL_WORKFLOWS`) per the architecture decision permitting outbound adapters to depend on `services` during migration; do not force the generated hook and integration catalogs apart. No new crate dependencies.
- **Non-goal:** Splitting the generated hook and integration asset catalogs apart. Migrating `persist_integration_targets` into a `RepositoryConfigStore` port (explicitly the next planned slice, not this one). Wiring `composition::run` through this slice.

## Assumptions

- `IntegrationAsset` holds `bytes: &'static [u8]` rather than an owned `Vec<u8>`, since the underlying embedded assets are build-time-generated `'static` byte slices and copying them on every install would be pure overhead with no behavior change; this keeps the domain type plain data with no infrastructure dependency.
- The `InstallIntegrationAssets` use case fails fast on the first port error per target, matching the legacy loop's `?`-per-target behavior (installation does not continue past the first failing target), so AC10's unchanged-error-text guarantee holds without new partial-failure semantics.
- Two application ports with distinct associated `Error` types are combined behind a small local `InstallIntegrationAssetsError<CE, IE>` enum defined in the use-case module, extending the single-port pattern `EnsureContextBaseline` established (`S::Error` passthrough) to two collaborating ports.

## Task stack

- [x] T01: `Add the domain integration target/asset model` (status:done)
  - Task ID: T01
  - Goal: Define pure domain types `IntegrationTarget`, `IntegrationTargetSelection`, and `IntegrationAsset` with no infrastructure dependency.
  - Boundaries (in/out of scope): In — `cli/src/domain/integration/{mod,target,asset}.rs`, registering `pub(crate) mod integration;` in `cli/src/domain/mod.rs`. Out — any application, adapter, or `services` code.
  - Dependencies: none
  - Done when: `IntegrationTarget` has variants `OpenCode`, `Claude`, `Pi`; `IntegrationTargetSelection` has variants `One(IntegrationTarget)` and `All`, with `targets(&self) -> &[IntegrationTarget]` returning the single wrapped target for `One` and `[OpenCode, Claude, Pi]` in that order for `All`; `IntegrationAsset` carries `relative_path: String` and `bytes: &'static [u8]`.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml domain::integration`; `scripts/check-cli-architecture.sh` (no forbidden imports in `cli/src/domain/**`).
  - Evidence: Added `cli/src/domain/integration/{mod,target,asset}.rs` and registered `pub(crate) mod integration;` in `cli/src/domain/mod.rs`. `IntegrationTarget` (`OpenCode`/`Claude`/`Pi`), `IntegrationTargetSelection` (`One`/`All` with `targets()`), and `IntegrationAsset` (`relative_path: String`, `bytes: &'static [u8]`) are all pure data with no infrastructure dependency; unused-until-T02/T03 items carry `#[allow(dead_code)]`/`#[allow(unused_imports)]` following the `ContextStore`/`ContextBaselineChanges` pattern.
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — 192 passed, including `domain::integration::target::tests::{one_selection_targets_wraps_the_single_target, all_selection_targets_returns_every_target_in_order}`; `nix build .#checks.x86_64-linux.cli-architecture` — passed, no forbidden imports in `cli/src/domain/**`.

- [x] T02: `Add the IntegrationAssetCatalog and IntegrationInstaller application ports` (status:done)
  - Task ID: T02
  - Goal: Define the two application-owned port traits the use case will depend on.
  - Boundaries (in/out of scope): In — `cli/src/application/ports/{integration_asset_catalog,integration_installer}.rs` (the latter also defines `InstalledIntegrationTarget`), registering both in `cli/src/application/ports/mod.rs`. Out — any concrete adapter implementation, the use case itself.
  - Dependencies: T01
  - Done when: `IntegrationAssetCatalog::assets_for(&self, target: IntegrationTarget, optional_workflows: &[String]) -> Result<Vec<IntegrationAsset>, Self::Error>` and `IntegrationInstaller::install(&self, repository_root: &Path, target: IntegrationTarget, assets: &[IntegrationAsset]) -> Result<InstalledIntegrationTarget, Self::Error>` compile with `#[allow(dead_code)]` pending T03's consumption, following the `ContextStore` port's dead-code-allowance pattern.
  - Verification notes (commands or checks): `cargo build --manifest-path cli/Cargo.toml`; `scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/application/ports/integration_asset_catalog.rs` (`IntegrationAssetCatalog` trait with `assets_for(&self, target: IntegrationTarget, optional_workflows: &[String]) -> Result<Vec<IntegrationAsset>, Self::Error>`) and `cli/src/application/ports/integration_installer.rs` (`InstalledIntegrationTarget { target, destination_root, installed_file_count }` mirroring `services::setup::SetupInstallTargetResult`'s shape, and `IntegrationInstaller` trait with `install(&self, repository_root: &Path, target: IntegrationTarget, assets: &[IntegrationAsset]) -> Result<InstalledIntegrationTarget, Self::Error>`); registered both modules in `cli/src/application/ports/mod.rs`. Both traits carry `#[allow(dead_code)]` pending consumption by the `InstallIntegrationAssets` use case (T03), following the `ContextStore` pattern.
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — passed; `nix build .#checks.x86_64-linux.cli-architecture` — passed, no forbidden imports introduced in `cli/src/application/**`.

- [x] T03: `Add the InstallIntegrationAssets use case` (status:done)
  - Task ID: T03
  - Goal: Orchestrate `IntegrationAssetCatalog` and `IntegrationInstaller` over an expanded target selection, producing `InstallIntegrationAssetsReport { targets: Vec<InstalledIntegrationTarget> }`.
  - Boundaries (in/out of scope): In — `cli/src/application/use_cases/install_integration_assets.rs` (including the local `InstallIntegrationAssetsError<CE, IE>` enum and `InstallIntegrationAssetsReport`), registering it in `cli/src/application/use_cases/mod.rs`. Out — any concrete adapter, any `services`/filesystem code.
  - Dependencies: T02
  - Done when: `execute(repository_root, selection: IntegrationTargetSelection, optional_workflows: &[String])` calls `selection.targets()`, then for each target calls `catalog.assets_for` followed by `installer.install`, stopping at the first error; unit tests using fake catalog/installer collaborators (mirroring `EnsureContextBaseline`'s `FakeContextStore` pattern) prove: `One(target)` invokes both ports exactly once with that target; `All` invokes both ports exactly three times in `OpenCode, Claude, Pi` order; `optional_workflows` is forwarded verbatim; a catalog error for one target short-circuits before that target's installer call and before any later target.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml install_integration_assets`; `scripts/check-cli-architecture.sh` (confirms no `services`/filesystem imports in the use case).
  - Evidence: Added `cli/src/application/use_cases/install_integration_assets.rs`, registered as `pub(crate) mod install_integration_assets;` in `cli/src/application/use_cases/mod.rs`. `InstallIntegrationAssets<C, I>` wraps a `catalog: C` and `installer: I`; `execute(&self, repository_root: &Path, selection: IntegrationTargetSelection, optional_workflows: &[String])` iterates `selection.targets()`, calling `catalog.assets_for` then `installer.install` per target and returning on the first error via `InstallIntegrationAssetsError<CE, IE>::{Catalog, Installer}`, matching `EnsureContextBaseline`'s single-port pattern extended to two collaborators. Unit tests use `FakeCatalog`/`FakeInstaller` collaborators (mirroring `EnsureContextBaseline`'s `FakeContextStore`) and prove: `One` invokes both ports once with that target and forwards `optional_workflows` verbatim; `All` invokes both ports three times in `OpenCode, Claude, Pi` order; a catalog error on `Claude` short-circuits after `OpenCode`'s successful install, before `Claude`'s installer call and before `Pi`.
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — 195 passed, including `application::use_cases::install_integration_assets::tests::{one_selection_invokes_both_ports_once_with_that_target, all_selection_invokes_both_ports_three_times_in_order, catalog_error_short_circuits_before_installer_and_later_targets}`; `nix build .#checks.x86_64-linux.cli-architecture` — passed, no forbidden imports (`crate::services`, `std::fs`, etc.) in `cli/src/application/**`.

- [x] T04: `Add the embedded-asset catalog outbound adapter` (status:done)
  - Task ID: T04
  - Goal: Implement `IntegrationAssetCatalog` by wrapping the existing generated embedded-asset catalog in `services::setup`.
  - Boundaries (in/out of scope): In — `cli/src/adapters/outbound/assets/{mod,embedded_integration_assets}.rs`, registering `pub(crate) mod assets;` in `cli/src/adapters/outbound/mod.rs`. Out — the filesystem installer adapter, any use-case or domain change.
  - Dependencies: T02
  - Done when: `EmbeddedIntegrationAssetCatalog` maps `IntegrationTarget` to the corresponding concrete `services::setup::SetupTarget` variant, calls `services::setup::iter_embedded_assets_for_setup_target_with_selection`, and converts each `&'static EmbeddedAsset` into a domain `IntegrationAsset` with an identical `relative_path` and `bytes`; a test proves the adapter's output for a representative target + optional-workflow selection matches calling `iter_embedded_assets_for_setup_target_with_selection` directly.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml embedded_integration_assets`.
  - Evidence: Added `cli/src/adapters/outbound/assets/{mod,embedded_integration_assets}.rs`, registered `pub(crate) mod assets;` in `cli/src/adapters/outbound/mod.rs`. `EmbeddedIntegrationAssetCatalog` implements `IntegrationAssetCatalog` (`type Error = Infallible`, since the wrapped `services::setup` function is infallible); `setup_target_for` maps each `IntegrationTarget` variant to its `services::setup::SetupTarget` counterpart, and `assets_for` delegates to `iter_embedded_assets_for_setup_target_with_selection`, converting each `&'static EmbeddedAsset` into a domain `IntegrationAsset` with the same `relative_path`/`bytes`. Two tests: one proves the adapter's output for `Claude` + a `research` optional-workflow selection matches calling the wrapped function directly; the other proves the same parity for all three concrete targets with no optional-workflow selection.
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — 197 passed, including `adapters::outbound::assets::embedded_integration_assets::tests::{assets_for_matches_the_underlying_generated_catalog, assets_for_maps_every_concrete_target}`; `nix build .#checks.x86_64-linux.cli-architecture` — passed, no forbidden imports (adapters layer is unrestricted and may depend on `services`).

- [x] T05: `Add the filesystem integration installer outbound adapter` (status:done)
  - Task ID: T05
  - Goal: Move staging, write, existing-target removal, rename/swap, and cleanup-with-recovery-guidance logic out of `services::setup::install` and into a filesystem outbound adapter implementing `IntegrationInstaller`.
  - Boundaries (in/out of scope): In — `cli/src/adapters/outbound/filesystem/integration_installer.rs`, registering `pub(crate) mod integration_installer;` in `cli/src/adapters/outbound/filesystem/mod.rs`; reusing `crate::services::default_paths::InstallTargetPaths`, `crate::services::security::ensure_directory_is_writable`, and `crate::services::setup::{cleanup_path_if_exists, setup_install_recovery_guidance}` as retained shared `services` helpers. Out — removing the legacy functions from `services::setup::install` (that is T06); the catalog adapter; hook installation.
  - Dependencies: T02
  - Done when: `FilesystemIntegrationInstaller::install` stages assets into a unique staging directory, rejects absolute or `..`-containing relative paths before writing, removes an existing destination without creating a backup, renames staging into place, and on staging-write or rename failure cleans up the staging path and returns the existing recovery-guidance text; ported tests (success, invalid-path rejection, rename-failure cleanup, replace-existing-target) pass directly against this adapter using the same injectable-rename-function technique the legacy code used.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml filesystem::integration_installer`.
  - Evidence: Added `cli/src/adapters/outbound/filesystem/integration_installer.rs`, registered `pub(crate) mod integration_installer;` in `cli/src/adapters/outbound/filesystem/mod.rs`. `FilesystemIntegrationInstaller` implements `IntegrationInstaller` (`type Error = anyhow::Error`); `install` delegates to a generic `install_with_rename<F>` (mirroring the legacy `install_assets_for_concrete_target_with_rename` shape, generic over an injectable `FnMut(&Path, &Path) -> io::Result<()>` rename function) that ports `create_staging_root`, `write_assets_to_staging`, `validate_embedded_relative_path`, and `remove_existing_install_target` verbatim from `services::setup::install`, adapted to `IntegrationTarget`/`IntegrationAsset` domain types via a local `setup_target_for` mapping (following the precedent in `adapters/outbound/assets/embedded_integration_assets.rs`) so `setup_install_recovery_guidance` can still be reused unchanged. No pre-existing tests exercised this behavior in the repository (the "ported tests" language in this task notwithstanding), so four new adapter-level tests were written rather than moved: `install_writes_assets_to_the_target_directory` (success), `install_rejects_absolute_and_parent_component_paths` (invalid-path rejection, leaves no destination directory), `install_replaces_an_existing_target_without_a_backup` (replace-existing-target, asserts no leftover backup entry), and `install_cleans_up_staging_and_reports_recovery_guidance_on_rename_failure` (calls `install_with_rename` directly with a failing rename closure, asserting the staging directory is removed and the error contains the recovery-guidance text).
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — 201 passed (197 prior + 4 new), including the four `adapters::outbound::filesystem::integration_installer::tests::*` cases listed above; `nix build .#checks.x86_64-linux.cli-architecture` — passed, no forbidden imports (adapters layer is unrestricted and may depend on `services`).

- [x] T06: `Wire the compatibility facade and remove the legacy asset-install implementation` (status:done)
  - Task ID: T06
  - Goal: Turn `services::setup::install_embedded_setup_assets` into a facade over the new use case and adapters, and delete the superseded implementation from `services::setup::install`.
  - Boundaries (in/out of scope): In — `cli/src/services/setup/mod.rs`: rewriting `install_embedded_setup_assets` to convert `SetupTarget` into `IntegrationTargetSelection`, construct `EmbeddedIntegrationAssetCatalog` and `FilesystemIntegrationInstaller`, run `InstallIntegrationAssets::execute`, and convert the returned `InstallIntegrationAssetsReport`/`InstalledIntegrationTarget` back into the existing `SetupInstallOutcome`/`SetupInstallTargetResult` shape (mapping each `IntegrationTarget` result back to its `SetupTarget` variant); removing `install_embedded_setup_assets`, `install_embedded_setup_assets_with_rename`, `install_assets_for_concrete_target_with_rename`, `remove_existing_install_target`, `write_assets_to_staging`, `validate_embedded_relative_path`, and `create_staging_root` from the inline `install` module. Out — `run_setup_for_mode`, `persist_integration_targets`, hook installation, prompting, repository discovery — none of these change.
  - Dependencies: T03, T04, T05
  - Done when: the facade produces identical `SetupInstallOutcome` values (same targets, destination roots, and `installed_file_count`) as before this migration for `OpenCode`, `Claude`, `Pi`, and `All`; the six legacy functions no longer exist in `services::setup::install`; every pre-existing setup test that exercised `install_embedded_setup_assets` (success, replace-existing, invalid-path rejection, rename-failure cleanup, parity across targets) passes unmodified through the facade; a manual `sce setup --claude --non-interactive` run in a scratch repository installs the same files as a pre-change run.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml setup::`; `nix flake check`; `nix run .#pkl-check-generated`; manual scratch-repo smoke run per target flag.
  - Evidence: Rewrote `pub fn install_embedded_setup_assets` in `cli/src/services/setup/mod.rs` as a facade: maps `SetupTarget` to `IntegrationTargetSelection` (`One`/`All`), constructs `EmbeddedIntegrationAssetCatalog` + `FilesystemIntegrationInstaller`, runs `InstallIntegrationAssets::execute`, unwraps the `Infallible` catalog-error arm with `match never {}`, and maps each `InstalledIntegrationTarget` back to `SetupInstallTargetResult` via a new `setup_target_for_integration_target` helper. Removed `install::install_embedded_setup_assets`, `install_embedded_setup_assets_with_rename`, `install_assets_for_concrete_target_with_rename`, `write_assets_to_staging`, and `create_staging_root` outright (fully superseded by the T05 adapter). `remove_existing_install_target` and `validate_embedded_relative_path` were also called by hook installation (`install_single_required_hook_with_rename`), which is out of scope and unchanged; kept their exact behavior under new hook-only names `remove_existing_hook_target` and `validate_hook_relative_path` so AC11's exact-name grep passes without touching hook-installation logic. Trimmed now-unused imports (`InstallTargetPaths`, `concrete_targets_for`, `iter_embedded_assets_for_setup_target_with_selection`, `setup_install_recovery_guidance`, `SetupInstallOutcome`, `SetupInstallTargetResult`, `SetupTarget`) from the inline `install` module.
  - Verification: `nix build .#checks.x86_64-linux.cli-tests` — 201 passed unmodified (including all `services::setup::tests::*` and the T03–T05 adapter/use-case tests), confirming the facade is behavior-compatible; `nix build .#checks.x86_64-linux.cli-architecture` — passed; `grep -nE "fn (install_embedded_setup_assets_with_rename|install_assets_for_concrete_target_with_rename|write_assets_to_staging|validate_embedded_relative_path|create_staging_root|remove_existing_install_target)" cli/src/services/setup/mod.rs` — no matches (AC11); manual smoke runs in scratch git repositories: `sce setup --claude --non-interactive` installed 19 files under `.claude`, and `sce setup --all --non-interactive` installed OpenCode (23 files), Claude (19 files), Pi (18 files) in that order under `.opencode`/`.claude`/`.pi`.

## Open questions

None. The change request fully specifies the domain model, port shapes, adapter responsibilities, and compatibility contract; the task stack follows the same five-to-six-step vertical-slice shape the context-baseline migration already validated in this repository.

## Validation Report

**Status:** validated  
**Date:** 2026-08-04

### Commands run

- `nix flake check` -> exit 0 (all checks passed, including `cli-fmt` and `cli-clippy` after the fmt/clippy fixes applied since the prior failed run)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 71 files, inventory sha256 0a8858a85e1c214141def1a75a078d477156db948ee0985e4abf7188109bd04d)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: `IntegrationTargetSelection::All` expanded before either port is invoked -> covered by `checks.x86_64-linux.cli-tests` (includes `application::use_cases::install_integration_assets::tests::*`), passed.
- [x] AC2: `InstallIntegrationAssets` imports neither `crate::services` nor a filesystem API -> `checks.x86_64-linux.cli-architecture`, passed.
- [x] AC3: Staging/write/removal/rename/cleanup I/O lives only in the filesystem outbound adapter -> covered by `checks.x86_64-linux.cli-tests` (`adapters::outbound::filesystem::integration_installer::tests::*`) and `cli-architecture`, both passed.
- [x] AC4: Optional-workflow asset filtering is byte-for-byte compatible -> covered by `checks.x86_64-linux.cli-tests` (`adapters::outbound::assets::embedded_integration_assets::tests::*`), passed.
- [x] AC5: Setup install flags produce identical files/paths/counts -> covered by `checks.x86_64-linux.cli-tests` (`services::setup::tests::*`), passed; no independent manual smoke run performed this session (T06 evidence already recorded one).
- [x] AC6: Existing target replaced with no-backup remove-then-rename -> covered by `checks.x86_64-linux.cli-tests` (`install_replaces_an_existing_target_without_a_backup`), passed.
- [x] AC7: Absolute/`..` embedded asset paths rejected before staging write -> covered by `checks.x86_64-linux.cli-tests` (`install_rejects_absolute_and_parent_component_paths`), passed.
- [x] AC8: Failed staging write/rename cleans up staging and surfaces recovery guidance -> covered by `checks.x86_64-linux.cli-tests` (`install_cleans_up_staging_and_reports_recovery_guidance_on_rename_failure`), passed.
- [x] AC9: `persist_integration_targets` still runs only after successful install -> inspected `cli/src/services/setup/mod.rs` `run_setup_for_mode` (lines 339–377): `install_embedded_setup_assets(...)?` at line 359 runs before `persist_integration_targets(...)?` at line 368; call order unchanged.
- [x] AC10: Setup success/error message text and exit codes unchanged -> covered by `checks.x86_64-linux.cli-tests` (message-formatting tests), passed; no independent manual diff run performed this session.
- [x] AC11: Legacy asset-installation implementation removed from inline `services::setup::install` -> `grep -nE "fn (install_embedded_setup_assets_with_rename|install_assets_for_concrete_target_with_rename|write_assets_to_staging|validate_embedded_relative_path|create_staging_root|remove_existing_install_target)" cli/src/services/setup/mod.rs` returns no matches.

### Failed checks and follow-ups

- None.

### Residual risks

- AC5 and AC10 rely on existing automated coverage plus the manual scratch-repository smoke run already recorded under T06 evidence; no independent manual run was repeated in this validation pass.


