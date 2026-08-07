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

Integration checks are target-scoped. The doctor resolves which integration targets to inspect using the following priority:

1. **Configured targets**: If `.sce/config.json` has `integrations.target` with a non-empty array, only the listed targets (`opencode`, `claude`, `pi`) are inspected.
2. **Empty target array**: If `integrations.target` exists but is an empty array `[]`, the user has not recorded any integration targets. The doctor returns no targets and renders a guidance message instead of group rows.
3. **Directory detection fallback**: When config has no `integrations` property or `integrations.target` property is absent, the doctor falls back to detecting installed repo-root directories — `.opencode/` is detected as OpenCode, `.claude/` is detected as Claude, and `.pi/` is detected as Pi.
4. **No targets**: When directory detection identifies no installed directories either, the `Integrations` section renders `[FAIL] No integrations installed; run 'sce setup'` and a blocking `NoIntegrationsInstalled` problem is recorded, so the Summary counts it as a blocking problem.

Human text output renders group rows only for the resolved targets:

- `OpenCode plugins`
- `OpenCode agents`
- `OpenCode commands`
- `OpenCode skills`
- `ClaudeCode plugins`
- `ClaudeCode agents`
- `ClaudeCode commands`
- `ClaudeCode skills`
- `Pi prompts`
- `Pi skills`
- `Pi extensions`

Within a resolved target, the required inventory is additionally scoped to the repository's optional-workflow selection. The doctor reads `integrations.optional_workflows` from `.sce/config.json`; an absent, unreadable, or key-less file means nothing is selected. There is no directory-detection fallback for optional workflows. An unselected optional workflow's command file and skill subtree are not part of the required inventory, so no child row and no missing-file problem is produced for them. A selected optional workflow's assets are required inventory like any core workflow's, keeping `[MISS]` and content-mismatch `[FAIL]` detection unchanged. Files belonging to a previously selected but now unselected optional workflow are not reported as stray; the doctor simply stops expecting them. See [setup local bootstrap](setup-repo-local-config-bootstrap.md).

Integration checks for this contract inspect installed repo-root artifacts only.
They validate file presence and content against embedded OpenCode, Claude, and Pi setup assets: byte-exact `sha256` for every asset except the two JSON configs `sce setup` installs by merge (`.claude/settings.json`, `.opencode/opencode.json`), which instead validate that the file's SCE-owned fragment matches the embedded catalog — a file that also carries extra user keys, permissions, or plugins still renders `[PASS]` as long as that fragment is current (see [non-destructive setup install merge seam](setup-no-backup-policy-seam.md)).
Generated `config/.opencode/**`, `config/.claude/**`, and `config/.pi/**` trees are out of scope for doctor integration checks in this change stream.

Required git hooks (`Git Hooks` section) are a third merge-target family with the same fragment-currency rule: a hook is `[PASS]` when merging the canonical template into its on-disk bytes is a no-op, whether or not foreign content (a hand-written hook, husky, lefthook) sits around the SCE managed block — not byte-exact equality against the canonical hook (see [git hooks install contract](setup-githooks-install-contract.md)).

Claude installed assets are grouped by repo-root `.claude/` relative path:

- `settings.json` and `hooks/**` -> `ClaudeCode plugins` (including `hooks/run-sce-or-show-install-guidance.sh`)
- `agents/**` -> `ClaudeCode agents`
- `commands/**` -> `ClaudeCode commands`
- `skills/**` -> `ClaudeCode skills`

Pi installed assets are grouped by repo-root `.pi/` relative path:

- `prompts/**` -> `Pi prompts`
- `skills/**` -> `Pi skills`
- `extensions/**` -> `Pi extensions`

For each resolved target, the grouped installed repo-root asset trees are required inventory.
If any required file in an integration group is missing or mismatched:

- missing child rows render `[MISS]`
- mismatched child rows render `[FAIL]` and include a content-mismatch detail
- the parent integration group renders `[FAIL]`

An integration group renders `[PASS]` only when every required installed file in that group is present.

Healthy integration parent rows render the group name only.
Integration child rows render as `[STATUS] relative/path (absolute/path)` in text mode.

## Non-goals for this contract slice

- no JSON output shape or semantic changes
- no Claude plugin registry or preset-catalog checks

These non-goals scoped the original text-contract slice only. A later plan (`non-destructive-setup-install` task `T05`) added `sce doctor --fix` behavior for the two merge-target JSON configs: when their SCE-owned fragment is missing or stale, `--fix` reinstalls just that one asset through the same per-asset merge-install path `sce setup` uses, leaving every other asset and every user key untouched. The status vocabulary and section order above are unchanged by that addition.

See also: [doctor operator contract](agent-trace-hook-doctor.md), [CLI command surface](../cli/cli-command-surface.md).
