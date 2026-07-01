# Generated OpenCode Plugin Registration

The generated-config pipeline now has one canonical Pkl-authored source for OpenCode plugin registration used by SCE-managed plugins.

## Source of truth

- `config/pkl/base/opencode.pkl` defines canonical `CanonicalOpenCodePluginRegistration` entries.
- The current canonical entries are `sce_bash_policy_plugin` (`./plugins/sce-bash-policy.ts`) and `sce_agent_trace_plugin` (`./plugins/sce-agent-trace.ts`).
- The current registration scope is intentionally limited to SCE-generated OpenCode plugins emitted by this repository.

## Renderer handoff

- `config/pkl/renderers/common.pkl` re-exports the canonical plugin list as `sceGeneratedOpenCodePlugins`.
- The same module also exposes `sceGeneratedOpenCodePluginPathsJson` so OpenCode renderers can serialize the documented `plugin` manifest field without restating path literals.
- OpenCode renderer code should consume these shared exports instead of hardcoding plugin paths in renderer-local templates.

## OpenCode generated outputs

- `config/pkl/renderers/opencode-content.pkl` and `config/pkl/renderers/opencode-automated-content.pkl` render `opencodeConfig` artifacts that include the shared plugin registration.
- `config/pkl/generate.pkl` writes those artifacts to `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`.
- Both generated OpenCode profiles currently serialize `plugin: ["./plugins/sce-bash-policy.ts", "./plugins/sce-agent-trace.ts"]`.
- The generated plugin files currently registered by those manifests are `config/.opencode/plugins/sce-bash-policy.ts`, `config/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-bash-policy.ts`, and `config/automated/.opencode/plugins/sce-agent-trace.ts`.

## Claude boundary

- Claude does not consume the OpenCode `plugin` manifest surface.
- Claude agent-trace event handling is registered through generated `.claude/settings.json` command hooks that call `.claude/hooks/run-sce-or-show-install-guidance.sh` before invoking `sce hooks`: `SessionStart` → `sce hooks session-model`, matched `PostToolUse Write|Edit|MultiEdit|NotebookEdit` → `sce hooks diff-trace`, and supported conversation events → `sce hooks conversation-trace`.
- The Rust CLI receives raw Claude hook event JSON on STDIN and handles extraction, validation, and persistence without a TypeScript translation layer.
- Claude bash-policy enforcement is registered through generated `.claude/settings.json` as a `PreToolUse` `Bash` command hook that calls the same generated helper before running `sce policy bash` and passing raw hook event JSON on STDIN.
- The Claude helper emits `sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli` and exits successfully when `sce` is missing, preserving fail-open hook behavior; when `sce` exists it `exec`s the original command arguments.
- OpenCode bash-policy enforcement delegates to the same Rust `sce policy bash` command through a thin generated plugin wrapper; the former TypeScript runtime (`bash-policy/runtime.ts`) has been removed from generated outputs.

## Ownership and edit policy

- Treat `config/.opencode/opencode.json`, `config/automated/.opencode/opencode.json`, and the corresponding generated plugin files under `config/.opencode/plugins/` and `config/automated/.opencode/plugins/` as generated-owned artifacts.
- When OpenCode plugin registration changes, edit canonical sources under `config/pkl/` (`config/pkl/base/opencode.pkl`, `config/pkl/renderers/common.pkl`, the OpenCode renderer modules, and `config/pkl/generate.pkl` when ownership wiring changes) instead of patching generated manifests directly.
- Do not broaden this contract to third-party or user-supplied plugins without an explicit plan/task that defines new ownership and scope rules.

## Verification

- Inspect `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json` for the generated `plugin` field.
- Inspect `config/.opencode/plugins/sce-bash-policy.ts`, `config/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-bash-policy.ts`, and `config/automated/.opencode/plugins/sce-agent-trace.ts` for the generated plugin implementations.
- Verify `config/.claude/settings.json` contains the generated `PreToolUse` `Bash` policy hook, verify `config/.claude/hooks/run-sce-or-show-install-guidance.sh` contains the missing-CLI guidance path, and verify `config/.claude/` still contains no Claude bash-policy TypeScript runtime files.

See also: [../overview.md](../overview.md), [../architecture.md](../architecture.md), [../glossary.md](../glossary.md)
