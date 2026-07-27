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

- `config/pkl/renderers/opencode-content.pkl` renders the `opencodeConfig` artifact with the shared plugin registration.
- `config/pkl/generate.pkl` writes that artifact to payload-relative `config/.opencode/opencode.json` beneath the selected temporary/`OUT_DIR` root.
- The generated OpenCode profile serializes `plugin: ["./plugins/sce-bash-policy.ts", "./plugins/sce-agent-trace.ts"]`.
- The registered generated plugin files are `config/.opencode/plugins/sce-bash-policy.ts` and `config/.opencode/plugins/sce-agent-trace.ts`. The removed `config/automated/.opencode` profile has no plugin manifest or generated plugin copies.

## Claude boundary

- Claude does not consume the OpenCode `plugin` manifest surface.
- Claude agent-trace event handling is registered through generated `.claude/settings.json` command hooks that call `.claude/hooks/run-sce-or-show-install-guidance.sh` before invoking `sce hooks`: matched `PostToolUse Write|Edit|MultiEdit|NotebookEdit` → `sce hooks diff-trace`, and supported conversation events → `sce hooks conversation-trace`. `SessionStart` is no longer registered and `sce hooks session-model` is no longer supported.
- The Rust CLI receives raw Claude hook event JSON on STDIN and handles extraction, validation, and persistence without a TypeScript translation layer.
- Claude bash-policy enforcement is registered through generated `.claude/settings.json` as a `PreToolUse` `Bash` command hook that calls the same generated helper before running `sce policy bash` and passing raw hook event JSON on STDIN.
- The Claude helper emits `sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli` and exits successfully when `sce` is missing, preserving fail-open hook behavior; when `sce` exists it `exec`s the original command arguments.
- OpenCode bash-policy enforcement delegates to the same Rust `sce policy bash` command through a thin generated plugin wrapper; the former TypeScript runtime (`bash-policy/runtime.ts`) has been removed from generated outputs.

## Ownership and edit policy

- Treat the `config/.opencode/opencode.json` and `config/.opencode/plugins/` names as ephemeral payload layouts. The repository must not contain those generated paths.
- When OpenCode plugin registration changes, edit canonical sources under `config/pkl/` and `config/lib/`, then inspect a temporary generation or Cargo `OUT_DIR` rather than patching generated manifests.
- Do not broaden this contract to third-party or user-supplied plugins without an explicit plan/task that defines new ownership and scope rules.

## Verification

- Run `nix run .#pkl-check-generated` and inspect an explicit temporary generation root when field-level review is needed.
- Verify the temporary OpenCode manifest/plugin files and Claude settings/hook helper, while asserting repository `config/.opencode`, `config/.claude`, and `config/.pi` paths remain absent.

See also: [../overview.md](../overview.md), [../architecture.md](../architecture.md), [../glossary.md](../glossary.md)
