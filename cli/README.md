# Shared Context Engineering CLI (`sce`)

[![crates.io](https://img.shields.io/crates/v/shared-context-engineering?logo=rust)](https://crates.io/crates/shared-context-engineering)
[![docs.rs](https://img.shields.io/docsrs/shared-context-engineering?logo=docs.rs)](https://docs.rs/shared-context-engineering)

Shared Context Engineering is AI-assisted software delivery with explicit, versioned context.

This crate publishes the `sce` CLI for Shared Context Engineering workflows.

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
- [GitHub repository](https://github.com/crocoder-dev/shared-context-engineering)

## Install with Cargo

Published Cargo releases target the `shared-context-engineering` crate and install the `sce` binary.

### crates.io

```bash
cargo install shared-context-engineering --locked
```

### Git repository

```bash
cargo install --git https://github.com/crocoder-dev/shared-context-engineering shared-context-engineering --locked
```

### Local checkout

```bash
cargo install --path cli --locked
```

## Other supported install channels

- Nix: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- npm: `npm install -g @crocoder-dev/sce`

Built by [CroCoder](https://www.crocoder.dev/)
