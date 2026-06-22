# Plan: Flatpak source-built `.flatpak` bundle release assets

## Change summary

Add source-built `.flatpak` bundle assets to the existing GitHub Release for the `dev.crocoder.sce` Flatpak channel.

Currently the release publishes source-manifest packaging metadata (tarball + checksum + JSON). This plan adds a pre-built `.flatpak` bundle so users can install directly from the GitHub Release without cloning the repository or submitting to Flathub:

```bash
flatpak install --user \
  https://github.com/crocoder-dev/shared-context-engineering/releases/download/v0.2.0/sce-v0.2.0-x86_64.flatpak
```

The bundle is built from source inside Flatpak (using `flatpak-builder` + `flatpak build-bundle`), not from a Nix-built or pre-compiled binary. The existing source-manifest tarball assets are preserved alongside the new bundle assets.

## Success criteria

- A `release-bundle` command exists in `packaging/flatpak/sce-flatpak.sh` that builds the Flatpak from source and emits a `.flatpak` bundle, SHA-256 checksum, and JSON metadata.
- The command validates version parity (`.version`, `cli/Cargo.toml`, `npm/package.json`, Flatpak AppStream) and fails fast on mismatch.
- The `.flatpak` bundle is deterministic for the same source checkout (given the same SDK version).
- Published artifact names:
  - `sce-v<version>-x86_64.flatpak` + `.sha256` + `.json`
  - `sce-v<version>-aarch64.flatpak` + `.sha256` + `.json`
- `.github/workflows/release-sce-linux.yml` builds and uploads the x86_64 Flatpak bundle.
- `.github/workflows/release-sce-linux-arm.yml` builds and uploads the aarch64 Flatpak bundle.
- The main `release-sce.yml` assemble step includes the `.flatpak` / `.sha256` / `.json` bundle assets in the GitHub Release file glob.
- Release notes describe the `.flatpak` bundle as a source-built Flatpak app for direct install, not a prebuilt binary or Flathub submission.
- Default `nix flake check` remains lightweight; full `flatpak-builder` network-heavy builds happen only in the release workflow, not in default checks.

## Constraints and non-goals

- Do not submit to Flathub.
- Do not add a Flathub publication workflow.
- Do not replace the existing source-manifest release assets.
- Do not publish prebuilt (non-Flatpak-compiled) binary bundles; the `.flatpak` must build `sce` from Rust source inside Flatpak.
- Do not add Homebrew, `.deb`, `.rpm`, AppImage, or other platform bundles.
- Keep the lightweight `flatpak-static-validation` check as the only default-flake Flatpak validation.
- Do not add release-version bumping in workflow code; consume `.version` like existing release apps.

## Task stack

- [x] T01: `Approve .flatpak bundle release asset contract in context` (status:done)
  - Task ID: T01
  - Goal: Update current-state context to approve source-built `.flatpak` bundle release assets for both x86_64 and aarch64 Linux, alongside existing source-manifest tarball assets.
  - Boundaries (in/out of scope): In - update `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md` to describe the new `.flatpak` bundle assets and their relationship to existing source-manifest assets. Out - any implementation code, workflow changes, README/docs outside durable context.
  - Done when: Current-state context no longer says "prebuilt Flatpak binaries/bundles are out of scope" for the active model (only Flathub submission + prebuilt non-source binaries remain out of scope); the new `.flatpak` bundle asset naming and architecture matrix are documented; existing source-manifest asset contracts are preserved.
  - Verification notes (commands or checks): Static review of changed context for contradictions; search for stale wording such as "prebuilt Flatpak binaries/bundles remain out of scope" and confirm updated to allow source-built bundles; confirm existing source-manifest contract references are intact.
  - **Status:** done
  - **Completed:** 2026-06-22
  - **Files changed:**
    - `context/sce/cli-first-install-channels-contract.md` — added "Flatpak GitHub Release bundle assets" section; updated ownership rules to include bundle assets
    - `context/sce/cli-release-artifact-contract.md` — added "Implemented Flatpak bundle artifact set" section; updated source-manifest and downstream channel lines
    - `context/architecture.md` — added bundle asset line; updated prebuilt wording to "(non-source-built)"
    - `context/patterns.md` — added bundle asset release pattern; updated prebuilt wording and GitHub Releases surface line
    - `context/glossary.md` — added "Flatpak bundle GitHub Release asset" glossary entry
    - `context/overview.md` — updated Flatpak release contract to include bundle assets with qualified exclusion
    - `context/context-map.md` — updated entries for both Flatpak contract files to mention bundle assets
  - **Evidence:**
    - Static stale-wording review: no unqualified "prebuilt Flatpak binaries/bundles" remains in current-state context (only qualified "prebuilt (non-source-built)" or source-manifest-specific statements)
    - `nix run .#pkl-check-generated` → exit 0, generated outputs up to date
    - `nix flake check` → exit 0, all checks passed
    - Existing source-manifest contract references are intact and unchanged

- [x] T02: `Add release-bundle command to sce-flatpak.sh` (status:done)
  - Task ID: T02
  - Goal: Implement a `sce-flatpak release-bundle` command that builds the `dev.crocoder.sce` Flatpak from source and emits a `.flatpak` bundle, SHA-256 checksum, and JSON metadata for the release.
  - Boundaries (in/out of scope): In - add `release-bundle` subcommand to `packaging/flatpak/sce-flatpak.sh` with `--repo-root`, `--version`, `--arch`, `--out-dir` flags; reuse existing `validate_release_version_parity`, `run_static_checks`, and `generate_local_manifest` functions; run `flatpak-builder` (without `--install`) then `flatpak build-bundle` to produce the `.flatpak` file; run `sha256sum` and render JSON metadata with `asset_type: flatpak-bundle`, architecture field, and app ID. Out - changes to existing `release-package` command, Flathub publication, GitHub workflow changes, flake app wrappers, default check changes.
  - Done when: `sce-flatpak release-bundle --version <semver> --arch x86_64 --out-dir <path>` builds the Flatpak from source in `<build-dir>`, emits `<out-dir>/sce-v<version>-x86_64.flatpak` and matching `.sha256` + `.json` files, fails on version parity mismatch, and does not install to the host system. Same for `--arch aarch64`.
  - Verification notes (commands or checks): Run against the current checkout with a fast incremental build; verify `.flatpak` file is a valid Flatpak bundle (`file` command or `flatpak info` on the bundle); verify JSON metadata contains expected fields; verify SHA-256 matches; verify wrong-version fails with diagnostics; verify `--arch` defaults to host arch when omitted.
  - **Status:** done
  - **Completed:** 2026-06-22
  - **Files changed:**
    - `packaging/flatpak/sce-flatpak.sh` — added `release-bundle` command, `--arch` flag, `cmd_release_bundle()` function, wired into `main()` dispatcher
  - **Evidence:**
    - `bash -n packaging/flatpak/sce-flatpak.sh` → exit 0, syntax OK
    - `--help` output shows `release-bundle` command with `--version`, `--arch`, `--out-dir`, `--repo-root` flags
    - Missing required flags prints usage and exits 1
    - `nix run .#pkl-check-generated` → exit 0, generated outputs up to date
    - `nix build '.#checks.x86_64-linux.flatpak-static-validation'` → exit 0, existing Flatpak validation passes

- [ ] T03: `Build and upload .flatpak bundles from Linux release workflows` (status:todo)
  - Task ID: T03
  - Goal: Add steps to `release-sce-linux.yml` (x86_64) and `release-sce-linux-arm.yml` (aarch64) to build and upload the source-built `.flatpak` bundle alongside existing native CLI artifacts.
  - Boundaries (in/out of scope): In - add a step in each Linux reusable workflow to run `sce-flatpak release-bundle` against the checked-out release commit, uploading the `.flatpak` / `.sha256` / `.json` files as workflow artifacts. In main `release-sce.yml`: add bundle artifacts to the `files:` glob and update release notes body. Out - macOS Flatpak bundles, new workflow files, Flathub publishing workflow, release-version bumping.
  - Done when: The x86_64 Linux reusable workflow uploads `sce-v<version>-x86_64.flatpak` + `.sha256` + `.json` as a named artifact. The aarch64 Linux reusable workflow uploads `sce-v<version>-aarch64.flatpak` + `.sha256` + `.json` as a named artifact. The `release` job downloads both bundle artifacts and includes `dist/flatpak-bundle/*.flatpak`, `dist/flatpak-bundle/*.sha256`, `dist/flatpak-bundle/*.json` in the GitHub Release file list. Release notes describe the bundle as a source-built Flatpak app (not prebuilt binary).
  - Verification notes (commands or checks): Static YAML review for step order and artifact naming; `workflow-actionlint` check on changed workflow files; confirm release notes body and file globs are correct.

- [ ] T04: `Document .flatpak bundle release assets` (status:todo)
  - Task ID: T04
  - Goal: Update user-facing documentation and durable context to describe the new `.flatpak` bundle release assets and the single-command install flow.
  - Boundaries (in/out of scope): In - update `README.md` Flatpak section to describe the new `.flatpak` bundle assets and the `flatpak install --user <url>` install command; update durable context files if not already covered in T01. Out - implementation changes to release tooling/workflows, new install channels, Flathub documentation.
  - Done when: README tells users they can install `sce` from GitHub Release `.flatpak` bundles with `flatpak install --user <url>`, lists the asset names, and states the bundles are source-built (not prebuilt binaries) and not Flathub submissions.
  - Verification notes (commands or checks): Static docs review; search for stale "no .flatpak bundle" wording in current-state docs; confirm new install command is documented accurately.

- [ ] T05: `Validate bundle release flow and clean up` (status:todo)
  - Task ID: T05
  - Goal: Run final validation, verify `.flatpak` bundle assets are correct, check consistency across the repo, and finalize plan evidence.
  - Boundaries (in/out of scope): In - run `sce-flatpak release-bundle` locally to verify output, inspect bundle file/checksum/JSON, run `pkl-check-generated`, run `nix flake check`, static review, cleanup temp directories, update plan evidence. Out - publishing an actual GitHub Release, running Flathub submission, full network-heavy Flatpak build unless explicitly needed.
  - Done when: Required checks pass; `.flatpak` bundle is proven to have expected name/content/metadata; no temporary artifacts remain; context is current.
  - Verification notes (commands or checks): See validation task in existing plan pattern: run release-bundle locally, inspect outputs, `nix flake check`, `pkl-check-generated`, static stale-wording check, cleanup.

## Open questions

- None. Architecture (x86_64 + aarch64), asset set (keep source-manifest + add bundles), and CI approach (in release workflow) are resolved.
