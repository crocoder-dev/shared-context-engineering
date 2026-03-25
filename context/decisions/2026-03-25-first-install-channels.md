# Decision: Define the first-wave install contract around `sce`

Date: 2026-03-25
Plan: `context/plans/sce-cli-first-install-channels.md`
Task: `T01`

## Decision

- The approved first-wave install/distribution channels for the active implementation stage are `Nix`, `Cargo`, and `npm`.
- The canonical binary, package, and user-facing product name for this wave is `sce`.
- Legacy `sce-editor` naming must be removed or explicitly mapped during migration; new first-wave surfaces should not introduce fresh `sce-editor` references.
- Nix-managed build/release entrypoints are the source of truth for build and release automation in this rollout.
- npm is a downstream consumer of Nix-produced release artifacts rather than owning a separate build pipeline.
- Homebrew is deferred from the active implementation stage and is not part of current code truth.

## First-wave channel contract

| Channel | Role in first wave | Artifact/build contract |
| --- | --- | --- |
| Repo-flake Nix | First-class run/install path | Uses repo `flake.nix` outputs directly |
| Cargo | Supported source/install path | Supported install guidance and release posture stay aligned with Nix-managed build policy |
| npm | Supported package-manager install path | Consumes released prebuilt artifacts produced by Nix-managed release flow |

This phase does not include `Homebrew`, AUR, `.deb`, `.rpm`, AppImage, Flatpak, or other legacy fallback routes.

## Why this path

- It narrows the supported install story to a coherent first wave instead of promising every previously documented channel.
- It keeps build ownership centralized in Nix-managed entrypoints, reducing per-channel drift.
- It keeps the shipped install story aligned to the currently implemented channel set.

## Consequences for follow-up tasks

- `T02` and later tasks must implement artifacts, packaging, and automation against the canonical `sce` naming.
- `T03` and later tasks must not add new first-wave channels without a new decision.
- `T08` must make the user-facing install documentation and current-state context reflect only the delivered subset of this approved contract.
