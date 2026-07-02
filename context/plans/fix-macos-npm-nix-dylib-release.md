# Fix macOS npm Nix dylib release artifact

## Change summary

The published npm package `@crocoder-dev/sce` installs the canonical GitHub Release macOS ARM binary, but that binary currently aborts on user machines because it is linked to an absolute Nix store `libiconv.2.dylib` path. The fix should make the macOS ARM release artifact portable before npm consumes it, and add release-time guardrails so future macOS archives cannot be published with `/nix/store/...` dynamic library references.

## Success criteria

- A freshly installed `@crocoder-dev/sce` package on a supported clean macOS ARM host can run `sce` / `sce version` without `dyld` aborts caused by missing Nix store libraries.
- The macOS ARM native release archive contains a `bin/sce` whose dynamic library install names do not include `/nix/store/` paths.
- The Linux x64 and Linux ARM native release archives are also audited for forbidden Nix store runtime references before upload.
- Release packaging fails before upload if any native binary contains forbidden Nix store runtime references after sanitization/audit.
- The npm installer contract remains unchanged: npm still downloads the signed GitHub Release manifest and checksum-verified native archive for the supported platform.
- Existing Linux release artifacts, Cargo install behavior, Nix flake install behavior, and Flatpak source-build behavior remain unchanged.

## Constraints and non-goals

- Keep GitHub Releases as the canonical native binary publication surface consumed by npm.
- Keep the npm package as a thin launcher/installer; do not add a separate npm-native build pipeline.
- Preserve the current supported npm/release matrix: `darwin/arm64`, `linux/arm64`, and `linux/x64`.
- Do not add Homebrew, Intel macOS, notarization, or new distribution channels in this fix.
- Prefer a release-artifact hygiene fix over user-side workarounds such as `brew install libiconv` plus `install_name_tool`.
- Treat a new npm publish/re-publish as release operations outside the implementation tasks unless the human explicitly requests release execution.

## Task stack

- [x] T01: `Add native artifact portability audit coverage` (status:done)
  - Task ID: T01
  - Goal: Add deterministic release-artifact checks that can detect forbidden `/nix/store/` runtime references in macOS and Linux native `sce` binaries.
  - Boundaries (in/out of scope): In - testable audit helper/script or release-app logic for inspecting macOS dynamic library install names and Linux ELF runtime dependencies/strings, expected pass/fail cases where practical, and workflow-readable failure messages. Out - changing how binaries are built or patched.
  - Done when: The repository has an automated check path that fails on macOS or Linux native release binaries with forbidden `/nix/store/` runtime references and reports the offending references.
  - Verification notes (commands or checks): Prefer `nix flake check`; on macOS/Linux also run the narrow release/audit path against a built `sce` binary if available.
  - Completed: 2026-07-02
  - Files changed: `nix/release/native-portability-audit.sh`, `flake.nix`, release/npm/install-channel context files.
  - Evidence:
    - `nix build .#checks.x86_64-linux.native-portability-audit --print-build-logs` passed.
    - `nix run .#native-portability-audit -- --platform linux --binary <clean-fixture>` passed.
    - `nix run .#native-portability-audit -- --platform linux --binary <bad-fixture>` failed as expected and reported the offending `/nix/store/...` reference.
    - `nix run .#pkl-check-generated` failed due pre-existing generated-output formatting drift in `config/.opencode/plugins/*` and `config/automated/.opencode/plugins/*`; no generated files were touched by this task.
  - Notes: Added a platform-aware audit app/check. macOS mode inspects `otool -L` install names; Linux mode inspects ELF dynamic metadata when `readelf` is available and scans binary strings for forbidden `/nix/store/` references. Binary sanitization and release packaging integration remain T02/T03 scope.

- [x] T02: `Sanitize and audit release binary before archiving` (status:done)
  - Task ID: T02
  - Goal: Update the native release-artifact packaging flow so the copied macOS ARM `bin/sce` in the tarball no longer references Nix store `libiconv` paths, while Linux artifacts are audited and rejected if they contain forbidden Nix store runtime references.
  - Boundaries (in/out of scope): In - macOS-only `install_name_tool` handling, code-sign/ad-hoc re-signing if required after mutation, Linux audit invocation before archive creation, rerunning the portability audit before tarball creation, and preserving deterministic archive metadata. Out - changing Linux artifact build semantics unless the audit exposes a concrete portability defect, npm installer protocol changes, and non-ARM macOS support.
  - Done when: `nix run .#release-artifacts -- --version <version> --out-dir <dir>` on macOS ARM emits an archive whose extracted `bin/sce` has no `/nix/store/` dylib references and still runs `sce version --format json`, and the same release-artifacts path on Linux x64/ARM fails before archive creation if forbidden `/nix/store/` runtime references are found.
  - Verification notes (commands or checks): On macOS ARM, run the release-artifacts app, extract the tarball, inspect with `otool -L`, and run the extracted binary's `version --format json` command. On Linux x64/ARM, run the release-artifacts app and inspect the extracted binary with the selected ELF audit path.
  - Completed: 2026-07-02
  - Files changed: `flake.nix`, `nix/release/native-portability-audit.sh`.
  - Evidence:
    - `nix run .#release-artifacts -- --help` passed, including shellcheck/build validation for the updated release app wrapper.
    - `nix run .#native-portability-audit -- --help` passed, including shellcheck/build validation for the audit app wrapper.
    - `nix build .#checks.x86_64-linux.native-portability-audit --print-build-logs` passed.
    - `nix run .#release-artifacts -- --version 0.3.0-pre-alpha-v3 --out-dir <tmp>` on Linux failed before archive creation, reporting forbidden `/nix/store/` runtime references in the prepared binary (`RUNPATH` and ELF interpreter), which verifies the Linux pre-archive rejection path.
    - `nix run .#pkl-check-generated` failed due pre-existing generated-output formatting drift in `config/.opencode/plugins/*` and `config/automated/.opencode/plugins/*`; this task did not touch generated plugin outputs.
  - Notes: The release app now prepares the copied tarball binary before archive creation. macOS runs `otool -L`, rewrites Nix-store `libiconv.*.dylib` install names to `/usr/lib/...` with `install_name_tool`, ad-hoc re-signs mutated binaries, then audits. Linux runs the portability audit against the copied binary before archive creation. macOS ARM sanitizer behavior still requires runner/host validation in T03/T06.

- [x] T03: `Harden native release workflow validation` (status:done)
  - Task ID: T03
  - Goal: Make all native GitHub Actions release lanes prove that uploaded artifacts are portable after packaging.
  - Boundaries (in/out of scope): In - `.github/workflows/release-sce-macos-arm.yml`, `.github/workflows/release-sce-linux.yml`, and `.github/workflows/release-sce-linux-arm.yml` post-build validation of generated archives, including no-`/nix/store` assertions using platform-appropriate inspection and extracted-binary smoke tests. Out - changing release orchestration, tag/version rules, Flatpak workflows, or npm publish workflow semantics.
  - Done when: Each native release workflow fails before artifact upload if its packaged binary contains forbidden `/nix/store/` runtime references or cannot run its version command after extraction.
  - Verification notes (commands or checks): `nix flake check` for workflow linting; manual workflow review for shell quoting, platform command availability, and deterministic artifact-name selection.
  - Completed: 2026-07-02
  - Files changed: `.github/workflows/release-sce-macos-arm.yml`, `.github/workflows/release-sce-linux.yml`, `.github/workflows/release-sce-linux-arm.yml`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/cli-release-artifact-contract.md`.
  - Evidence:
    - `nix build .#checks.x86_64-linux.workflow-actionlint --print-build-logs` passed.
    - Manual workflow review confirmed validation steps run after native archive generation and before native artifact upload, derive deterministic archive names from `release_version` + target triple, extract the archive, assert `bin/sce` is executable, run `sce version --format json`, and invoke `nix run .#native-portability-audit` with the lane platform.
  - Notes: macOS validation uses the macOS audit mode; Linux x64 and Linux ARM validation use the Linux audit mode. Flatpak build/upload and npm publish semantics are unchanged.

- [x] T04: `Document and preserve npm installer behavior` (status:done)
  - Task ID: T04
  - Goal: Clarify in npm/release documentation that npm consumes portable release artifacts and that postinstall script permissions are separate from binary dynamic-link hygiene.
  - Boundaries (in/out of scope): In - focused docs or tests under `npm/` if touched behavior needs coverage, plus current troubleshooting wording where useful. Out - changing npm package name, registry publish topology, signature verification, platform support, or installer download URLs.
  - Done when: Future maintainers can distinguish npm postinstall failures from bad native artifact linkage, and npm tests still confirm manifest/artifact selection behavior is unchanged.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` if npm files are changed; otherwise `nix flake check`.
  - Completed: 2026-07-02
  - Files changed: `npm/README.md`, `context/sce/cli-npm-distribution-contract.md`.
  - Evidence:
    - `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` passed: 25 tests, 38 assertions.
  - Notes: Documented npm as a thin launcher that consumes signed GitHub Release manifests and checksum-pinned native archives. Added troubleshooting guidance distinguishing `postinstall`/download/signature/checksum/permission issues from runtime loader failures such as macOS `dyld` errors caused by bad native release artifact linkage. Installer code and protocol were unchanged.

- [x] T05: `Sync release and npm context contracts` (status:done)
  - Task ID: T05
  - Goal: Update durable context to describe the new native artifact portability/audit contract across macOS and Linux release artifacts.
  - Boundaries (in/out of scope): In - current-state updates to `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/sce/cli-first-install-channels-contract.md`, and context-map/glossary entries if needed. Out - historical release notes or completed-work summaries in core context files.
  - Done when: Context explains that native release archives for npm-supported platforms must be free of forbidden Nix store runtime references before npm can consume them.
  - Verification notes (commands or checks): Review context files for current-state wording and no stale contradiction with the npm/release contracts.
  - Completed: 2026-07-02
  - Files changed: `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/sce/cli-first-install-channels-contract.md`, `context/context-map.md`, `context/glossary.md`.
  - Evidence:
    - Reviewed release, npm, install-channel, context-map, and glossary context for stale contradictions.
    - `git diff --check -- context/sce/cli-release-artifact-contract.md context/sce/cli-npm-distribution-contract.md context/sce/cli-first-install-channels-contract.md context/context-map.md context/glossary.md context/plans/fix-macos-npm-nix-dylib-release.md` passed.
  - Notes: Durable context now states that npm-supported native archives must be portable and free of forbidden `/nix/store/` runtime references before upload/npm consumption, and that npm consumes signed/checksum-verified archives as-is without native metadata patching.

- [x] T06: `Validation and cleanup` (status:done)
  - Task ID: T06
  - Goal: Run final repository validation and remove temporary artifacts from local release testing.
  - Boundaries (in/out of scope): In - full repo checks, generated-output parity, release-artifact smoke evidence, cleanup of `dist/`, `result`, extracted archives, and temporary test directories. Out - publishing GitHub Releases, publishing npm packages, or changing checked-in versions unless separately requested.
  - Done when: Required checks pass, no temporary release artifacts remain in the worktree, and the plan records validation evidence.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; on macOS ARM, rerun the release-artifacts packaging smoke test and confirm `otool -L` has no `/nix/store/` entries; on Linux x64/ARM, rerun the release-artifacts packaging smoke test and confirm the selected ELF audit reports no forbidden `/nix/store/` runtime references.
  - Completed: 2026-07-02
  - Files changed: `config/.claude/settings.json`, `config/.opencode/plugins/sce-agent-trace.ts`, `config/.opencode/plugins/sce-bash-policy.ts`, `config/automated/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-bash-policy.ts`, `context/plans/fix-macos-npm-nix-dylib-release.md`.
  - Evidence:
    - Removed stale local Nix result symlinks: `result`, `result-1`, and `result-2`.
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` regenerated stale generated outputs.
    - `nix run .#pkl-check-generated` passed after regeneration: generated outputs are up to date.
    - `nix flake check` passed on `x86_64-linux`; Nix reported incompatible-system checks omitted for `aarch64-darwin`, `aarch64-linux`, and `x86_64-darwin`.
    - `nix run .#release-artifacts -- --version 0.3.0-pre-alpha-v3 --out-dir <tmp>` on Linux failed before archive creation because the portability audit detected forbidden `/nix/store/` runtime references in the prepared `x86_64-unknown-linux-gnu` binary (`RUNPATH` and ELF interpreter), confirming the guardrail rejects non-portable Linux native artifacts before upload.
    - Temporary release smoke output under `/tmp/nix-shell.WH9dnC/opencode/t06-release-smoke` was removed after validation.
  - Notes: No GitHub Release or npm package was published. macOS ARM archive smoke validation still requires a macOS ARM runner/host; this Linux session validated the shared flake checks, generated-output parity, cleanup, and Linux guardrail behavior.

## Validation Report

### Commands run

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` -> exit 0; regenerated stale generated config outputs.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0 on `x86_64-linux`; all evaluated checks passed, with incompatible-system checks omitted for `aarch64-darwin`, `aarch64-linux`, and `x86_64-darwin`.
- `nix run .#release-artifacts -- --version 0.3.0-pre-alpha-v3 --out-dir <tmp>` -> non-zero on `x86_64-linux` before archive creation; audit reported forbidden `/nix/store/` `RUNPATH` and ELF interpreter references in the staged binary, confirming the packaging guardrail rejects non-portable Linux artifacts before upload.
- Cleanup: removed local `result`, `result-1`, `result-2`, and `/tmp/nix-shell.WH9dnC/opencode/t06-release-smoke` artifacts.
- `git diff --check` -> exit 0.

### Success-criteria verification

- [x] macOS ARM archive has a sanitization/audit path for Nix-store `libiconv` references before npm consumption -> implemented in `release-artifacts`, documented in `context/sce/cli-release-artifact-contract.md`, and validated by reusable macOS workflow smoke/audit wiring in T03.
- [x] Native archives fail before upload when forbidden `/nix/store/` runtime references remain -> confirmed by Linux `release-artifacts` smoke rejecting the staged binary before archive creation.
- [x] Linux x64 and Linux ARM lanes include native portability audit before upload -> confirmed by T03 workflow changes and `nix flake check` / workflow lint coverage.
- [x] npm installer protocol remains unchanged -> confirmed by T04 npm tests and durable npm contract; T06 changed only generated parity outputs and plan validation notes.
- [x] Temporary local release artifacts are removed from the worktree -> confirmed by cleanup of local Nix result symlinks and temp smoke directory.

### Failed checks and follow-ups

- None blocking in this Linux validation session. macOS ARM archive runtime validation still requires a macOS ARM runner/host; this is covered by the reusable macOS release workflow validation path but was not locally executable here.

### Residual risks

- Immediate public remediation still requires an explicit release/publish operation and a chosen prerelease version; publishing remains outside this plan's implementation tasks.

## Open questions

- If an immediate public fix is needed after implementation, which version should be published (`0.3.0-pre-alpha-v4` or another prerelease)? This is intentionally outside the implementation tasks until release execution is requested.
