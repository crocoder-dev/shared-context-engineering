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
- `linux/arm64` → `aarch64-unknown-linux-gnu`
- `linux/x64` → `x86_64-unknown-linux-gnu`

Unsupported platforms fail with explicit guidance instead of attempting an alternate install channel inside the npm package.

## How the installer works

The npm package is a thin launcher. During `postinstall`, it downloads the signed GitHub Release manifest for the package version, verifies the bundled manifest signature, selects the checksum-pinned native archive for the current supported platform, extracts `bin/sce`, and stores it in package-local runtime storage.

Native binary portability is handled before the GitHub Release archive is published. The npm installer does not rewrite macOS dynamic library install names, run `install_name_tool`, patch Linux ELF metadata, or build a separate npm-native binary.

## Troubleshooting install vs runtime failures

- `postinstall` failures usually indicate npm lifecycle-script execution, network/download, signature, checksum, unsupported-platform, or package-local filesystem permission issues.
- Runtime loader failures after a successful install, such as macOS `dyld` errors mentioning `/nix/store/.../*.dylib`, indicate a bad native release artifact. Fix those in the GitHub Release artifact pipeline before npm consumes the archive; do not add user-side npm workarounds or a separate npm build path.

## Other supported install channels

- Nix: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- Cargo: `cargo install shared-context-engineering --locked`

Built by [CroCoder](https://www.crocoder.dev/)
