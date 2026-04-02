# Plan: drop-macos-intel-release-support

## Change summary

Stop supporting macOS Intel in the CLI release pipeline. Remove the macOS Intel workflow lane, job names, artifact handling, and supported-matrix references so the repository's current release contract reflects Apple Silicon-only macOS support.

## Success criteria

- The release orchestrator no longer invokes a macOS Intel reusable workflow or waits on a macOS Intel build job.
- The repository no longer contains the dedicated macOS Intel release workflow or `macos-intel`-named release jobs/artifact producers for the CLI release flow.
- The npm launcher no longer advertises or resolves `darwin/x64` / `x86_64-apple-darwin` as a supported install target.
- Current-state release/context docs no longer advertise `x86_64-apple-darwin` as a supported automated release target.
- Validation confirms the release workflow graph, artifact contract references, and context files are internally consistent after the removal.

## Constraints and non-goals

- In scope: release workflow topology, supported-matrix documentation, and current-state context updates required to reflect the removal.
- In scope: npm launcher platform mapping, npm-facing support docs, and tests that currently encode macOS Intel support.
- In scope: removing `build-macos-intel` / `build-macos-intel-artifacts` references and the dedicated macOS Intel workflow file.
- Out of scope: introducing Rosetta-based Intel cross-build support.
- Out of scope: changing Linux release lanes, Cargo/npm downstream publish behavior, or non-macOS artifact naming.
- Out of scope: broader installer/distribution redesign beyond removing macOS Intel support from current code truth.

## Task stack

- [x] T01: `Remove macOS Intel release workflow lane` (status:done)
  - Task ID: T01
  - Goal: Remove the macOS Intel reusable workflow entrypoint and the orchestrator references that currently schedule and require that lane.
  - Boundaries (in/out of scope): In - `.github/workflows/release-sce.yml` orchestration edges, required job list, success gate logic, and deletion of `.github/workflows/release-sce-macos-intel.yml`. Out - changes to surviving Linux/macOS ARM lane behavior, artifact format, or release manifest assembly logic unrelated to the removed lane.
  - Done when: The top-level release workflow no longer defines `build-macos-intel`, no longer depends on a macOS Intel result in the release job, and the dedicated macOS Intel reusable workflow file is removed.
  - Verification notes (commands or checks): Inspect `.github/workflows/release-sce.yml` to confirm no `build-macos-intel` references remain; verify `.github/workflows/release-sce-macos-intel.yml` is absent; run the repo validation flow that covers workflow/config integrity.
  - Completed: 2026-04-02
  - Files changed: `.github/workflows/release-sce.yml`, `.github/workflows/release-sce-macos-intel.yml`, `context/plans/drop-macos-intel-release-support.md`
  - Evidence: targeted workflow reference searches passed; `nix flake check` passed
  - Notes: Removed the macOS Intel reusable workflow lane and release-gate dependency without changing surviving lane behavior.

- [x] T02: `Remove macOS Intel support from release contract docs` (status:done)
  - Task ID: T02
  - Goal: Update release-contract documentation and related current-state wording so macOS support is described as Apple Silicon-only.
  - Boundaries (in/out of scope): In - focused context files plus npm-facing docs that describe release topology, workflow names, supported target matrices, or npm install support. Out - unrelated CLI/install docs that do not mention the release matrix.
  - Done when: Current-state context and npm-facing docs no longer list `x86_64-apple-darwin`, no longer reference `.github/workflows/release-sce-macos-intel.yml`, and no longer describe a four-target matrix that includes macOS Intel.
  - Verification notes (commands or checks): Inspect `context/sce/cli-release-artifact-contract.md`, `context/overview.md`, `context/context-map.md`, `context/glossary.md`, and `npm/README.md` as needed to confirm macOS Intel references were removed or corrected consistently.
  - Completed: 2026-04-02
  - Files changed: `context/overview.md`, `context/plans/drop-macos-intel-release-support.md`
  - Evidence: targeted context/doc reference searches passed after narrowing scope; `context/sce/cli-release-artifact-contract.md` already matched the 3-target automated release contract
  - Notes: Kept this task strictly scoped to release-contract/current-state docs; npm launcher docs/code were deferred to T03 and are now resolved there.

- [x] T03: `Remove macOS Intel npm launcher support` (status:done)
  - Task ID: T03
  - Goal: Remove macOS Intel from the npm launcher's supported platform map, user-facing errors/docs, and test coverage.
  - Boundaries (in/out of scope): In - `npm/lib/platform.js`, npm tests, and npm-facing support messaging/documentation. Out - redesigning the npm installer flow or adding Rosetta-based fallback behavior.
  - Done when: The npm launcher no longer maps `darwin/x64` to `x86_64-apple-darwin`, unsupported-platform messaging no longer advertises `darwin/x64`, and npm tests/docs align with the reduced matrix.
  - Verification notes (commands or checks): Inspect `npm/lib/platform.js`, `npm/README.md`, and npm tests for removed `darwin/x64` / `x86_64-apple-darwin` support references; run the narrow npm test slice that covers platform resolution/install behavior.
  - Completed: 2026-04-02
  - Files changed: `npm/lib/platform.js`, `npm/test/platform.test.js`, `npm/README.md`, `context/sce/cli-npm-distribution-contract.md`, `context/context-map.md`, `context/glossary.md`, `context/plans/drop-macos-intel-release-support.md`
  - Evidence: `nix develop -c sh -c 'cd npm && bun test ./test/platform.test.js'` passed; `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` passed; targeted reference searches removed active `darwin/x64` / `x86_64-apple-darwin` npm support references from current-state docs
  - Notes: Removed macOS Intel npm launcher support without adding alternate fallback behavior.

- [x] T04: `Sync remaining current-state references to Apple Silicon-only macOS support` (status:done)
  - Task ID: T04
  - Goal: Remove any remaining repository references that imply macOS Intel release support and align current-state files with the new supported matrix.
  - Boundaries (in/out of scope): In - residual references in repo-owned docs/config metadata touched by the release-support change. Out - speculative future platform additions or Rosetta-based replacement work.
  - Done when: Residual `macos-intel`, `release-sce-macos-intel`, `build-macos-intel`, `darwin/x64`, and `x86_64-apple-darwin` support references tied to the current release matrix are either removed or intentionally retained only where historical/deferred wording is explicitly required.
  - Verification notes (commands or checks): Run targeted repository searches for `macos-intel`, `release-sce-macos-intel`, `build-macos-intel`, `darwin/x64`, and `x86_64-apple-darwin`; confirm each remaining match is intentional and not advertising active support.
  - Completed: 2026-04-02
  - Files changed: `flake.nix`, `npm/test/platform.test.js`, `context/sce/cli-release-artifact-contract.md`, `context/plans/drop-macos-intel-release-support.md`
  - Evidence: targeted repo searches now leave only intentional plan-history references; `nix develop -c sh -c 'cd npm && bun test ./test/platform.test.js'` passed; `nix run .#release-artifacts -- --version 0.2.0-pre-alpha-v1 --out-dir <tmp>` passed on the supported Linux host
  - Notes: Removed the last active macOS Intel release-target mapping from the flake release-artifact helper; remaining plan-file references are historical execution context rather than active support.

- [x] T05: `Run validation and cleanup for release-matrix removal` (status:done)
  - Task ID: T05
  - Goal: Execute final validation, ensure no stale references remain, and leave context aligned with the removed macOS Intel lane.
  - Boundaries (in/out of scope): In - final repo verification, generated/parity checks if touched surfaces require them, and plan/context cleanup for this change. Out - new feature work or unrelated release refactors.
  - Done when: Required validation passes, any temporary scaffolding/search leftovers are removed, and the plan/task evidence is sufficient for handoff completion.
  - Verification notes (commands or checks): Run `nix run .#pkl-check-generated` if generated/config surfaces changed; run `nix flake check`; perform a final search for removed lane identifiers and confirm current-state context matches code truth.
  - Completed: 2026-04-02
  - Files changed: `context/plans/drop-macos-intel-release-support.md`
  - Evidence: `nix flake check` passed; final repository search for `x86_64-apple-darwin|darwin/x64|macos-intel|release-sce-macos-intel|build-macos-intel` left only intentional plan-history references; no temporary scaffolding remained
  - Notes: `nix run .#pkl-check-generated` was not required because this plan did not touch generated config surfaces.

## Open questions

- None.

## Validation Report

### Commands run

- `nix flake check` -> exit 0 (all repo checks passed; included `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`)
- `rg -n "x86_64-apple-darwin|darwin/x64|macos-intel|release-sce-macos-intel|build-macos-intel"` -> exit 0 (matches remained only in `context/plans/drop-macos-intel-release-support.md` as intentional execution history)

### Failed checks and follow-ups

- None.

### Success-criteria verification

- [x] The release orchestrator no longer invokes a macOS Intel reusable workflow or waits on a macOS Intel build job -> confirmed by prior T01 workflow removal and preserved by final search with no remaining active workflow references outside plan history
- [x] The repository no longer contains the dedicated macOS Intel release workflow or `macos-intel`-named release jobs/artifact producers for the CLI release flow -> confirmed by final search with no active non-plan matches
- [x] The npm launcher no longer advertises or resolves `darwin/x64` / `x86_64-apple-darwin` as a supported install target -> confirmed by prior T03/T04 changes plus `nix flake check` coverage of npm tests and final search showing no active non-plan matches
- [x] Current-state release/context docs no longer advertise `x86_64-apple-darwin` as a supported automated release target -> confirmed by current context files and final search showing no active current-state doc matches outside plan history
- [x] Validation confirms the release workflow graph, artifact contract references, and context files are internally consistent after the removal -> confirmed by `nix flake check` exit 0 and root/domain context verification during T05 context sync

### Residual risks

- None identified.
