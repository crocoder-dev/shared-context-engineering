# Plan: Flatpak GitHub Release assets for `sce`

## Change summary

Add GitHub Release Flatpak assets for the existing source-built `dev.crocoder.sce` Flatpak channel.

The release should publish a deterministic Flatpak source-manifest package alongside the existing CLI binary archives, signed native release manifest, and npm tarball. The Flatpak asset is not a prebuilt `sce` binary, not a `.flatpak`/OSTree bundle, and not a Flathub publication workflow. It packages the Flathub-style manifest plus support files needed to build `sce` from source inside Flatpak, with the manifest's git source pinned to the release commit.

Resolved implementation decisions:

- Scope is GitHub Release assets only.
- No automatic Flathub submission or downstream `publish-flatpak.yml` workflow is in scope.
- Flatpak remains source-built and must not consume Nix-built, GitHub Release native, npm-native, or other prebuilt `sce` binaries.
- The Flatpak asset set is separate from the signed native `sce-v<version>-release-manifest.json` consumed by npm.
- Planned artifact names:
  - `sce-v<version>-flatpak-manifest.tar.gz`
  - `sce-v<version>-flatpak-manifest.tar.gz.sha256`
  - `sce-v<version>-flatpak.json`

## Success criteria

- A root-flake release app exists for building Flatpak GitHub Release assets from the checked-in Flatpak packaging source, for example `nix run .#release-flatpak-package -- --version <semver> --out-dir <path>`.
- The app refuses to package when the requested version disagrees with repo-root `.version`, `cli/Cargo.toml`, `npm/package.json`, or the Flatpak AppStream release metadata.
- The Flatpak release tarball is deterministic and contains a top-level `sce-v<version>-flatpak-manifest/` directory with:
  - `dev.crocoder.sce.yml`
  - `dev.crocoder.sce.metainfo.xml`
  - `cargo-sources.json`
  - `git-host-bridge`
- The packaged `dev.crocoder.sce.yml` pins the release git source to the current release commit without mutating the checked-in manifest.
- The app emits a SHA-256 checksum file and JSON metadata describing the Flatpak source asset, app ID, version, release commit, manifest name, package file, checksum file, checksum, and packaged support files.
- `.github/workflows/release-sce.yml` builds and uploads the Flatpak source-manifest assets to the GitHub Release alongside the existing CLI/npm release assets.
- Documentation and durable context clearly state that GitHub Releases now include Flatpak source-manifest assets, while Flatpak remains source-built and Flathub publication remains out of scope.

## Constraints and non-goals

- Do not publish to Flathub.
- Do not add a downstream Flatpak publication workflow.
- Do not build or publish `.flatpak`, OSTree repository, bundle, AppImage, `.deb`, `.rpm`, AUR, or Homebrew assets.
- Do not add Flatpak content to the native signed release manifest used by npm.
- Do not add release-version bumping in workflow code; release packaging must consume checked-in `.version` like the existing release apps.
- Preserve current source-built Flatpak semantics and the release-source-plus-local-checkout override model.
- Keep default `nix flake check` lightweight; do not add full network-heavy Flatpak builds to default checks.
- Each executable task below is scoped as one atomic commit unit by default.

## Task stack

- [x] T01: `Approve Flatpak GitHub Release asset contract in context` (status:done)
  - Task ID: T01
  - Goal: Update current-state context so GitHub Release Flatpak source-manifest assets are in scope while preserving source-built Flatpak semantics.
  - Boundaries (in/out of scope): In - update relevant distribution/release context files such as `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md`; define the Flatpak GitHub Release asset shape, naming, version authority, and no-Flathub boundary. Out - application code, flake implementation, workflow implementation, README/user docs outside durable context.
  - Done when: Current-state context no longer says GitHub Release Flatpak assets are out of scope for the active model; it records that only source-manifest assets are approved, not prebuilt Flatpak binaries/bundles or Flathub automation; existing Nix/Cargo/npm release contracts remain intact.
  - Verification notes (commands or checks): Static review of changed context for contradictions; search touched context for stale current-state wording such as “GitHub Release Flatpak assets are not part of scope” and confirm any remaining matches are historical completed-plan references only.
  - Completed: 2026-06-22
  - Files changed: `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/flatpak-github-release-assets.md`
  - Evidence: static context review completed; `git diff --check -- <touched-context-files>` passed; touched Markdown trailing-whitespace scan passed; stale current-state wording search for Flatpak GitHub Release assets/out-of-scope phrases found only plan/historical references after edits.
  - Context-sync classification: important change; root and domain context updated because the task changes the approved release/distribution contract; context line-count check passed for updated shared/domain files.

- [x] T02: `Add Flatpak release package flake app` (status:done)
  - Task ID: T02
  - Goal: Implement the Nix-owned local release packaging entrypoint that emits deterministic Flatpak source-manifest GitHub Release assets.
  - Boundaries (in/out of scope): In - add a Flatpak release packaging command, likely by extending `packaging/flatpak/sce-flatpak.sh` and exposing it through a new Linux-compatible root-flake app such as `apps.release-flatpak-package`; update app metadata and dev-shell banner if appropriate; stage a generated release manifest that rewrites only the staged copy's git commit to the current release commit; emit tarball, checksum, and JSON metadata. Out - GitHub workflow changes, README/docs updates, Flathub publishing, native binary archive changes, signed native release-manifest changes.
  - Done when: `nix run .#release-flatpak-package -- --version <semver> --out-dir <path>` exists, validates version parity across `.version`, `cli/Cargo.toml`, `npm/package.json`, and Flatpak metainfo release metadata, fails outside a repository checkout when it cannot resolve the release commit, creates deterministic `sce-v<version>-flatpak-manifest.tar.gz`, `sce-v<version>-flatpak-manifest.tar.gz.sha256`, and `sce-v<version>-flatpak.json`, and does not mutate checked-in Flatpak files.
  - Verification notes (commands or checks): Run the new app against the checked-in `.version` into a temporary output directory; inspect the JSON metadata and tarball contents; run `nix run .#flatpak-validate -- --skip-optional-lint`; run targeted syntax checks for touched shell/Nix surfaces; leave full repository validation for T05.
  - Completed: 2026-06-22
  - Files changed: `packaging/flatpak/sce-flatpak.sh`, `flake.nix`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/plans/flatpak-github-release-assets.md`
  - Evidence: `bash -n packaging/flatpak/sce-flatpak.sh` passed; `nix eval .#apps.x86_64-linux.release-flatpak-package.meta.description` returned the new app metadata; `nix run .#release-flatpak-package -- --version 0.2.0 --out-dir <tmp-dir>` emitted the Flatpak tarball, checksum, and JSON metadata; inspected JSON/tarball confirmed expected names, checksum match, packaged files, and staged manifest commit; repeated packaging produced byte-identical tarball/checksum/JSON outputs; `nix run .#flatpak-validate -- --skip-optional-lint` passed; wrong-version packaging failed with all four version-parity diagnostics; a metadata-complete non-git fixture failed with the expected release-commit resolution diagnostic; checked-in Flatpak manifest/support files had no git diff.
  - Context-sync classification: important change; root and Flatpak/release domain context updated for the implemented `release-flatpak-package` app and `sce-flatpak release-package` command; context line-count check passed and `git diff --check -- <touched-files>` passed.

- [x] T03: `Publish Flatpak assets from the CLI release workflow` (status:done)
  - Task ID: T03
  - Goal: Add Flatpak source-manifest asset generation and upload to the existing GitHub Release workflow.
  - Boundaries (in/out of scope): In - update `.github/workflows/release-sce.yml` so the release job invokes `nix run .#release-flatpak-package -- --version <resolved-version> --out-dir dist/flatpak`, mentions the Flatpak source-manifest asset in release notes, and includes `dist/flatpak/*.tar.gz`, `dist/flatpak/*.sha256`, and `dist/flatpak/*.json` in the GitHub Release file list. Out - new reusable platform workflows, downstream publish workflows, Flathub credentials/secrets, native release manifest assembly changes, npm/crates publication workflow changes.
  - Done when: A normal `release-sce.yml` run would publish the Flatpak source-manifest tarball, checksum, and metadata JSON alongside the existing CLI/npm assets for the same resolved version; the release body describes it as source-built Flatpak packaging metadata rather than a prebuilt Flatpak app.
  - Verification notes (commands or checks): Static YAML review for step ordering and file globs; if available, run workflow linting; confirm release notes and `files:` globs include the three Flatpak asset types and do not introduce Flathub publishing.
  - Completed: 2026-06-22
  - Files changed: `.github/workflows/release-sce.yml`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/plans/flatpak-github-release-assets.md`
  - Evidence: Static YAML review confirmed the release job now builds `dist/flatpak` via `nix run .#release-flatpak-package -- --version "${{ needs.resolve-release.outputs.version }}" --out-dir dist/flatpak` before creating the GitHub Release; release notes describe the Flatpak files as source-built packaging metadata and explicitly not a prebuilt Flatpak app or Flathub publication; `files:` globs include `dist/flatpak/*.tar.gz`, `dist/flatpak/*.sha256`, and `dist/flatpak/*.json`; `nix build .#checks.x86_64-linux.workflow-actionlint` passed; `git diff --check -- <touched workflow/context files>` passed. A direct `nix develop -c actionlint .github/workflows/release-sce.yml` check was attempted first but `actionlint` is not exposed in the dev shell.
  - Context-sync classification: important workflow/release behavior change; root and release/distribution domain context updated to reflect `.github/workflows/release-sce.yml` Flatpak source-manifest asset upload wiring while leaving broader README/user-facing documentation to T04.

- [x] T04: `Document Flatpak GitHub Release assets` (status:done)
  - Task ID: T04
  - Goal: Update user-facing docs and durable context to describe the new GitHub Release Flatpak source-manifest asset workflow accurately.
  - Boundaries (in/out of scope): In - update README/release-facing docs and current-state context to explain the Flatpak GitHub Release assets, how they differ from native archives and npm assets, and that Flatpak remains source-built with no Flathub automation. Out - implementation changes to release tooling/workflows, broad marketing copy, new install channels.
  - Done when: Documentation tells users/contributors that GitHub Releases include `sce-v<version>-flatpak-manifest.tar.gz` plus checksum/metadata, the tarball contains the Flathub-style source manifest and support files, and no docs imply a prebuilt Flatpak binary or automated Flathub publication exists.
  - Verification notes (commands or checks): Static docs review; search README/context for stale current-state statements that GitHub Release Flatpak assets are out of scope; confirm remaining historical references are clearly historical or completed-plan evidence.
  - Completed: 2026-06-22
  - Files changed: `README.md`, `context/plans/flatpak-github-release-assets.md`
  - Evidence: README Flatpak section now describes GitHub Release source-manifest assets, lists the tarball/checksum/metadata asset names, names the packaged manifest/support files, and states the assets are not prebuilt Flatpak apps/bundles, OSTree repositories, Flathub submissions, or npm-consumed native release-manifest content; `git diff --check -- README.md` passed; stale exact wording search for active “GitHub Release Flatpak assets are out of scope/not part of scope” statements found only plan verification/history references.
  - Context-sync result: verify-only localized docs update; `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, and release/Flatpak domain context already describe the implemented asset workflow, so no root/domain context edits were needed.

- [x] T05: `Validate release asset flow and clean up` (status:done)
  - Task ID: T05
  - Goal: Run final validation, remove temporary packaging outputs, and ensure durable context matches the implemented Flatpak GitHub Release asset flow.
  - Boundaries (in/out of scope): In - full repository validation where practical, targeted Flatpak release app validation, Flatpak validation command evidence, workflow/docs/context consistency checks, cleanup of temporary release output directories, and final plan evidence. Out - publishing an actual GitHub Release, running Flathub submission, completing a full network-heavy Flatpak build unless explicitly needed and practical.
  - Done when: Required checks pass or any blockers are documented with actionable follow-up; generated Flatpak release assets are proven to have the expected names/content/metadata; no temporary release packaging artifacts remain in the repository; context accurately describes the final current state.
  - Verification notes (commands or checks): `nix run .#release-flatpak-package -- --version "$(tr -d '\n' < .version)" --out-dir <tmp-dir>`; inspect the emitted tarball/checksum/JSON; `nix run .#flatpak-validate -- --skip-optional-lint`; `nix run .#pkl-check-generated`; `nix flake check`; static search for stale Flatpak release-asset scope wording; cleanup temporary output directories.
  - Completed: 2026-06-22
  - Evidence:
    - `nix run .#release-flatpak-package -- --version 0.2.0 --out-dir <tmp-dir>`: emitted `sce-v0.2.0-flatpak-manifest.tar.gz` (37961 bytes), `sce-v0.2.0-flatpak-manifest.tar.gz.sha256`, and `sce-v0.2.0-flatpak.json` with expected fields (app_id, version, release_commit, manifest_name, checksum_sha256, packaged_files). Tarball contains `sce-v0.2.0-flatpak-manifest/` directory with `dev.crocoder.sce.yml`, `dev.crocoder.sce.metainfo.xml`, `cargo-sources.json`, and `git-host-bridge`. SHA-256 checksum `efb3e869fe69c452f91ce37f6c328ab718faf4666c7808644f22fdb8262766bd` verified against tarball.
    - `nix run .#flatpak-validate -- --skip-optional-lint`: passed (`✔ Validation was successful.`)
    - `nix run .#pkl-check-generated`: passed (`Generated outputs are up to date.`)
    - `nix flake check`: all checks passed (92/92 CLI tests, clippy, fmt, flatpak-static-validation, pkl-parity, workflow-actionlint, all npm/config-lib JS checks, integrations-install checks)
    - Static stale-scope wording search: clean; all Flatpak "out of scope" / "not a prebuilt" matches in current-state context are correct contract-boundary descriptions; remaining matches are historical/completed-plan references only.
    - Temporary output directory cleaned up.
  - Context-sync classification: verify-only; no root context edits expected since all context files already describe the implemented current state.

## Open questions

- None. The user clarified that this plan should cover GitHub Release Flatpak assets only, not Flathub publication automation.
