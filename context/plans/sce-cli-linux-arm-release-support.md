# Plan: sce-cli-linux-arm-release-support

## Change summary

Add end-to-end Linux ARM release support for `sce`, targeting `aarch64-unknown-linux-gnu`.

The current repository already recognizes Linux ARM in some downstream packaging/install surfaces, but the GitHub release workflow stack still lacks a dedicated Linux ARM build lane and the durable release-support context still describes only three automated release targets. This plan closes that gap so GitHub Releases, npm installation, and current-state context all agree that Linux ARM is a supported release/install target.

## Success criteria

- GitHub Actions includes a dedicated reusable Linux ARM release workflow that builds canonical `sce` artifacts for `aarch64-unknown-linux-gnu`.
- `.github/workflows/release-sce.yml` invokes that Linux ARM workflow and requires it before publishing the GitHub release.
- Published release artifacts and merged release-manifest outputs include the Linux ARM archive/metadata/checksum alongside the existing targets.
- The npm launcher package and its tests continue to recognize `linux/arm64` as a supported install platform with no workflow-vs-installer drift.
- Durable context and user-facing npm/release docs describe Linux ARM as an officially supported current-state target.
- Final validation covers the touched workflow/npm/release surfaces and records evidence in the plan.

## Constraints and non-goals

- In scope: `.github/workflows/**`, `npm/**`, and the durable context/docs needed to describe the resulting current-state support matrix.
- In scope: aligning workflow automation, npm install expectations, and context/docs around `aarch64-unknown-linux-gnu` support.
- In scope: task slicing that keeps each executable task to one coherent commit unit.
- Out of scope: adding musl-based Linux ARM support.
- Out of scope: adding Windows targets or any additional release channels beyond the existing GitHub Releases/Cargo/npm topology.
- Out of scope: redesigning the release artifact naming contract or the npm trust/signing model.
- Out of scope: broad refactors to the release pipeline unrelated to Linux ARM enablement.

## Task stack

- [x] T01: `Add a reusable Linux ARM release workflow` (status:done)
  - Task ID: T01
  - Goal: Add a dedicated reusable GitHub Actions workflow for Linux ARM that builds canonical `sce` release artifacts for `aarch64-unknown-linux-gnu` using the existing per-platform workflow pattern.
  - Boundaries (in/out of scope): In - a new `.github/workflows/release-sce-linux-arm.yml` workflow and any minimal workflow-local configuration needed to run the existing `nix run .#release-artifacts` path on a Linux ARM runner. Out - orchestrator wiring, release job dependency changes, npm/package/docs/context edits.
  - Done when: A reusable Linux ARM workflow file exists, accepts the same release inputs as the other per-platform workflows, runs on an ARM Linux runner, builds the canonical release artifacts, and uploads them under a deterministic Linux ARM artifact name.
  - Verification notes (commands or checks): Manual workflow review against `.github/workflows/release-sce-linux.yml`, `.github/workflows/release-sce-macos-*.yml`, and the `release-artifacts` target-triple contract in `flake.nix`.
  - Implementation notes: Added `.github/workflows/release-sce-linux-arm.yml` as a reusable Linux ARM workflow that mirrors the existing per-platform release pattern, runs on `ubuntu-24.04-arm`, invokes the canonical `nix run .#release-artifacts` path, and uploads artifacts under `sce-release-aarch64-unknown-linux-gnu`. Verified with `git diff --check` on touched workflow/context files plus manual parity review against the existing Linux/macOS reusable release workflows and the `release-artifacts` target-triple contract in `flake.nix`.

- [x] T02: `Wire Linux ARM into the release orchestrator` (status:done)
  - Task ID: T02
  - Goal: Update the top-level CLI release workflow so Linux ARM builds are invoked and required before the GitHub release is assembled.
  - Boundaries (in/out of scope): In - `.github/workflows/release-sce.yml` job graph/dependencies and any release-asset download assumptions needed so Linux ARM artifacts are merged into the final release assembly. Out - npm installer code, user-facing docs, or durable context updates.
  - Done when: The orchestrator calls the Linux ARM reusable workflow with the resolved release ref/version, the final `release` job depends on its success, and the artifact download/release publication path includes the Linux ARM outputs without special-case drift.
  - Verification notes (commands or checks): Manual parity review of `.github/workflows/release-sce.yml` against the per-platform workflow matrix; confirm the orchestrator now covers Linux x64, Linux ARM, macOS Intel, and macOS ARM.
  - Implementation notes: Updated `.github/workflows/release-sce.yml` to invoke `.github/workflows/release-sce-linux-arm.yml` with the same resolved ref/version inputs as the other reusable release lanes, added Linux ARM to the `release` job `needs` gate, and required `build-linux-arm` success before manifest assembly and GitHub release publication. Verified with `git diff --check` plus manual parity review confirming the orchestrator now includes Linux x64, Linux ARM, macOS Intel, and macOS ARM in the release matrix and artifact merge path.

- [x] T03: `Align npm install/docs with the supported Linux ARM release matrix` (status:done)
  - Task ID: T03
  - Goal: Ensure the npm package’s documented and tested support matrix remains explicitly aligned with the now-complete release automation for Linux ARM.
  - Boundaries (in/out of scope): In - `npm/README.md`, `npm/test/**`, and any narrow npm-package metadata or assertions needed to keep Linux ARM support explicit and drift-resistant. Out - workflow implementation beyond what T01/T02 already cover, and durable context updates outside npm-facing/current-state release docs.
  - Done when: npm docs and tests explicitly reflect that `linux/arm64` maps to `aarch64-unknown-linux-gnu` as a supported release/install target, and no npm-facing wording still implies Linux ARM is unsupported or unimplemented.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`.
  - Implementation notes: Updated `npm/README.md` to list the supported npm platform matrix explicitly, including `linux/arm64` → `aarch64-unknown-linux-gnu`, and added npm-platform test coverage to keep unsupported-platform guidance aligned with that documented support set. Verified with `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`.

- [x] T04: `Sync release-support context for Linux ARM current state` (status:done)
  - Task ID: T04
  - Goal: Update durable context so the repository’s supported automated release targets and npm install matrix include Linux ARM as current-state truth.
  - Boundaries (in/out of scope): In - focused edits to release/npm context files and root shared files only where the supported-target matrix is part of current-state documentation. Out - implementation changes to workflows/npm code.
  - Done when: Durable context no longer lists only three automated targets, Linux ARM is documented as supported where appropriate, and `context/context-map.md` remains accurate for any touched context files.
  - Verification notes (commands or checks): Manual parity review across `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/overview.md`, `context/glossary.md`, and `context/context-map.md` against the implemented workflow/npm truth.
  - Implementation notes: Updated `context/sce/cli-release-artifact-contract.md` and `context/sce/cli-npm-distribution-contract.md` to state the current four-target release/install matrix explicitly, added root shared-file current-state references in `context/overview.md`, `context/glossary.md`, and `context/context-map.md`, and kept the edits aligned with `.github/workflows/release-sce.yml`, `.github/workflows/release-sce-linux-arm.yml`, `npm/README.md`, and `npm/test/platform.test.js`. Verified with manual parity review plus `git diff --check -- context/overview.md context/glossary.md context/context-map.md context/sce/cli-release-artifact-contract.md context/sce/cli-npm-distribution-contract.md context/plans/sce-cli-linux-arm-release-support.md`.

- [x] T05: `Run validation and cleanup for Linux ARM release support` (status:done)
  - Task ID: T05
  - Goal: Execute the final validation pass for the Linux ARM release-support rollout, confirm touched workflow/npm/context surfaces are aligned, and leave the plan ready for implementation handoff/closure.
  - Boundaries (in/out of scope): In - targeted npm validation, lightweight generated-output parity if touched, repo validation appropriate to the changed surfaces, and final context-sync verification. Out - new feature work beyond Linux ARM release support.
  - Done when: Required validation passes, no in-scope workflow/npm/context drift remains, and the plan records evidence that Linux ARM support is coherent end to end.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; `nix run .#pkl-check-generated` (if generated outputs change); `nix flake check`; manual parity review across `.github/workflows/release-sce.yml`, `.github/workflows/release-sce-linux-arm.yml`, `.github/workflows/release-sce-linux.yml`, `npm/README.md`, `npm/test/platform.test.js`, `context/sce/cli-release-artifact-contract.md`, and `context/sce/cli-npm-distribution-contract.md`.
  - Implementation notes: Ran `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` (24 tests passed) and `nix flake check` (all 10 flake checks passed, including `pkl-parity`). Manually reviewed `.github/workflows/release-sce.yml`, `.github/workflows/release-sce-linux-arm.yml`, `.github/workflows/release-sce-linux.yml`, `npm/README.md`, `npm/test/platform.test.js`, `context/sce/cli-release-artifact-contract.md`, and `context/sce/cli-npm-distribution-contract.md`; all remained aligned on the four-target release/install matrix including `aarch64-unknown-linux-gnu`, so no additional cleanup changes were required.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` -> exit 0 (`24 pass`, `0 fail`)
- `nix flake check` -> exit 0 (all 10 configured checks passed, including `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, and `config-lib-biome-format`)

### Temporary scaffolding

- None found or required for this task.

### Context sync result

- Classified as **verify-only** for root context sync: this final task recorded validation evidence and confirmed existing Linux ARM release/install documentation remained current.
- Verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` against the current workflow/npm/context truth; no root-context edits were required.
- Confirmed durable feature documentation already exists and remains discoverable via `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, and `context/context-map.md`.

### Success-criteria verification

- [x] GitHub Actions includes a dedicated reusable Linux ARM release workflow that builds canonical `sce` artifacts for `aarch64-unknown-linux-gnu`.
  Evidence: `.github/workflows/release-sce-linux-arm.yml` defines the reusable workflow on `ubuntu-24.04-arm` and uploads `sce-release-aarch64-unknown-linux-gnu` artifacts.
- [x] `.github/workflows/release-sce.yml` invokes that Linux ARM workflow and requires it before publishing the GitHub release.
  Evidence: `.github/workflows/release-sce.yml` includes `build-linux-arm` in both the reusable workflow invocation and the `release` job `needs`/success gate.
- [x] Published release artifacts and merged release-manifest outputs include the Linux ARM archive/metadata/checksum alongside the existing targets.
  Evidence: the release orchestrator downloads `sce-release-*` artifacts into `dist/cli` with `merge-multiple: true`, then runs `nix run .#release-manifest`; `context/sce/cli-release-artifact-contract.md` documents the four-target current state.
- [x] The npm launcher package and its tests continue to recognize `linux/arm64` as a supported install platform with no workflow-vs-installer drift.
  Evidence: `npm/README.md` lists `linux/arm64` -> `aarch64-unknown-linux-gnu`; `npm/test/platform.test.js` covers supported Linux ARM mapping; `bun test ./test/*.test.js` passed.
- [x] Durable context and user-facing npm/release docs describe Linux ARM as an officially supported current-state target.
  Evidence: `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/overview.md`, `context/glossary.md`, and `context/context-map.md` all reflect the four-target matrix including Linux ARM.
- [x] Final validation covers the touched workflow/npm/release surfaces and records evidence in the plan.
  Evidence: this validation report plus the T05 implementation notes capture command outputs and manual parity review across the named workflow/npm/context files.

### Failed checks and follow-ups

- None.

### Residual risks

- No in-scope drift identified after validation. Linux ARM runtime artifact production still depends on GitHub-hosted ARM runner availability during release execution.

## Open questions

- None. The user confirmed the target is `aarch64-unknown-linux-gnu`, that downstream npm support is in scope, and that success means end-to-end supported-release status rather than a workflow-only partial step.
