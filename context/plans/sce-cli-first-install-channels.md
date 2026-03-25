# Plan: sce-cli-first-install-channels

## Change summary

Revise the first-wave install/distribution plan so the repository no longer ships `install.sh` at all. The supported current-state install channels after this revision must be repo-flake Nix, Cargo, and npm only. This revision removes the installer script from repo code truth, updates user-facing install guidance to stop advertising it, and repairs durable context/decision records so future sessions do not treat the installer as an active channel.

## Success criteria

- The canonical product/package naming for the first wave remains `sce`, with any legacy `sce-editor` references either removed or explicitly mapped during migration.
- Users can install or run `sce` through all current in-scope channels for this plan revision: repo-flake Nix, Cargo, and npm.
- `install.sh` is removed from the repository with no compatibility stub or redirect script left behind.
- User-facing install guidance no longer describes the installer script as a supported path and instead points only at the supported channels above.
- Release/build automation and packaging references no longer depend on, publish, or validate the removed installer path.
- Durable context and decision records capture the revised current-state install/distribution contract for future sessions.

## Constraints and non-goals

- In scope: installing the `sce` CLI only.
- In scope: hard removal of `install.sh` and cleanup of code, docs, validation, and context that still treat it as supported current state.
- In scope: repo-flake Nix, Cargo, and npm as the only supported install channels after this revision.
- All build and packaging processes for this rollout must run through Nix-managed entrypoints rather than ad hoc host-native build commands.
- Out of scope for this plan revision: restoring the installer under a new name, adding a compatibility shim, or introducing replacement orchestration code.
- Out of scope for this plan: Homebrew, Debian `.deb`, AUR, rpm, Flatpak, AppImage, and other install methods unless they are touched only for removal/deferment wording.
- Out of scope: broad product rebranding beyond what is necessary to keep supported install surfaces coherent on `sce`.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Define the first-scope install contract` (status:done)
  - Task ID: T01
  - Goal: Record the canonical first-wave install/distribution contract for `sce`, including package naming, supported channels, required release artifacts, Nix-only build policy, and installer-script routing order.
  - Boundaries (in/out of scope): In - decision/context/docs updates that define the contract, lock installer priority, require Nix-managed build entrypoints, and de-scope unsupported channels for this phase. Out - implementing packaging logic or release automation.
  - Done when: A durable contract exists that names the supported first-wave channels (Nix, Homebrew, Cargo, npm, installer), standardizes on `sce`, states that Homebrew and npm consume Nix-produced release artifacts, records the installer fallback order (`Homebrew` on macOS, `Nix` when available, then `Cargo`, then `npm`, then clear failure), and makes the acceptance target for later tasks unambiguous.
  - Verification notes (commands or checks): Review `context/` and decision/doc updates for explicit channel matrix, naming policy, installer priority, Nix-only build rule, and first-wave scope boundaries.
  - Completed: 2026-03-25
  - Files changed: `context/decisions/2026-03-25-first-install-channels.md`, `context/sce/cli-first-install-channels-contract.md`, `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `docs/installation.md`
  - Evidence: Durable install contract added; installation doc trimmed to first-wave scope; root context updated for naming/build/routing policy.
  - Notes: Classified as an important context change because it defines cross-cutting install/distribution policy and canonical terminology for later tasks.

- [x] T02: `Add shared release artifact foundation for install channels` (status:done)
  - Task ID: T02
  - Goal: Implement the common release/build outputs required by downstream install channels, such as versioned CLI artifacts, checksums, and stable asset naming for `sce`.
  - Boundaries (in/out of scope): In - shared artifact packaging, naming normalization, Nix-owned build entrypoints, and release-workflow outputs consumed by multiple channels. Out - channel-specific Homebrew/AUR/npm/deb publication logic.
  - Done when: The repo can produce the canonical release assets and metadata that downstream channel tasks can consume without each channel inventing its own packaging shape, and those build steps are rooted in Nix-managed commands.
  - Verification notes (commands or checks): `nix flake check`; targeted verification that release workflow/config references stable `sce` asset names, checksum outputs, and Nix-based build entrypoints.
  - Completed: 2026-03-25
  - Files changed: `flake.nix`, `.github/workflows/release-sce.yml`, `.github/workflows/release-sce-linux.yml`, `.github/workflows/release-sce-macos-intel.yml`, `.github/workflows/release-sce-macos-arm.yml`, `.github/workflows/release-agents.yml`
  - Evidence: `nix run .#release-artifacts -- --help`; `nix run .#release-manifest -- --help`; `nix run .#release-artifacts -- --version 0.1.0 --out-dir /tmp/sce-release-task`; `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir /tmp/sce-release-task --out-dir /tmp/sce-release-task`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added Nix app entrypoints for per-platform archive/checksum generation and manifest assembly; moved CLI release automation into dedicated `sce` workflows with separate reusable CI files for Linux, macOS Intel, and macOS ARM while keeping `release-agents.yml` Tessl-only.

- [x] T03: `Finalize repo-flake Nix install path` (status:done)
  - Task ID: T03
  - Goal: Make the repository flake a supported install/run path for `sce`, with user-facing docs aligned to `nix run` / `nix profile install` from this repo.
  - Boundaries (in/out of scope): In - Nix package/app outputs, install-oriented docs, and any fixes required for the repo-flake user flow. Out - separate non-flake Nix packaging or NixOS module support.
  - Done when: A user can install or run `sce` from the repo flake using the documented commands, and the docs describe that path as a supported first-wave method.
  - Verification notes (commands or checks): `nix flake check`; verify documented `nix run github:crocoder-dev/sce` and `nix profile install github:crocoder-dev/sce` flows against the packaged app/output contract.
  - Completed: 2026-03-25
  - Files changed: `flake.nix`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`
  - Evidence: `nix run . -- --help`; `nix profile install --profile <tmp>/profile .`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added explicit `apps.default` alongside `apps.sce`, set package `meta.mainProgram = "sce"`, and aligned root context with the repo-flake default run/install surface. Per user direction, docs were intentionally left untouched in this session.

- [x] T04: `Add npm distribution for sce` (status:done)
  - Task ID: T04
  - Goal: Publish an npm install surface for `sce` that installs the CLI using the canonical release artifacts and package naming.
  - Boundaries (in/out of scope): In - npm package metadata/templates/scripts, release integration, and naming cleanup needed for `sce`. Out - supporting non-first-scope binary fallbacks or unrelated Node tooling changes.
  - Done when: The npm package can install or run `sce` using the agreed artifact contract, with docs pointing at the supported npm commands.
  - Verification notes (commands or checks): Targeted npm package smoke checks; verify package metadata, install script behavior, and doc examples all reference the canonical `sce` naming.
  - Completed: 2026-03-25
  - Files changed: `npm/package.json`, `npm/README.md`, `npm/bin/sce.js`, `npm/lib/platform.js`, `npm/lib/install.js`, `npm/test/platform.test.js`, `flake.nix`, `.github/workflows/release-sce.yml`
  - Evidence: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`; `nix run .#release-npm-package -- --help`; `nix run .#release-npm-package -- --version 0.1.0 --out-dir /tmp/sce-npm-release-task-3`; `tar -tzf /tmp/sce-npm-release-task-3/sce-v0.1.0-npm.tgz`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added a thin npm launcher package named `sce` that downloads canonical GitHub release archives, verifies SHA-256 checksums from the release manifest, installs the native binary into package-local runtime storage, and added a Nix-owned `release-npm-package` app plus release-workflow asset publication for the npm tarball and metadata. Per user direction, docs were deferred to T08.

- [ ] T05: `Add Homebrew release integration for sce` (status:cancelled)
  - Task ID: T05
  - Goal: Provide a working Homebrew distribution path for `sce` backed by release artifacts and deterministic formula generation/publication.
  - Boundaries (in/out of scope): In - formula source/generation, tap integration, release-workflow wiring, and checksum consumption. Out - Linux package manager work outside Homebrew.
  - Done when: Homebrew users have a documented and automation-backed install path for `sce`, and the formula references the canonical release artifacts.
  - Verification notes (commands or checks): Verify formula generation inputs/outputs, checksum references, and installation docs for `brew tap` / `brew install` consistency.
  - Cancelled: 2026-03-25
  - Notes: Removed from the current plan scope per user direction to skip Homebrew for now. If Homebrew returns to scope later, restore or replace this task under a revised install/distribution decision.

- [x] T06: `Add Cargo distribution for sce` (status:done)
  - Task ID: T06
  - Goal: Provide a supported Cargo install path for `sce` that fits the first-wave install contract and Nix-managed release/build policy.
  - Boundaries (in/out of scope): In - crate/install metadata, Cargo-facing docs, and any release/publish prerequisites needed for `cargo install` or `cargo binstall` guidance. Out - introducing non-Nix build orchestration or unrelated Rust packaging changes.
  - Done when: Cargo users have a documented, supported install path for `sce`, and that path is consistent with the first-wave naming/build contract.
  - Verification notes (commands or checks): Verify Cargo install guidance, crate metadata/readiness assumptions, and any release artifact references used by `cargo binstall`-style flows.
  - Completed: 2026-03-25
  - Files changed: `cli/Cargo.toml`, `cli/README.md`, `README.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Enabled crates.io publication posture for the `sce` crate, added crate-facing Cargo install guidance for crates.io, git, and local-path installs, and explicitly deferred `cargo binstall` to later work.

- [x] T07: `Implement first-wave installer script routing` (status:done)
  - Task ID: T07
  - Goal: Add a thin installer script, using the referenced `fresh` installer as a behavioral model, that detects platform and routes only across the supported first-wave methods.
  - Boundaries (in/out of scope): In - OS/distro detection, supported-method routing, clear failure messaging, and alignment with shipped package names/assets. Out - fallbacks for unsupported channels such as `.deb`, AUR, AppImage, rpm, or Flatpak.
  - Done when: The installer script uses Nix when available, falls back to Cargo before npm, and exits with clear guidance when no current in-scope route applies.
  - Verification notes (commands or checks): Review script decision order against the current plan scope; verify URLs/package names/method selection match the shipped channels and explicitly exclude unsupported fallback methods.
  - Completed: 2026-03-25
  - Files changed: `install.sh`, `context/plans/sce-cli-first-install-channels.md`
  - Evidence: `./install.sh --help`; `./install.sh --dry-run` (tool availability route); `env -i PATH=/usr/bin:/bin SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=1 SCE_INSTALLER_HAS_NPM=0 ./install.sh --dry-run`; `env -i PATH=/usr/bin:/bin SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=0 SCE_INSTALLER_HAS_NPM=1 ./install.sh --dry-run`; `env -i PATH=/usr/bin:/bin SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=0 SCE_INSTALLER_HAS_NPM=0 ./install.sh --dry-run`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added a thin root installer script that intentionally excludes Homebrew for this task revision and routes only across Nix, Cargo, then npm before emitting explicit failure guidance.

- [x] T08: `Sync installation docs and current-state context` (status:done)
  - Task ID: T08
  - Goal: Update `docs/installation.md` and relevant `context/` files so the installation documentation reflects only the implemented first-wave channels and their real commands/artifacts.
  - Boundaries (in/out of scope): In - installation doc edits, context sync, glossary/overview updates if the shipped install surface changes canonical terminology or cross-cutting behavior. Out - reintroducing unsupported install methods as active guidance.
  - Done when: `docs/installation.md` matches the delivered first-wave install surface, unsupported methods are removed or clearly marked out of scope, and `context/` reflects the current install/distribution reality.
  - Verification notes (commands or checks): Manual doc/context review for code-truth parity; confirm install commands, package names, release asset names, and supported-channel list are consistent everywhere.
  - Completed: 2026-03-25
  - Files changed: `README.md`, `context/decisions/2026-03-25-first-install-channels.md`, `context/sce/cli-release-artifact-contract.md`, `context/plans/sce-cli-first-install-channels.md`
  - Evidence: Manual parity review against `install.sh`, `cli/README.md`, `npm/README.md`, and current root context; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: `docs/installation.md` does not exist in the current repository, so this task aligned the user-facing install guidance and durable context through the existing README/context surfaces instead. The stale Homebrew-first decision/context wording was updated to match current code truth: repo-flake Nix, Cargo, npm, and the thin installer script only.

- [x] T09: `Run validation and cleanup for first-wave install support` (status:done)
  - Task ID: T09
  - Goal: Perform final validation across packaging, release metadata, installer behavior, docs, and context; remove temporary scaffolding or stale references left by the rollout.
  - Boundaries (in/out of scope): In - full repo validation, targeted packaging/install checks, stale-reference cleanup, and final context-sync verification. Out - new feature work beyond the first-wave install scope.
  - Done when: Required validation passes, obsolete first-wave-predecessor references are removed or updated, the plan has evidence that the first-wave install surface is coherent end-to-end, and no build/package path in scope depends on non-Nix build orchestration.
  - Verification notes (commands or checks): `nix flake check`; any targeted packaging/install smoke checks needed by prior tasks; verify `docs/installation.md` and relevant `context/` files reflect final current state.
  - Completed: 2026-03-25
  - Files changed: `context/plans/sce-cli-first-install-channels.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `nix run .#release-artifacts -- --version 0.1.0 --out-dir /tmp/sce-t09-release-artifacts`; `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir /tmp/sce-t09-release-artifacts --out-dir /tmp/sce-t09-release-manifest`; `nix run .#release-npm-package -- --version 0.1.0 --out-dir /tmp/sce-t09-npm-package`; `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`; `./install.sh --dry-run` with `SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=1 SCE_INSTALLER_HAS_NPM=0`; `./install.sh --dry-run` with `SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=0 SCE_INSTALLER_HAS_NPM=1`; `./install.sh --dry-run` with `SCE_INSTALLER_HAS_NIX=0 SCE_INSTALLER_HAS_CARGO=0 SCE_INSTALLER_HAS_NPM=0`; manual parity review against `README.md`, `install.sh`, `cli/README.md`, `npm/README.md`, and current context files
  - Notes: Final validation passed for the implemented first-wave channels (repo-flake Nix, Cargo, npm, and the thin installer script). No additional in-scope stale current-state install references required cleanup; root context remained verify-only for this task.

- [x] T10: `Remove install.sh and installer-owned code references` (status:done)
  - Task ID: T10
  - Goal: Delete `install.sh` from the repository and remove direct runtime/build/doc-source references that assume the installer script still exists as a shipped surface.
  - Boundaries (in/out of scope): In - deleting `install.sh`, removing repo surfaces that invoke, package, link to, or validate it, and tightening any install-channel lists in non-context source/docs to the remaining supported set. Out - durable context/decision sync (handled in T11), broad install-doc rewrites beyond installer removal, or introduction of replacement orchestration.
  - Done when: `install.sh` no longer exists in the repo, no remaining non-context source or user-facing repo docs present it as an available path, and any installer-specific verification hooks or references are removed or updated to the three supported channels.
  - Verification notes (commands or checks): Verify the repo no longer contains `install.sh`; review remaining non-context references to installer-script wording; run the narrowest repo validation needed for touched packaging/docs surfaces plus the repo baseline checks.
  - Completed: 2026-03-25
  - Files changed: `README.md`, `install.sh`, `context/plans/sce-cli-first-install-channels.md`
  - Evidence: `fff_grep install.sh` review confirmed no remaining non-context `install.sh` references after deletion aside from the active plan/context files; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Classified as an important change for context sync because the shipped first-wave install-channel contract changed from Nix/Cargo/npm plus installer to Nix/Cargo/npm only.

- [x] T11: `Sync install contract docs and durable context after installer removal` (status:done)
  - Task ID: T11
  - Goal: Update current-state install guidance, contract files, and decision/context records so future sessions treat repo-flake Nix, Cargo, and npm as the only supported first-wave channels.
  - Boundaries (in/out of scope): In - README/install guidance, context root files, focused install-contract context, and decision-record repairs needed because the supported-channel contract changed. Out - new packaging behavior, replacement installer design, or unrelated context churn.
  - Done when: Durable context and user-facing install docs no longer mention `install.sh` as supported current state, channel-count/routing wording is updated to Nix/Cargo/npm only, and any superseded installer-policy wording is repaired for code-truth parity.
  - Verification notes (commands or checks): Manual parity review across touched docs/context files against current code truth after T10; confirm supported-channel lists, guidance, and terminology consistently exclude the installer script.
  - Completed: 2026-03-25
  - Files changed: `context/sce/cli-first-install-channels-contract.md`, `context/decisions/2026-03-25-first-install-channels.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/cli-cargo-distribution-contract.md`
  - Evidence: Manual parity review against current code truth after `install.sh` removal; confirmed supported-channel wording now lists only repo-flake Nix, Cargo, and npm in durable context and user-facing repo guidance.
  - Notes: Classified as an important context change because root context and first-wave install contract wording changed from Nix/Cargo/npm plus installer to Nix/Cargo/npm only.

- [x] T12: `Run validation and cleanup for installer removal revision` (status:done)
  - Task ID: T12
  - Goal: Perform final validation for the hard-removal revision and capture end-state evidence that the repository no longer ships an installer script.
  - Boundaries (in/out of scope): In - full repo validation for touched surfaces, cleanup of stale installer references missed by earlier tasks, and final context-sync verification. Out - additional install-channel changes or new feature work.
  - Done when: Required validation passes, no in-scope stale `install.sh` current-state references remain, and the plan records evidence that the supported install surface is now repo-flake Nix, Cargo, and npm only.
  - Verification notes (commands or checks): Run the repo baseline validation (`nix run .#pkl-check-generated` and `nix flake check`) plus targeted checks for any touched docs/packaging surfaces; manually verify installer removal parity across repo docs and context.
  - Completed: 2026-03-25
  - Files changed: `context/plans/sce-cli-first-install-channels.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `rg -n "install\.sh" .`; manual parity review against `README.md`, `context/overview.md`, `context/context-map.md`, `context/glossary.md`, `context/sce/cli-first-install-channels-contract.md`, and `context/decisions/2026-03-25-first-install-channels.md`
  - Notes: No additional in-scope cleanup was required. Remaining `install.sh` mentions are historical references inside the active plan only; current-state docs and durable context now describe repo-flake Nix, Cargo, and npm as the only supported install channels.

## Open questions

- None. The user confirmed this revision should remove the installer script from the supported first-wave contract and perform a hard removal with no compatibility stub.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (flake outputs evaluated; `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, and `config-lib-tests` passed)
- `rg -n "install\.sh" .` -> matches only historical references in `context/plans/sce-cli-first-install-channels.md`; no current-state source, packaging, or durable-context surfaces still advertise the removed installer path

### Context and cleanup verification

- Manual parity review confirmed `README.md`, `cli/README.md`, `npm/README.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/cli-first-install-channels-contract.md`, and `context/decisions/2026-03-25-first-install-channels.md` all match the current first-wave install surface.
- No additional in-scope stale current-state install references required cleanup.
- No temporary scaffolding introduced by this task required removal.

### Success-criteria verification

- [x] Canonical naming is `sce` with legacy naming removed or explicitly deferred/mapped where relevant -> confirmed in `README.md`, `cli/README.md`, `npm/package.json`, and first-wave context files.
- [x] Users can install or run `sce` through all current in-scope channels for this revision: repo-flake Nix, Cargo, and npm -> confirmed by current user-facing guidance and durable context after installer removal.
- [x] `install.sh` is removed from the repository with no compatibility stub or redirect script left behind -> confirmed by search results showing only historical plan references.
- [x] User-facing install guidance no longer describes the installer script as a supported path and instead points only at the supported channels above -> confirmed by manual parity review across `README.md`, `cli/README.md`, `npm/README.md`, and durable context files.
- [x] Release/build automation and packaging references no longer depend on, publish, or validate the removed installer path -> confirmed by successful baseline validation plus absence of non-plan `install.sh` references.
- [x] Durable context and decision records capture the revised current-state install/distribution contract for future sessions -> confirmed by `context/sce/cli-first-install-channels-contract.md`, `context/decisions/2026-03-25-first-install-channels.md`, and discoverability entries in root context files.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for the current first-wave scope.
