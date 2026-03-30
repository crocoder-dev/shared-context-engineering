# sce

Thin npm launcher package for the `sce` CLI.

Published from the `crocoder-dev/shared-context-engineering` repository.

## Install

```bash
npm install -g sce
```

Supported npm install targets:

- `darwin/arm64` → `aarch64-apple-darwin`
- `darwin/x64` → `x86_64-apple-darwin`
- `linux/arm64` → `aarch64-unknown-linux-gnu`
- `linux/x64` → `x86_64-unknown-linux-gnu`

## Release flow

On install, this package downloads the matching platform release artifact for the
current `sce` version from GitHub Releases, verifies the published SHA-256
checksum, and installs the native `sce` binary for local execution. Linux ARM
is an officially supported npm install target via `linux/arm64` mapping to the
GitHub release artifact for `aarch64-unknown-linux-gnu`.

Repo-root `.version` is the canonical checked-in release version source. GitHub
Releases publish the canonical signed release archives and manifest/checksum
assets first; npm registry publication is a separate downstream publish stage
for the already-versioned checked-in package and does not auto-bump the package
version during publish.
