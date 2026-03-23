# Generated OpenCode Plugin Registration

The generated-config pipeline now has one canonical Pkl-authored source for OpenCode plugin registration used by SCE-managed plugins.

## Source of truth

- `config/pkl/base/opencode.pkl` defines canonical `CanonicalOpenCodePluginRegistration` entries.
- The current implemented entry is `sce_bash_policy_plugin` with path `./plugins/sce-bash-policy.ts`.
- The current registration scope is intentionally limited to SCE-generated OpenCode plugins emitted by this repository.

## Renderer handoff

- `config/pkl/renderers/common.pkl` re-exports the canonical plugin list as `sceGeneratedOpenCodePlugins`.
- The same module also exposes `sceGeneratedOpenCodePluginPathsJson` so OpenCode renderers can serialize the documented `plugin` manifest field without restating path literals.
- OpenCode renderer code should consume these shared exports instead of hardcoding plugin paths in renderer-local templates.

## OpenCode generated outputs

- `config/pkl/renderers/opencode-content.pkl` and `config/pkl/renderers/opencode-automated-content.pkl` render `opencodeConfig` artifacts that include the shared plugin registration.
- `config/pkl/generate.pkl` writes those artifacts to `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`.
- Both generated OpenCode profiles currently serialize `plugin: ["./plugins/sce-bash-policy.ts"]`.
- The registered plugin file itself is generated-owned at `config/.opencode/plugins/sce-bash-policy.ts` and `config/automated/.opencode/plugins/sce-bash-policy.ts`.

## Claude boundary

- Claude does not consume this OpenCode `plugin` manifest surface.
- Claude bash-policy enforcement has been removed from generated outputs.
- OpenCode is now the sole target for SCE-managed bash-policy enforcement via the plugin registration contract.

## Ownership and edit policy

- Treat `config/.opencode/opencode.json`, `config/automated/.opencode/opencode.json`, and the corresponding generated plugin files under `config/.opencode/plugins/` and `config/automated/.opencode/plugins/` as generated-owned artifacts.
- When OpenCode plugin registration changes, edit canonical sources under `config/pkl/` (`config/pkl/base/opencode.pkl`, `config/pkl/renderers/common.pkl`, the OpenCode renderer modules, and `config/pkl/generate.pkl` when ownership wiring changes) instead of patching generated manifests directly.
- Do not broaden this contract to third-party or user-supplied plugins without an explicit plan/task that defines new ownership and scope rules.

## Verification

- Inspect `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json` for the generated `plugin` field.
- Inspect `config/.opencode/plugins/sce-bash-policy.ts` and `config/automated/.opencode/plugins/sce-bash-policy.ts` for the generated plugin implementation.
- Verify `config/.claude/` contains no bash-policy files (no `lib/bash-policy-*`, no `hooks/sce-bash-policy-hook.js`, no bash-policy hooks in `settings.json`).

See also: [../overview.md](../overview.md), [../architecture.md](../architecture.md), [../glossary.md](../glossary.md)
