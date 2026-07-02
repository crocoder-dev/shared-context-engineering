# Flatpak Python Validators to Bash

## Change summary

Rewrite the Flatpak validation helper scripts currently implemented in Python under `nix/flatpak/` as Bash scripts that run inside the repository's Nix-provided environment. The target scripts are:

- `nix/flatpak/static-validate.py`
- `nix/flatpak/version-parity.py`
- `nix/flatpak/local-manifest-validate.py`

The Bash replacements should preserve the current command-line contracts and validation behavior closely enough that existing Flatpak flake checks and Flatpak release/local-manifest orchestration continue to pass without Python-specific helper scripts.

## Success criteria

- All three Python Flatpak validator scripts are replaced by Bash equivalents.
- Existing invocation contracts are preserved:
  - static validation accepts `--repo-root <path>`.
  - version parity validation accepts `--repo-root <path> --version <semver>`.
  - local manifest validation accepts `--repo-root <path> --manifest-path <path>`.
- Failure prefixes and validation messages remain equivalent unless a minor wording change is necessary for Bash/tooling implementation.
- Bash scripts may depend on tools available through the repository Nix environment, such as `jq`, shell text tools, and any explicitly added Nix-provided validator dependency.
- Nix/flake wrappers and release/local Flatpak orchestration call the Bash scripts instead of the removed Python scripts.
- Default validation remains covered by `nix flake check`; Flatpak-specific app or release-package checks that consume these scripts still work.

## Constraints and non-goals

- Do not change Flatpak packaging semantics, asset names, release contract, or source-built policy.
- Do not rewrite generated Flatpak manifest/cargo-source generation in this plan except where references to the validator scripts must be updated.
- Do not introduce host-global dependencies; required tools must be available via the repo's Nix shell/check environment.
- Prefer deterministic output and stable error ordering.
- Keep each implementation task as one atomic commit unit.

## Task stack

- [x] T01: `Port local manifest validator to Bash` (status:done)
  - Task ID: T01
  - Goal: Replace `nix/flatpak/local-manifest-validate.py` with a Bash script that performs the same local-checkout manifest checks.
  - Boundaries (in/out of scope): In - argument parsing for `--repo-root` and `--manifest-path`, path canonicalization equivalent to Python `Path.resolve()`, manifest text checks, stderr failure prefix parity, executable bit/Nix reference updates for this validator. Out - static Flatpak validation, version parity validation, Flatpak packaging behavior changes.
  - Done when: The local manifest validator is Bash-owned, the Python file is removed or no longer referenced, its current success/failure cases are preserved, and all call sites use the Bash entrypoint.
  - Verification notes (commands or checks): Run the narrow Flatpak local-manifest validation command or wrapper that invokes this validator; run `nix flake check` if no narrower check exists.
  - Completed: 2026-06-23
  - Files changed: `nix/flatpak/local-manifest-validate.sh`, `nix/flatpak/local-manifest-validate.py`, `flake.nix`
  - Evidence: `nix run .#sce-flatpak -- prepare-local-manifest --repo-root . --out-dir /tmp/opencode/sce-flatpak-t01` passed and printed `/tmp/opencode/sce-flatpak-t01/dev.crocoder.sce.yml`; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Bash replacement preserves the local manifest validation checks and failure prefix; `flake.nix` now builds `flatpak-local-manifest-check` from the Bash script via `pkgs.writeShellApplication`.

- [x] T02: `Port version parity validator to Bash` (status:done)
  - Task ID: T02
  - Goal: Replace `nix/flatpak/version-parity.py` with a Bash script that validates `.version`, `cli/Cargo.toml`, `npm/package.json`, and Flatpak AppStream release metadata against the requested version.
  - Boundaries (in/out of scope): In - argument parsing for `--repo-root` and `--version`, file-read/parse failure handling, Cargo/npm/AppStream version extraction using Nix-provided tools, parity error prefix/message preservation, Nix/release wrapper updates for this validator. Out - release asset naming changes, version bumping, AppStream content changes beyond tests/fixtures if needed.
  - Done when: The version parity validator is Bash-owned, no Python implementation is invoked, and mismatched versions still produce deterministic `Flatpak release version validation failed: ...` diagnostics.
  - Verification notes (commands or checks): Run the release-version parity wrapper/check that invokes this validator with the current `.version`; run the relevant Flatpak release-package dry/narrow validation if available; include `nix flake check` before handoff if feasible.
  - Completed: 2026-06-23
  - Files changed: `nix/flatpak/version-parity.sh`, `nix/flatpak/version-parity.py`, `flake.nix`
  - Evidence: `nix develop -c bash nix/flatpak/version-parity.sh --repo-root . --version 0.3.0-pre-alpha-v2` passed; `nix run .#flatpak-version-parity-check -- --repo-root . --version 0.3.0-pre-alpha-v2` passed; negative `nix run .#flatpak-version-parity-check -- --repo-root . --version 0.0.0` failed with `Flatpak release version validation failed:` diagnostics; `nix run .#release-flatpak-package -- --version 0.3.0-pre-alpha-v2 --out-dir /tmp/opencode/sce-flatpak-t02` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Bash replacement preserves the release-version parity contract, uses `jq` for npm JSON and `xmllint` for AppStream XML, and `flake.nix` now builds `flatpak-version-parity-check` from the Bash script via `pkgs.writeShellApplication`.

- [x] T03: `Port static Flatpak validator to Bash` (status:done)
  - Task ID: T03
  - Goal: Replace `nix/flatpak/static-validate.py` with a Bash script that preserves the manifest, cargo-sources, and metainfo validations.
  - Boundaries (in/out of scope): In - manifest required-snippet checks, pinned release git-source check, banned artifact-source scan over `packaging/flatpak` files except `sce-flatpak.sh`, `cargo-sources.json` JSON checks, AppStream metainfo ID/provided-binary checks, deterministic stderr diagnostics, Nix check updates for this validator. Out - changing the Flatpak manifest, cargo source generation semantics, or AppStream schema/content except to fix validator integration issues.
  - Done when: Static Flatpak validation is Bash-owned and preserves the existing positive checks and banned-snippet safeguards without invoking Python.
  - Verification notes (commands or checks): Run the static Flatpak validation wrapper/check; run `nix flake check` or the relevant Linux Flatpak check if available.
  - Completed: 2026-06-23
  - Files changed: `nix/flatpak/static-validate.sh`, `nix/flatpak/static-validate.py`, `flake.nix`
  - Evidence: `nix run .#flatpak-static-check -- --repo-root .` passed; `nix build --no-link .#checks.x86_64-linux.flatpak-static-validation` passed; negative check with the Flatpak talk permission removed failed with `Flatpak static validation failed: host Flatpak permission is missing`; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Bash replacement preserves the static manifest, pinned git source, banned artifact-source, cargo-sources, and AppStream metainfo checks; `flake.nix` now builds `flatpak-static-check` from the Bash script via `pkgs.writeShellApplication` with `jq`, `gawk`, and `xmllint` available.

- [x] T04: `Remove Python validator dependency surface and sync context` (status:done)
  - Task ID: T04
  - Goal: Ensure the repository no longer presents these Flatpak validators as Python-owned and update durable context if the implemented tooling contract changes.
  - Boundaries (in/out of scope): In - remove remaining references to the `.py` validators, update Nix packaging/check inputs if not already updated by T01-T03, adjust context files that describe the validators from Python scripts to Bash scripts if current-state docs would otherwise be stale. Out - broad Flatpak architecture rewrites or unrelated context cleanup.
  - Done when: Searchable references to the removed Python validator filenames are gone or intentionally historical, context reflects Bash validator ownership where relevant, and no stale Python-specific dependency remains for this validator surface.
  - Verification notes (commands or checks): Search for `static-validate.py`, `version-parity.py`, and `local-manifest-validate.py`; run `nix run .#pkl-check-generated` if generated/config context changed; run targeted Flatpak checks as appropriate.
  - Completed: 2026-06-23
  - Files changed: `context/glossary.md`, `context/plans/flatpak-python-validators-to-bash.md`
  - Evidence: `rg -n "static-validate\.py|version-parity\.py|local-manifest-validate\.py|Python validator|Python-owned|python-owned" .` reports only historical plan target/evidence references plus this task's note; `rg -n "static-validate\.py|version-parity\.py|local-manifest-validate\.py|writers\.writePython3Bin|python3 - <<'PY'|python3 -" flake.nix nix packaging .github README.md` produced no matches; `nix run .#flatpak-static-check -- --repo-root .` passed; `nix run .#flatpak-version-parity-check -- --repo-root . --version 0.3.0-pre-alpha-v2` passed; `nix run .#pkl-check-generated` passed; `git diff --check` passed.
  - Notes: Remaining literal `.py` validator filename references are intentionally historical plan target/evidence references, including this plan's change summary/T01-T03 evidence and the earlier completed `nix-native-flatpak-release` plan's historical T04 record. Durable current-state context now describes Bash validator ownership and no current Nix/check dependency references the Python validator scripts. Context-sync classification: localized cleanup/current-state wording update; root context edit limited to removing stale Python-validator phrasing from `context/glossary.md`.

- [x] T05: `Validate Flatpak Bash validator migration` (status:done)
  - Task ID: T05
  - Goal: Run final validation and cleanup for the full Python-to-Bash Flatpak validator migration.
  - Boundaries (in/out of scope): In - full repo validation, Flatpak-specific checks/apps that exercise all three Bash validators, generated-output parity check, cleanup of temporary files, final plan status/evidence updates, context sync verification. Out - new feature work or additional Flatpak packaging changes.
  - Done when: Full required checks pass or failures are documented with clear external/blocking cause; temporary scaffolding is removed; plan evidence is recorded; context is confirmed current.
  - Verification notes (commands or checks): Prefer `nix flake check`; run `nix run .#pkl-check-generated`; run any Flatpak-specific wrapper checks needed to exercise the local-manifest, version-parity, and static validators.
  - Completed: 2026-06-23
  - Files changed: `context/plans/flatpak-python-validators-to-bash.md`
  - Evidence: `nix run .#flatpak-static-check -- --repo-root .` passed; `nix run .#flatpak-version-parity-check -- --repo-root . --version 0.3.0-pre-alpha-v2` passed; `nix run .#sce-flatpak -- prepare-local-manifest --repo-root . --out-dir /tmp/opencode/sce-flatpak-t05` passed and printed `/tmp/opencode/sce-flatpak-t05/dev.crocoder.sce.yml`; `/tmp/opencode/sce-flatpak-t05` was removed after the local-manifest check; `nix run .#pkl-check-generated` passed; `nix flake check` passed; `git diff --check` passed.
  - Notes: Final validation covered all three Bash validator entrypoints plus generated-output parity and the full default flake check suite. Context-sync classification: verify-only/current-state confirmation; no additional durable context wording change was needed for this validation-only task.

## Open questions

- None. User clarified that all three Python scripts are in scope, Bash may use tools included in the Nix shell, and behavior should preserve current validation acceptance.

## Validation Report

### Commands run

- `nix run .#flatpak-static-check -- --repo-root .` -> exit 0.
- `nix run .#flatpak-version-parity-check -- --repo-root . --version 0.3.0-pre-alpha-v2` -> exit 0.
- `nix run .#sce-flatpak -- prepare-local-manifest --repo-root . --out-dir /tmp/opencode/sce-flatpak-t05` -> exit 0; printed `/tmp/opencode/sce-flatpak-t05/dev.crocoder.sce.yml`.
- `rm -rf /tmp/opencode/sce-flatpak-t05` -> exit 0; temporary local-manifest output removed.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all checks passed.
- `git diff --check` -> exit 0.

### Success-criteria verification

- [x] All three Python Flatpak validator scripts are replaced by Bash equivalents: confirmed by T01-T04 evidence and current checked-in Bash validators under `nix/flatpak/`.
- [x] Invocation contracts are preserved: static, version-parity, and local-manifest entrypoints were exercised through their Nix app/orchestration wrappers.
- [x] Nix/flake wrappers call Bash scripts: confirmed by targeted app checks and full `nix flake check`.
- [x] Default validation remains covered by `nix flake check`: full flake check passed, including Linux Flatpak checks.
- [x] Context reflects current behavior: current-state context documents Bash-owned Flatpak validators and context sync found no additional drift for this validation-only task.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
