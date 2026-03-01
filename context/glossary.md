# Glossary

- `sync-opencode-config`: Flake app command exposed as `nix run .#sync-opencode-config`; canonical operator entrypoint for staged regeneration/replacement of `config/` and replacement of repository-root `.opencode/` from regenerated `config/.opencode/`.
- generated-owned outputs: Files materialized by `config/pkl/generate.pkl` under `config/.opencode/**` and `config/.claude/**`.
- `agnix-config-validate-report`: GitHub Actions workflow at `.github/workflows/agnix-config-validate-report.yml` that runs `nix develop -c agnix validate .` from `config/` on push/PR to `main`.
- `agnix validation report artifact`: Failure-investigation artifact named `agnix-validate-report`, uploaded from deterministic path `context/tmp/ci-reports/agnix-validate-report.txt` when non-info (`warning:`/`error:`/`fatal:`) findings are detected.
- `sce` (placeholder CLI): Rust binary crate at `cli/` that currently provides only command-surface scaffolding and deterministic placeholder messaging.
- `command surface contract`: The static command catalog in `cli/src/command_surface.rs` that marks each top-level command as `implemented` or `placeholder`.
- `sce dependency contract`: Minimal crate dependency baseline declared in `cli/Cargo.toml` and referenced via `cli/src/dependency_contract.rs` (`anyhow`, `lexopt`, `tokio`, `turso`).
