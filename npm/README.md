# sce

Thin npm launcher package for the `sce` CLI.

Published from the `crocoder-dev/shared-context-engineering` repository.

## Install

```bash
npm install -g sce
```

## Release flow

On install, this package downloads the matching platform release artifact for the
current `sce` version from GitHub Releases, verifies the published SHA-256
checksum, and installs the native `sce` binary for local execution.

Repo-root `.version` is the canonical checked-in release version source. GitHub
Releases publish the canonical signed release archives and manifest/checksum
assets first; npm registry publication is a separate downstream publish stage
for the already-versioned checked-in package and does not auto-bump the package
version during publish.
