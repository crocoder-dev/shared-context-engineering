# MCP Generated Config Canonical Source

The generated-config pipeline now has one canonical Pkl-authored source for the local `sce mcp` stdio server.

## Source of truth

- `config/pkl/base/mcp.pkl` defines the shared `sce` server contract.
- The canonical fields are `name`, `transport`, `command`, and `subcommand`.
- The current contract resolves to the local command line `sce mcp`.

## Renderer handoff

- `config/pkl/renderers/common.pkl` re-exports the canonical server as `sceMcpServer`.
- OpenCode and Claude renderer code should consume `sceMcpServer` instead of restating `sce`, `mcp`, or stdio transport literals.

## OpenCode generated outputs

- `config/pkl/renderers/opencode-content.pkl` and `config/pkl/renderers/opencode-automated-content.pkl` each render an `opencodeConfig` text artifact from `sceMcpServer`.
- `config/pkl/generate.pkl` writes those artifacts to `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`.
- Both generated OpenCode profiles register the local `sce` MCP server with the documented local MCP schema: `type: "local"`, `command: ["sce", "mcp"]`, `enabled: true`.

## Claude generated output

- `config/pkl/renderers/claude-content.pkl` renders a `mcpProjectConfig` text artifact from `sceMcpServer`.
- `config/pkl/generate.pkl` writes that artifact to `config/.mcp.json` using Claude Code's project-scoped `mcpServers` schema.
- `scripts/sync-opencode-config.sh` treats `config/.mcp.json` as generated-owned input and replaces repository-root `.mcp.json` from staged output after regenerating `config/`.
- The current generated Claude project manifest registers `sce` with `command: "sce"`, `args: ["mcp"]`, and an empty `env` object.

## Ownership and edit policy

- Treat `config/.opencode/opencode.json`, `config/automated/.opencode/opencode.json`, and `config/.mcp.json` as generated-owned artifacts.
- Treat repository-root `.mcp.json` as a synced install target derived from generated `config/.mcp.json`, not as a hand-authored source file.
- When the MCP registration contract changes, edit canonical sources under `config/pkl/` (`config/pkl/base/mcp.pkl`, relevant renderer modules, and `config/pkl/generate.pkl`) instead of patching generated manifests directly.
- `config/pkl/README.md` is the contributor-facing runbook for regeneration, ownership, and sync behavior; keep it aligned with this contract.

## Current serialized helpers

- `command_line` exposes the deterministic shell form `sce mcp`.
- `command_argv_json` exposes the canonical JSON argv form `["sce", "mcp"]`.
- `stdio_args_json` exposes the canonical stdio args form `["mcp"]`.

## Verification

- `nix develop -c pkl eval -m . config/pkl/generate.pkl`
- `nix run .#pkl-check-generated`
- `nix flake check`

See also: [../architecture.md](../architecture.md), [../patterns.md](../patterns.md), [../glossary.md](../glossary.md)
