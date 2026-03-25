# SCE CLI First-Wave Install Contract

This file captures the approved first-wave install/distribution contract for the `sce` CLI from `context/plans/sce-cli-first-install-channels.md` task `T01`.

## Canonical naming

- The canonical CLI/binary/package name for this rollout is `sce`.
- Legacy `sce-editor` references are migration debt and should be removed or explicitly mapped when a touched surface still uses the old name.
- New first-wave assets, docs, packaging, and automation should default to `sce` naming.

## Approved first-wave channels for the current implementation stage

- Repo-flake `Nix`
- `Cargo`
- `npm`

`Homebrew` is deferred from the current implementation stage.

No other install channels are in scope for the first wave.

## Channel ownership rules

- `flake.nix` is the first-class Nix run/install surface for this rollout.
- Nix-managed build/release entrypoints are the required build source for first-wave release automation.
- `npm` consumes release artifacts produced by Nix-managed build/release flows.
- `Cargo` is a first-class supported install path.
- `Homebrew` can return in a later plan stage, but it is not part of current code truth.

## Explicitly out of scope in this phase

- AUR
- `.deb`
- `.rpm`
- Flatpak
- AppImage
- Other Linux package-manager specific channels not listed above

## Implementation implications for later tasks

- Release assets must be named and published for `sce`.
- Unsupported channels in older docs should be removed or explicitly deferred rather than implied as active support.
- Later packaging tasks should implement the contract above rather than redefining channel scope per channel.
