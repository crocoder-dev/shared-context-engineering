# Shared Context Engineering npm package (`@crocoder-dev/sce`)

[![npm](https://img.shields.io/npm/v/%40crocoder-dev%2Fsce?logo=npm)](https://www.npmjs.com/package/@crocoder-dev/sce)

Shared Context Engineering is AI-assisted software delivery with explicit, versioned context.

This package publishes the `@crocoder-dev/sce` npm launcher for the `sce` CLI.

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
- [GitHub repository](https://github.com/crocoder-dev/shared-context-engineering)

## Install with npm

Published npm releases target the `@crocoder-dev/sce` package and install the `sce` launcher.

```bash
npm install -g @crocoder-dev/sce
```

## Supported platforms

- `darwin/arm64` → `aarch64-apple-darwin`
- `linux/arm64` → `aarch64-unknown-linux-musl`
- `linux/x64` → `x86_64-unknown-linux-musl`

Unsupported platforms fail with explicit guidance instead of attempting an alternate install channel inside the npm package.

## How the installer works

The npm package is a thin launcher. During `postinstall`, it downloads the signed GitHub Release manifest for the package version, verifies the bundled manifest signature, checks that the signed manifest matches this package (same version, binary `sce`, exactly one canonical `sce-v<version>-<target>.tar.gz` artifact for the current platform with a valid SHA-256 checksum), downloads and checksum-verifies that archive, extracts only `bin/sce` with a bounded built-in reader that rejects unsafe or unexpected archive entries, and atomically installs it with mode `0755` into package-local runtime storage. A failed install leaves any previously installed binary in place.

Native binary portability is handled before the GitHub Release archive is published. The npm installer does not rewrite macOS dynamic library install names, run `install_name_tool`, patch Linux ELF metadata, or build a separate npm-native binary.

## Troubleshooting install vs runtime failures

- `postinstall` failures usually indicate npm lifecycle-script execution, network/download, signature, checksum, unsupported-platform, or package-local filesystem permission issues.
- `Invalid sce release manifest`, `Invalid sce release artifact`, or `No sce release artifact found` errors mean the signed manifest does not describe this package version and platform; `Invalid sce release archive` errors mean the downloaded archive contained unsafe, unexpected, or oversized entries. Both abort the install without replacing an existing binary and indicate a release publication problem, not a local configuration issue.
- Runtime loader failures after a successful install, such as macOS `dyld` errors mentioning `/nix/store/.../*.dylib`, indicate a bad native release artifact. Fix those in the GitHub Release artifact pipeline before npm consumes the archive; do not add user-side npm workarounds or a separate npm build path.

## Other supported install channels

- Nix: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- Cargo: `cargo install shared-context-engineering --locked`

Built by [CroCoder](https://www.crocoder.dev/)
