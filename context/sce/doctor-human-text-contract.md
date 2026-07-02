# SCE doctor human text contract

Plan `doctor-human-text-integration-audit` task `T01` locks the approved human-facing `sce doctor` text contract for downstream implementation tasks.
This contract is implemented by the current runtime and remains normative for future changes.

## Text-mode section order

Human text output for `sce doctor` must render these sections in this exact order:

1. `Environment`
2. `Configuration` (includes Agent Trace DB health row)
3. `Repository`
4. `Git Hooks`
5. `Integrations`

## Human text status vocabulary

Human text rows must use exactly this status vocabulary:

- `[PASS]`: healthy
- `[FAIL]`: SCE will not work unless fixed
- `[MISS]`: required file is missing

No alternate human text status labels are allowed for this layout.

When shared CLI color output is enabled, `[PASS]` renders green and `[FAIL]` / `[MISS]` render red.
When color is disabled, human text still renders the exact bracketed tokens without ANSI sequences.

## Header and row formatting

Diagnose mode renders the header `SCE doctor diagnose`.
Fix mode renders the header `SCE doctor fix`.

Human text rows with path detail use the simplified `label (path)` form.
Healthy human rows do not append redundant prose such as `present`, `expected`, or `all required files present`.

Repository rows use the labels `Repository` and `Hooks` in text mode.

## Git Hooks text simplification

Human text output for `Git Hooks` is simplified to top-level required-hook presence rows only.
Nested human text rows for hook `content` or `executable` detail are not part of the approved layout.
This simplification is text-mode only and does not change JSON output requirements.

## Integrations text contract

Human text output for `Integrations` must use exactly these groups:

- `OpenCode plugins`
- `OpenCode agents`
- `OpenCode commands`
- `OpenCode skills`
- `ClaudeCode plugins`
- `ClaudeCode agents`
- `ClaudeCode commands`
- `ClaudeCode skills`

Integration checks for this contract inspect installed repo-root artifacts only.
They validate file presence and content hashes against embedded OpenCode and Claude setup assets.
Generated `config/.opencode/**` and `config/.claude/**` trees are out of scope for doctor integration checks in this change stream.

Claude installed assets are grouped by repo-root `.claude/` relative path:

- `settings.json` and `hooks/**` -> `ClaudeCode plugins` (including `hooks/run-sce-or-show-install-guidance.sh`)
- `agents/**` -> `ClaudeCode agents`
- `commands/**` -> `ClaudeCode commands`
- `skills/**` -> `ClaudeCode skills`

For `agents`, `commands`, and `skills`, the installed repo-root trees are required inventory.
If any required file in an integration group is missing or mismatched:

- missing child rows render `[MISS]`
- mismatched child rows render `[FAIL]` and include a content-mismatch detail
- the parent integration group renders `[FAIL]`

An integration group renders `[PASS]` only when every required installed file in that group is present.

Healthy integration parent rows render the group name only.
Integration child rows render as `[STATUS] relative/path (absolute/path)` in text mode.

## Non-goals for this contract slice

- no JSON output shape or semantic changes
- no `sce doctor --fix` behavior changes
- no Claude plugin registry or preset-catalog checks

See also: [doctor operator contract](agent-trace-hook-doctor.md), [CLI command surface](../cli/cli-command-surface.md).
