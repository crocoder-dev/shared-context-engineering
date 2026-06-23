# GitHub Release Pre-release Option

## Change summary

Add a manual GitHub Actions `workflow_dispatch` checkbox/input to the `sce` release orchestrator so maintainers can mark a manually-created GitHub Release as a GitHub pre-release. The release command remains thin orchestration: the flag is passed to the GitHub Release creation step as the release-level `prerelease` boolean only.

The requested behavior does not change tag naming, release version validation, generated release notes, title, body copy, artifact names, downstream Cargo/npm publication workflows, or tag-triggered release behavior.

## Success criteria

- Manual runs of `.github/workflows/release-sce.yml` expose a boolean pre-release checkbox/input.
- The GitHub Release creation step passes that manual input to `softprops/action-gh-release` as the release-level `prerelease` value.
- Push/tag-triggered releases preserve current behavior and are not automatically marked pre-release based on tag names such as `-pre`, `-alpha`, `-beta`, or `-rc`.
- Release title/body/notes remain unchanged except for the GitHub Release `prerelease` flag.
- The workflow remains valid under the repository's existing workflow validation surface.
- Current release context documents are updated only where they describe the release orchestrator contract.

## Constraints and non-goals

- In scope: `.github/workflows/release-sce.yml` manual dispatch input and `softprops/action-gh-release` `prerelease` wiring.
- In scope: targeted context sync for release-orchestrator behavior.
- Out of scope: a new `sce-release` CLI command, tag naming changes, semver prerelease parsing, automatic prerelease inference, release-note/body/title changes, GitHub CLI usage, GitHub API code, and registry publish-stage changes.
- Out of scope: changing native, npm, or Flatpak release asset names or artifact assembly logic.
- Constraint: preserve deterministic `.version`/tag/Cargo/npm validation behavior.
- Constraint: keep task slices suitable for one atomic commit each.

## Task stack

- [x] T01: `Add manual GitHub Release prerelease workflow input` (status:done)
  - Task ID: T01
  - Goal: Add a boolean `workflow_dispatch` input to `.github/workflows/release-sce.yml` and wire it into the `Create GitHub release` action as the `prerelease` value.
  - Boundaries (in/out of scope): In - release orchestrator workflow input definition, expression plumbing for manual runs, default false behavior for tag pushes, `softprops/action-gh-release` `prerelease` input. Out - release title/body/notes edits, tag/version parsing changes, reusable platform workflow changes, publish workflows, CLI code.
  - Done when: Manual dispatch includes a pre-release checkbox/input; manual runs with the checkbox selected create/update the GitHub Release with `prerelease: true`; manual runs without it and tag-triggered runs keep `prerelease: false`; release artifact assembly and validation gates remain unchanged.
  - Verification notes (commands or checks): Inspect `.github/workflows/release-sce.yml` for valid `workflow_dispatch.inputs.<name>.type: boolean`, default false, and `softprops/action-gh-release` `prerelease: ${{ ... }}` wiring; run `nix flake check` if implementation scope allows full workflow/actionlint validation.
  - Completed: 2026-06-24
  - Files changed: `.github/workflows/release-sce.yml`
  - Evidence: `nix run nixpkgs#actionlint -- .github/workflows/release-sce.yml` passed; `nix build .#checks.x86_64-linux.workflow-actionlint` passed; `nix flake check` passed.
  - Notes: Added manual-only boolean `prerelease` input with default `false` and wired it to `softprops/action-gh-release`; non-dispatch tag releases resolve the release flag to `false`.

- [x] T02: `Sync release context for prerelease option` (status:done)
  - Task ID: T02
  - Goal: Update current-state release context to document that manual release dispatch can mark the GitHub Release as a pre-release flag without changing tag/version semantics.
  - Boundaries (in/out of scope): In - focused updates to release-related context such as `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-first-install-channels-contract.md`, and context-map/glossary entries only if needed. Out - broad rewrite of release docs, completed-plan summaries, unrelated install-channel changes.
  - Done when: Context accurately states the manual pre-release flag behavior, preserves the distinction that GitHub pre-release is a release flag rather than a tag property, and does not imply auto-inference from tag names.
  - Verification notes (commands or checks): Read changed context files for current-state wording; check that context references align with `.github/workflows/release-sce.yml`; run `nix run .#pkl-check-generated` if generated/context-adjacent changes are touched by the implementation session.
  - Completed: 2026-06-24
  - Files changed: `context/architecture.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-first-install-channels-contract.md`, `context/context-map.md`
  - Evidence: Reviewed `.github/workflows/release-sce.yml` lines 13-17 and 225-230 against release context; `nix run .#pkl-check-generated` passed.
  - Notes: Durable context states the manual checkbox controls only the GitHub Release-level `prerelease` flag; tag/version semantics, generated notes/title/body, and artifact naming remain unchanged with no tag-name inference.

- [ ] T03: `Validate release workflow and cleanup` (status:todo)
  - Task ID: T03
  - Goal: Run final validation for the completed plan and remove any temporary planning or implementation artifacts.
  - Boundaries (in/out of scope): In - repository validation, workflow/actionlint coverage through the existing flake checks, generated-output parity, plan checkbox/status updates, cleanup of temporary files. Out - new feature work beyond the prerelease flag and context sync.
  - Done when: Full required validation passes or any failures are captured with actionable notes; temporary files are removed; the plan reflects completed tasks; context sync has been verified.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; review `git diff` to confirm only intended workflow/context/plan changes remain.

## Open questions

None. The clarification gate resolved that this is a new manual checkbox only, and that the implementation should only set the GitHub Release `prerelease` true/false flag.
