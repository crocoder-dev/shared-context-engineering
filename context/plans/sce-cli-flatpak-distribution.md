# Add Flatpak distribution for the `sce` CLI

## Change summary

Add Flatpak as an officially supported `sce` CLI distribution channel by introducing a Flathub-ready manifest and clear local build/install documentation. The first Flatpak iteration is intentionally limited to source-built Flatpak packaging and docs; it does not add a full CI publishing pipeline or automated Flathub submission workflow.

## Success criteria

- Flatpak is represented as an official supported install channel in current-state distribution context, replacing the prior first-wave statement that Flatpak is out of scope.
- A Flathub-ready manifest exists for application ID `dev.crocoder.sce` and builds the Rust CLI from source inside the Flatpak build rather than wrapping prebuilt Nix/GitHub Release artifacts.
- The manifest exposes the `sce` CLI command and includes the Flatpak metadata/files expected for Flathub review, with the minimum sandbox permissions needed for a CLI tool.
- Local documentation explains how to build, install, run, and uninstall/test the Flatpak package from a checkout.
- Verification covers manifest syntax/buildability as far as local tooling allows, repository checks, and context sync for the new distribution channel.

## Constraints and non-goals

- In scope: Flathub-ready manifest, Flatpak metadata, local build/install documentation, and current-state context updates for Flatpak support.
- Out of scope for this iteration: CI publishing, automatic Flathub submission, GitHub Release Flatpak assets, wrapper packages around prebuilt binaries, release-version bumping, and changes to existing Cargo/npm/Nix publication workflows unless required by Flatpak source builds.
- The Flatpak package must build `sce` from source inside the Flatpak manifest.
- The Flatpak application ID is `dev.crocoder.sce`.
- Existing Nix, Cargo, npm, and release-artifact contracts remain source-of-truth for their channels and should not be regressed.
- Each executable task below is scoped as one atomic commit unit.

## Task stack

- [ ] T01: `Approve Flatpak distribution contract in context` (status:todo)
  - Task ID: T01
  - Goal: Update current-state distribution context so Flatpak is an official supported channel for the first Flatpak iteration, with source-built manifest/docs scope and no publishing pipeline commitment.
  - Boundaries (in/out of scope): In - update `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md` downstream-channel notes if needed, `context/context-map.md`, `context/overview.md`, and `context/glossary.md` entries that currently describe supported channels. Out - application code, manifest implementation, docs outside context.
  - Done when: Context no longer lists Flatpak as out of scope for the active distribution model; it records `dev.crocoder.sce`, source-built Flatpak packaging, and local-docs-only scope for this iteration; existing Nix/Cargo/npm contracts remain intact.
  - Verification notes (commands or checks): Review changed context files for contradictions with the new Flatpak scope; no shell validation required beyond later full-plan checks.

- [ ] T02: `Add source-built Flatpak manifest and metadata` (status:todo)
  - Task ID: T02
  - Goal: Add a Flathub-ready Flatpak manifest for `dev.crocoder.sce` that builds the Rust `sce` CLI from repository source and installs the `sce` command.
  - Boundaries (in/out of scope): In - new Flatpak packaging files such as a manifest, metainfo/appstream metadata, icon/desktop metadata only if required for Flathub readiness, and any minimal build-support files needed for Flatpak source builds. Out - CI workflows, release asset generation, prebuilt binary downloads, and unrelated packaging refactors.
  - Done when: The manifest uses application ID `dev.crocoder.sce`, builds from source, installs/runs `sce`, declares only required runtime/build dependencies and permissions, and includes Flathub-review-oriented metadata without depending on Nix-produced or GitHub Release binary artifacts.
  - Verification notes (commands or checks): Run the narrowest available Flatpak manifest lint/build command if Flatpak tooling is installed; otherwise document the missing local tooling and perform static review of manifest paths, app ID, modules, sources, command, and permissions. Later T05 runs repository-wide validation.

- [ ] T03: `Document local Flatpak build and install workflow` (status:todo)
  - Task ID: T03
  - Goal: Add user-facing documentation for building, installing, running, validating, and uninstalling the Flatpak package from a local checkout.
  - Boundaries (in/out of scope): In - README or dedicated docs updates that explain prerequisites, local `flatpak-builder` flow, install/run commands, expected binary invocation, troubleshooting, and the current no-publish-pipeline boundary. Out - marketing copy, release automation docs, and changes to npm/Cargo/Nix install instructions except adding Flatpak to the channel list.
  - Done when: A contributor can follow docs to build/install the Flatpak locally and understands that this iteration is Flathub-ready manifest/docs only, not automated publication.
  - Verification notes (commands or checks): Review commands for consistency with the manifest path/app ID; if Flatpak tooling is available, smoke-test the documented local flow or record why it could not be run.

- [ ] T04: `Add Flatpak package checks where practical` (status:todo)
  - Task ID: T04
  - Goal: Add focused validation coverage for Flatpak packaging that is practical within this repository's existing check model.
  - Boundaries (in/out of scope): In - static checks, manifest lint/test hooks, or lightweight Nix/dev-shell support needed to validate the Flatpak manifest without making full Flatpak builds part of default `nix flake check` unless intentionally lightweight and deterministic. Out - full CI publishing, mandatory network-heavy Flathub builds in default checks, and broad release workflow changes.
  - Done when: The repo has a clear, documented verification path for the Flatpak manifest, and any added check is deterministic, scoped, and does not regress existing default validation expectations.
  - Verification notes (commands or checks): Run the newly added Flatpak-specific check if available; run `nix flake check` if the task changes flake/dev-shell/check surfaces.

- [ ] T05: `Validate Flatpak distribution and sync context` (status:todo)
  - Task ID: T05
  - Goal: Run final validation for the Flatpak distribution slice, clean up temporary scaffolding, and ensure durable context matches code truth.
  - Boundaries (in/out of scope): In - full repository validation, Flatpak-specific validation evidence, cleanup of temporary build files, and final context sync for Flatpak distribution docs/contracts. Out - publishing to Flathub, creating GitHub release assets, or starting a new distribution channel beyond the approved Flatpak slice.
  - Done when: Required repository checks pass or failures are documented with actionable blockers; Flatpak build/lint/docs checks have evidence; no temporary Flatpak build artifacts are left in the repo; context files accurately describe Flatpak as an official source-built manifest/docs channel.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; Flatpak-specific command(s) introduced or documented by T02-T04, such as `flatpak-builder --force-clean --user --install-deps-from=flathub <build-dir> <manifest>` when local tooling is available.

## Open questions

- None. Clarified decisions: Flatpak is now an official supported distribution channel; first iteration is Flathub-ready manifest plus local build/install docs only; the manifest builds from source; Flatpak app ID is `dev.crocoder.sce`.
