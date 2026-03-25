# Shared Context Engineering

AI-assisted software delivery with explicit, versioned context.

---

## Run and install the CLI

Use the [WIP] `sce` CLI at `cli/`. See `cli/README.md` for current behavior and usage.

Current first-wave install channels:

- Repo flake via Nix: `nix run github:crocoder-dev/sce -- --help` or `nix profile install github:crocoder-dev/sce`
- Cargo: `cargo install sce`
- npm: `npm install -g sce`

Additional supported Cargo paths:

- `cargo install --git https://github.com/crocoder-dev/shared-context-engineering sce --locked`
- `cargo install --path cli --locked`

Homebrew is currently deferred from the active implementation stage.

- [Docs](https://sce.crocoder.dev/docs)
- [Getting Started](https://sce.crocoder.dev/docs/getting-started)
- [Motivation](https://sce.crocoder.dev/docs/motivation)

Built by [CroCoder](https://www.crocoder.dev/)
