# Glossary

- `sync-opencode-config`: Flake app command exposed as `nix run .#sync-opencode-config`; canonical operator entrypoint for staged regeneration/replacement of `config/` and replacement of repository-root `.opencode/` from regenerated `config/.opencode/`.
- generated-owned outputs: Files materialized by `config/pkl/generate.pkl` under `config/.opencode/**` and `config/.claude/**`.
