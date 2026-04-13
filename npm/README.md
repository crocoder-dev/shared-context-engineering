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

## Other supported install channels

- Nix: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- Cargo: `cargo install shared-context-engineering --locked`

Built by [CroCoder](https://www.crocoder.dev/)
