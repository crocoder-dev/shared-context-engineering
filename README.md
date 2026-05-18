# Shared Context Engineering (SCE)

[![GitHub Actions](https://img.shields.io/github/actions/workflow/status/crocoder-dev/shared-context-engineering/publish-tiles.yml?branch=main&label=github%20actions)](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/publish-tiles.yml)
[![crates.io](https://img.shields.io/crates/v/shared-context-engineering?logo=rust)](https://crates.io/crates/shared-context-engineering)
[![npm](https://img.shields.io/npm/v/%40crocoder-dev%2Fsce?logo=npm)](https://www.npmjs.com/package/@crocoder-dev/sce)

Shared Context Engineering is AI-assisted software delivery with explicit, versioned context.

This repository contains the `sce` CLI, generated assistant configuration, and the shared `context/` memory used across SCE workflows.

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
- [Motivation](https://sce.crocoder.dev/docs/motivation)
- [GitHub repository](https://github.com/crocoder-dev/shared-context-engineering)

## Install the `sce` CLI

### Nix

```bash
nix run github:crocoder-dev/shared-context-engineering -- --help
```

To install it into your profile:

```bash
nix profile install github:crocoder-dev/shared-context-engineering
```

### Cargo

Published releases target the `shared-context-engineering` crate and install the `sce` binary.

```bash
cargo install shared-context-engineering --locked
```

Additional supported Cargo install paths:

```bash
cargo install --git https://github.com/crocoder-dev/shared-context-engineering shared-context-engineering --locked
cargo install --path cli --locked
```

### npm

Published npm releases target the `@crocoder-dev/sce` package and install the `sce` launcher.

```bash
npm install -g @crocoder-dev/sce
```

Built by [CroCoder](https://www.crocoder.dev/)
