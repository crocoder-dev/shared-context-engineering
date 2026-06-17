# Plan: Nix-orchestrated Flatpak distribution for `sce`

## Change summary

Add Flatpak as an officially supported `sce` CLI distribution channel by introducing a Flathub-ready source-built manifest, Nix-backed local build/check entrypoints, and clear local build/install documentation.

The Flatpak package must remain source-built: Nix may provide and orchestrate Flatpak tooling, generate local build overrides, and run validation, but the Flatpak manifest must not copy or download a Nix-built `sce` binary, GitHub Release binary artifact, npm native binary, or other prebuilt `sce` artifact. This first Flatpak iteration does not add CI publishing, automatic Flathub submission, GitHub Release Flatpak assets, or release-version bumping.

Resolved implementation decisions:

- Flatpak application ID is `dev.crocoder.sce`.
- Use a Flathub-style release-source manifest plus a Nix-generated local checkout override for local builds from the current checkout.
- Use a host `git` bridge for Flatpak runtime Git access: install a small `/app/bin/git` wrapper that delegates to `flatpak-spawn --host git`, and add the required Flatpak permission for `org.freedesktop.Flatpak`.
- Use the standard Flatpak Rust source-build pattern with the Freedesktop SDK Rust extension, offline Cargo dependency sources generated from `cli/Cargo.lock`, and build-time preparation of `cli/assets/generated/config/**` from checked-in `config/` inputs.

## Success criteria

- Flatpak is represented as an official supported install channel in current-state distribution context, replacing the prior first-wave statement that Flatpak is out of scope.
- A Flathub-ready manifest exists for application ID `dev.crocoder.sce` and builds the Rust CLI from source inside Flatpak rather than wrapping prebuilt Nix/GitHub Release/npm binary artifacts.
- The manifest source model is suitable for Flathub review while the Nix local build path can build from the current checkout through a generated local-source override.
- Nix provides the preferred local orchestration path for Flatpak tooling/build validation while preserving the manifest's source-built Flatpak semantics.
- The package exposes the `sce` CLI command and includes expected Flatpak metadata/files for Flathub review, with a minimal sandbox profile for a CLI tool plus the explicit host-git bridge permission.
- Local documentation explains how to build, install, run, validate, and uninstall/test the Flatpak package from a checkout using the Nix-backed path first, with a direct `flatpak-builder` fallback where practical.
- Verification covers manifest syntax/buildability as far as local tooling allows, repository checks, Nix-backed Flatpak entrypoints, and context sync for the new distribution channel.

## Constraints and non-goals

- In scope: Flathub-ready manifest, AppStream/metainfo metadata for a console application, optional icon/desktop metadata only if validation requires it, host-git bridge wrapper, Nix-backed Flatpak build/check entrypoints, local build/install documentation, and current-state context updates for Flatpak support.
- Out of scope: CI publishing, automatic Flathub submission, GitHub Release Flatpak assets, Flatpak wrappers around prebuilt binaries, release-version bumping, and changes to existing Cargo/npm/Nix publication workflows unless required to orchestrate source-built Flatpak validation.
- Nix may install/provide `flatpak-builder`, AppStream validation tools, `flatpak-builder-lint`, wrapper scripts, flake apps, opt-in checks, and generated local manifests/overrides used to build/test the Flatpak package locally.
- The Flatpak package must build `sce` from source inside Flatpak and must not consume `nix build .#sce`, packaged flake outputs, GitHub Release tarballs, npm native binaries, or other prebuilt `sce` artifacts.
- Existing Nix, Cargo, npm, and release-artifact contracts remain source-of-truth for their channels and should not be regressed.
- GitHub Releases remain the canonical binary artifact publication surface for existing release channels; Flatpak intentionally supersedes the generic future-channel reuse guidance by using a source-built Flatpak manifest instead of consuming release archives.
- Default `nix flake check` integration should remain lightweight and deterministic; network-heavy Flatpak builds should be opt-in unless the implementation can make them reliably cheap and non-interactive.
- Each executable task below is scoped as one atomic commit unit.

## Task stack

- [x] T01: `Approve Nix-orchestrated Flatpak contract in context` (status:done)
  - Task ID: T01
  - Goal: Update current-state distribution context so Flatpak is an official supported channel whose local build/check path is Nix-orchestrated while the Flatpak package remains source-built.
  - Boundaries (in/out of scope): In - update `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md` downstream-channel notes if needed, `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` entries that currently describe supported channels or Nix-owned build policy. Also repair pre-existing conflict-marker text in touched context sections if it blocks accurate current-state context. Out - application code, Flatpak manifest implementation, Nix code, and user-facing docs outside `context/`.
  - Done when: Context no longer lists Flatpak as out of scope for the active distribution model; it records `dev.crocoder.sce`, the source-built Flatpak requirement, the release-source-plus-local-override model, the allowed Nix orchestration role, the host-git bridge decision, and the no-publish-pipeline scope for this iteration; existing Nix/Cargo/npm contracts remain intact.
  - Verification notes (commands or checks): Review changed context files for contradictions, especially wording that implies Flatpak wraps `nix build .#sce`, GitHub Release binaries, or npm native binaries; no shell validation required beyond later full-plan checks.
  - Completed: 2026-06-16
  - Files changed: `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/optional-install-channel-integration-test-entrypoint.md`, `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/plans/nix-orchestrated-flatpak.md`.
  - Evidence: static review via `Grep` confirmed no merge conflict markers under `context/**/*.md`; targeted stale-wording search found only active-plan references; `git diff --check` returned clean; Flatpak/current-state wording now records `dev.crocoder.sce`, source-built semantics, release-source plus Nix-generated local checkout override model, allowed Nix orchestration role, host-git bridge, no-publish-pipeline scope, and binary-artifact non-consumption in active distribution context.
  - Context sync: completed as an important-change pass; root context files and distribution-domain files were updated because the supported distribution-channel contract changed.

- [x] T02: `Add source-built Flatpak manifest and metadata` (status:done)
  - Task ID: T02
  - Goal: Add a Flathub-ready Flatpak packaging surface for `dev.crocoder.sce` that builds the Rust `sce` CLI from source and installs the `sce` command.
  - Boundaries (in/out of scope): In - new Flatpak packaging files such as `packaging/flatpak/dev.crocoder.sce.yml`, `dev.crocoder.sce.metainfo.xml`, generated/checked-in Cargo dependency sources from `cli/Cargo.lock`, a host-git bridge wrapper installed as `/app/bin/git`, and minimal build-support files needed for Flatpak source builds. Out - CI workflows, release asset generation, prebuilt binary downloads, Nix-built binary copying, and unrelated packaging refactors.
  - Done when: The manifest uses application ID `dev.crocoder.sce`, declares the Freedesktop runtime/SDK and Rust SDK extension, builds from source within Flatpak, prepares required generated CLI assets from checked-in `config/` inputs before Cargo builds, installs/runs `sce`, installs AppStream metadata for a console application with `<provides><binary>sce</binary></provides>`, declares only required runtime/build dependencies and permissions including the explicit host-git bridge permission, and contains no references to Nix-produced or GitHub/npm binary artifacts.
  - Verification notes (commands or checks): Run the narrowest available Flatpak manifest lint/build command if Flatpak tooling is installed; otherwise document missing local tooling and perform static review of manifest paths, app ID, modules, sources, command, permissions, host-git wrapper, metadata, Cargo source list, and absence of prebuilt `sce` artifact references. Later T05 runs repository-wide validation.
  - Completed: 2026-06-16
  - Files changed: `packaging/flatpak/dev.crocoder.sce.yml`, `packaging/flatpak/dev.crocoder.sce.metainfo.xml`, `packaging/flatpak/git-host-bridge`, `packaging/flatpak/cargo-sources.json`, `context/sce/cli-first-install-channels-contract.md`, `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/plans/nix-orchestrated-flatpak.md`.
  - Evidence: `python3` static manifest/source-descriptor assertions passed (`dev.crocoder.sce`, `sce` command, Rust SDK extension, generated-asset preparation, offline locked Cargo source build, `/app/bin/sce`, `/app/bin/git`, metainfo install, host-git permission, Turso source patching, 1044 cargo source entries); `appstreamcli validate --pedantic packaging/flatpak/dev.crocoder.sce.metainfo.xml` passed; static prebuilt-artifact grep under `packaging/flatpak` found no `nix build`, GitHub Release, npm, or prebuilt/native-binary references; local `flatpak-builder` and `flatpak-builder-lint` were unavailable, so no Flatpak build/lint command was run; `git diff --check` passed; `nix run .#pkl-check-generated` passed; `nix flake check` was run and failed in pre-existing `checks.x86_64-linux.cli-fmt` on committed `cli/src/generated_migrations.rs` formatting, which this task did not modify.
  - Context sync: completed as an important-change pass; Flatpak distribution context now points to the implemented `packaging/flatpak/` manifest, AppStream metadata, host-git wrapper, and Cargo source descriptor, with root toolchain context repaired to the current flake-pinned Rust `1.95.0`.

- [x] T03: `Add Nix-backed Flatpak build and validation entrypoints` (status:done)
  - Task ID: T03
  - Goal: Add Nix-based developer entrypoints that provide Flatpak tooling and orchestrate local manifest validation/builds without making the Flatpak package consume Nix-built `sce` binaries.
  - Boundaries (in/out of scope): In - flake/dev-shell support, flake app(s), scripts, generated local-source override/manifest support for current-checkout builds, AppStream/Flatpak lint entrypoints, and opt-in checks for invoking `flatpak-builder`/`flatpak-builder-lint`/`appstreamcli` against the manifest. Out - mandatory network-heavy Flatpak builds in default `nix flake check` unless explicitly lightweight, CI publishing, Flathub submission automation, and changes to existing release artifact generation.
  - Done when: Contributors have a Nix-provided command/check path for Flatpak validation and local package build; the local build path uses a generated checkout-source override rather than a prebuilt `sce` binary; any default-check integration is intentional, deterministic, and documented; the dev shell or app output surfaces Flatpak tooling availability consistently with existing flake-app conventions.
  - Verification notes (commands or checks): Run the new Nix-backed Flatpak command/check if practical; run `nix flake check` if the task changes flake/dev-shell/check surfaces; statically review that Nix orchestration passes sources/manifests into Flatpak tooling and does not bypass the Flatpak source build.
  - Completed: 2026-06-16
  - Files changed: `flake.nix`, `packaging/flatpak/sce-flatpak.sh`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/optional-install-channel-integration-test-entrypoint.md`, `context/plans/nix-orchestrated-flatpak.md`.
  - Evidence: `bash -n packaging/flatpak/sce-flatpak.sh` passed; `packaging/flatpak/sce-flatpak.sh validate --skip-optional-lint` passed static source-build checks, local-manifest generation checks, and `appstreamcli validate --pedantic --no-net`; `nix run .#flatpak-validate -- --skip-optional-lint` passed; `nix run .#flatpak-local-manifest` passed and emitted a generated checkout-source manifest; `nix run .#flatpak-build -- --help` passed; `nix build .#checks.x86_64-linux.flatpak-static-validation` passed; `nix run .#pkl-check-generated` passed; `nix flake check` evaluated the new Flatpak apps/check/dev-shell outputs and then failed in pre-existing `checks.x86_64-linux.cli-fmt` on committed `cli/src/generated_migrations.rs` formatting, matching the T02 known failure and not introduced by T03. Because the Flatpak packaging files from T02 are currently untracked in this worktree, Git-backed flake commands that directly reference them were run with temporary `git add packaging/flatpak` staging and immediate `git reset -- packaging/flatpak` cleanup.
  - Context sync: completed as an important-change pass; root and distribution-domain context now describes the implemented Linux Flatpak flake apps/check/dev-shell tooling, `packaging/flatpak/sce-flatpak.sh` local-manifest generation, no-net lightweight validation, and opt-in full `flatpak-builder` build boundary.

- [x] T04: `Document local Flatpak build and install workflow` (status:done)
  - Task ID: T04
  - Goal: Add user-facing documentation for building, installing, running, validating, and uninstalling the Flatpak package from a local checkout, with the Nix-backed workflow as the preferred path.
  - Boundaries (in/out of scope): In - README or dedicated docs updates that explain prerequisites, the Nix-backed Flatpak command/check, direct `flatpak-builder` fallback where practical, release-source vs local-checkout override behavior, install/run commands, host-git bridge implications, troubleshooting, uninstall/cleanup commands, and the current no-publish-pipeline boundary. Out - marketing copy, release automation docs, and changes to npm/Cargo/Nix install instructions except adding Flatpak to the supported channel list.
  - Done when: A contributor can follow docs to build/install the Flatpak locally via Nix orchestration, understands how the manifest remains source-built, can run/uninstall/test the package, understands Git-dependent commands use the host-git bridge, and understands that this iteration is Flathub-ready manifest/docs/tooling only, not automated publication.
  - Verification notes (commands or checks): Review documented commands for consistency with the manifest path, app ID, Nix entrypoint names, local build directory names, and host-git bridge behavior; if Flatpak tooling is available, smoke-test the documented local flow or record why it could not be run.
  - Completed: 2026-06-16
  - Files changed: `README.md`
  - Evidence: `git diff --check` clean; static consistency review confirmed all documented commands/paths/IDs match manifest (`dev.crocoder.sce.yml`), metainfo (`dev.crocoder.sce.metainfo.xml`), Nix entrypoints (`flatpak-validate`, `flatpak-local-manifest`, `flatpak-build`), shell script (`sce-flatpak.sh`), host-git bridge (`git-host-bridge`), and cargo sources (`cargo-sources.json`); `nix run .#pkl-check-generated` passed; `nix run .#flatpak-validate -- --skip-optional-lint` passed.

- [x] T05: `Validate Flatpak distribution and sync context` (status:done)
  - Task ID: T05
  - Goal: Run final validation for the Flatpak distribution slice, clean up temporary scaffolding, and ensure durable context matches code truth.
  - Boundaries (in/out of scope): In - full repository validation, Nix-backed Flatpak validation evidence, direct Flatpak-specific validation evidence where practical, cleanup of temporary build files, final plan evidence, and final context sync for Flatpak distribution docs/contracts. Out - publishing to Flathub, creating GitHub Release Flatpak assets, changing existing release channel behavior, or starting a new distribution channel beyond the approved Flatpak slice.
  - Done when: Required repository checks pass or failures are documented with actionable blockers; Nix-backed and Flatpak-specific build/lint/docs checks have evidence; no temporary Flatpak build artifacts are left in the repo; context files accurately describe Flatpak as an official source-built, Nix-orchestrated local build/docs channel with release-source plus local override behavior and host-git bridge semantics.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; Nix-backed Flatpak command/check introduced by T03; direct Flatpak-specific command(s) introduced or documented by T02-T04, such as AppStream validation and `flatpak-builder --force-clean --user --install-deps-from=flathub <build-dir> <manifest>` when local tooling is available; static search for stale context/docs wording that still says Flatpak is out of scope or implies prebuilt-binary consumption.
  - Completed: 2026-06-17
  - Files changed: `context/plans/nix-orchestrated-flatpak.md`, `context/sce/cli-first-install-channels-contract.md`.
  - Evidence: `nix run .#pkl-check-generated` passed; `nix run .#flatpak-validate -- --skip-optional-lint` passed; `nix build --no-link .#checks.x86_64-linux.flatpak-static-validation` passed; `nix run .#flatpak-local-manifest` passed and emitted a generated checkout-source manifest under `/tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/sce-flatpak-manifest.*`; `nix run .#flatpak-build -- --help` passed; direct `appstreamcli validate --pedantic --no-net packaging/flatpak/dev.crocoder.sce.metainfo.xml` passed; direct `packaging/flatpak/sce-flatpak.sh validate --skip-optional-lint` passed; direct Flatpak tooling was available (`flatpak-builder-1.4.4`, remotes included `flathub`), so the opt-in source build was attempted with `/tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/opencode` build/manifest/state paths, `--install-deps-from flathub`, `--user`, `--state-dir`, `--delete-build-dirs`, and `-y`; it downloaded dependencies and reached `Building module sce` but exceeded the 45-minute task timeout before completion. `nix flake check` was run and failed in the pre-existing `checks.x86_64-linux.cli-fmt` blocker already observed in T02/T03: `cli/src/generated_migrations.rs` needs rustfmt cleanup near the generated auth migration array. Actionable blocker: run `nix develop -c sh -c 'cd cli && cargo fmt'` (or regenerate/fix the formatted generated file if that is the intended owner) and rerun `nix flake check` in a separate approved scope.
  - Cleanup: Removed the repo-local `.flatpak-builder/` cache and the `/tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/opencode/sce-flatpak-*` build/manifest/state directories left by the timed-out Flatpak build attempt, including stopping the leftover `rofiles-fuse` process before deletion. Final Flatpak artifact check found no `.flatpak-builder*` paths in the repo.
  - Static review: Stale wording search found no `Flatpak out of scope` wording in `README.md` or `context/sce/*.md`; broader context matches were limited to the active plan's historical/success-criteria text. Packaging prebuilt-artifact search found banned binary-source strings only inside `packaging/flatpak/sce-flatpak.sh` validation denylist checks, not in the manifest, metadata, Cargo source descriptor, or host-git wrapper.
  - Context sync: completed as a verify-only root pass with one domain-context drift repair; root context already described the implemented source-built Flatpak channel, and `context/sce/cli-first-install-channels-contract.md` was corrected to match the manifest's actual Cargo command (`cargo --offline build --release --manifest-path cli/Cargo.toml --bin sce`, without `--locked`). `git diff --check` passed after the context edit.

## Validation Report - 2026-06-17

### Commands run

- `git diff --check` -> exit 0 (no whitespace errors).
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`).
- `nix run .#flatpak-validate -- --skip-optional-lint` -> exit 0 (static checks, AppStream validation, and generated local-manifest validation passed).
- `nix run .#flatpak-validate` -> exit 0 (same validation passed; reported `flatpak-builder-lint not found; optional Flathub lint skipped.`).
- `nix build --no-link .#checks.x86_64-linux.flatpak-static-validation` -> exit 0 (Flatpak static validation derivation built successfully without creating a `result` symlink).
- `nix run .#flatpak-local-manifest` -> exit 0 (emitted a generated checkout-source manifest under `/tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/sce-flatpak-manifest.*`).
- `nix run .#flatpak-build -- --help` -> exit 0 (build app help rendered expected `sce-flatpak build` options).
- `appstreamcli validate --pedantic --no-net packaging/flatpak/dev.crocoder.sce.metainfo.xml` -> exit 0 (`Validation was successful.`).
- `packaging/flatpak/sce-flatpak.sh validate --skip-optional-lint` -> exit 0 (direct script validation passed).
- `flatpak-builder --version && flatpak remotes --columns=name` -> exit 0 (`flatpak-builder-1.4.4`; remotes included `flathub`).
- `nix run .#flatpak-build -- --build-dir /tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/opencode/sce-flatpak-build/dev.crocoder.sce --manifest-out /tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/opencode/sce-flatpak-manifest --install-deps-from flathub --user -- --state-dir=/tmp/nix-shell.xBAUE9/nix-shell.SNEmRO/opencode/sce-flatpak-state --delete-build-dirs -y` -> timed out after 45 minutes after dependency downloads and reaching `Building module sce`; temporary build/state artifacts were cleaned.
- `nix flake check` -> exit 1 due to pre-existing `checks.x86_64-linux.cli-fmt` failure in `cli/src/generated_migrations.rs` formatting, matching the blocker already recorded in T02/T03.

### Failed checks and follow-ups

- Full `nix flake check` remains blocked by the existing `cli-fmt` issue in `cli/src/generated_migrations.rs` near the generated auth migration array. Follow-up: in a separately approved scope, run `nix develop -c sh -c 'cd cli && cargo fmt'` (or repair the generated file through its intended generation owner), then rerun `nix flake check`.
- The opt-in full Flatpak source build exceeded the 45-minute task timeout after reaching the Rust build phase. Follow-up: rerun the same `nix run .#flatpak-build` command with a longer timeout or in an interactive developer environment when full Flatpak build evidence is required.
- `flatpak-builder-lint` was not available on PATH; the optional lint step was skipped by the validation command without failing the required static/AppStream checks.

### Success-criteria verification

- [x] Flatpak is documented as an official supported source-built channel in durable context (`context/sce/cli-first-install-channels-contract.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`).
- [x] The checked-in manifest and metadata exist under `packaging/flatpak/` for application ID `dev.crocoder.sce` and expose the `sce` command plus AppStream `<provides><binary>sce</binary></provides>`; confirmed by static validation and AppStream validation.
- [x] The local build model uses a generated checkout-source manifest instead of a prebuilt binary; confirmed by `nix run .#flatpak-local-manifest`, `sce-flatpak.sh` validation, and static source-build assertions.
- [x] Nix-backed Flatpak entrypoints exist and validate: `flatpak-validate`, `flatpak-local-manifest`, `flatpak-build`, and `flatpak-static-validation`.
- [x] Stale wording/static prebuilt-artifact search found no current-state docs claiming Flatpak is out of scope and no Flatpak manifest/metadata/source files consuming prebuilt `sce` artifacts.
- [x] Temporary Flatpak artifacts introduced or discovered during validation were removed; final artifact check found no repo-local `.flatpak-builder*` paths.

### Residual risks

- Repository-level sign-off is not fully green until the existing `cli-fmt` blocker is fixed.
- Full Flatpak source-build completion remains unproven in this session due to timeout, although static validation, AppStream validation, local-manifest generation, and build invocation all reached expected stages.
- Pre-existing unrelated worktree changes remain untouched: `.gitignore` is modified and `claude/` is untracked.

## Open questions

- None. Clarified decisions: Flatpak is an official supported distribution channel for this slice; first iteration is Flathub-ready manifest plus Nix-backed local build/check tooling and docs only; Nix may orchestrate the Flatpak build and generate local-source overrides; the Flatpak manifest must build `sce` from source and must not consume Nix-built, GitHub Release, or npm-native `sce` binaries; Flatpak app ID is `dev.crocoder.sce`; runtime Git access uses a host `git` bridge through `flatpak-spawn --host git`.
