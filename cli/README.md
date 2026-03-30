# sce

Shared Context Engineering CLI.

## Install with Cargo

Published releases target the `sce` crate name.

Repo-root `.version` is the canonical checked-in release version source for the CLI rollout; Cargo publication publishes the already-versioned checked-in crate metadata rather than bumping versions during the publish workflow.

### crates.io

```bash
cargo install sce
```

### Git repository

```bash
cargo install --git https://github.com/crocoder-dev/shared-context-engineering sce --locked
```

### Local checkout

```bash
cargo install --path cli --locked
```

## Other supported install channels

- Repo flake: `nix run github:crocoder-dev/shared-context-engineering -- --help`
- npm package: `npm install -g sce`

GitHub Releases are the canonical publication surface for signed release archives and manifest/checksum assets; Cargo and npm registry publication are separate downstream publish stages.

Current repository slug for git-based installation and release references: `crocoder-dev/shared-context-engineering`.

See the repository root and Shared Context docs for the broader first-wave install contract.
