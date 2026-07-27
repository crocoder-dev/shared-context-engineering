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

### Local checkout

Repository source builds require Pkl-generated assets to be prepared before
Cargo starts. From the repository root, use the repository-owned wrapper rather
than invoking Cargo directly:

```bash
./scripts/run-cli-cargo.sh install --path cli --locked
```

The wrapper creates a fresh temporary payload from the canonical Pkl inputs,
passes it to the build through `SCE_CLI_GENERATED_INPUT_DIR`, and removes it when
Cargo exits. Direct `cargo install --git` is not supported because a Git install
has no pre-Cargo generation boundary; use crates.io or a local checkout instead.

## Develop from a local checkout

Use the same wrapper for supported Cargo development workflows:

```bash
./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml
./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup
./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings
```

The wrapper requires `pkl`, `cargo`, and `sha256sum` on `PATH`. The repository
Nix dev shell provides them when they are not installed by the host system.

## Other supported install channels

- Nix: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- npm: `npm install -g @crocoder-dev/sce`

Built by [CroCoder](https://www.crocoder.dev/)
