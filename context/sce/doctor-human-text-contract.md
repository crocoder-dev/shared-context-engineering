# SCE doctor human text contract

The default human-readable `sce doctor` report is a compact, domain-oriented
health summary. Diagnosis remains complete and read-only; JSON remains the
full-detail machine-readable route.

## Text-mode section order

Human text output renders these sections in this exact order:

1. `Environment`
2. `Repository`
3. `Integrations`

The `Environment` domain contains `State`, `Configuration`, and `Repository
identity`. The `Repository` domain contains `Git repository` and `Git hooks`.

## Header and status vocabulary

- Diagnose mode renders `SCE doctor`.
- Fix mode renders `SCE doctor fix` and appends the existing `Fix results`
  section.
- `[PASS]` means the node is healthy.
- `[WARN]` means a non-blocking warning exists below the node.
- `[FAIL]` means a blocking failure exists below the node.
- `[MISS]` is reserved for a missing required leaf; parent aggregation may
  promote that condition to `[FAIL]`.

When shared CLI color output is enabled, pass is green, warning is yellow, and
fail/miss are red. Non-TTY and `NO_COLOR` output contains the exact bracketed
tokens without ANSI sequences.

Healthy rows contain only their status and display label. They do not expose
absolute paths, UUIDs, repository IDs, hashes, canonical identities, configured
remote names, or individual integration asset paths.

## Integration hierarchy

Integration checks remain target-scoped. The doctor resolves targets using this
priority:

1. A non-empty `.sce/config.json` `integrations.target` array selects only the
   listed targets (`opencode`, `claude`, `pi`).
2. An explicitly empty target array selects no targets and renders the no-target
   guidance row.
3. Without a configured target property, repo-root `.opencode/`, `.claude/`,
   and `.pi/` directories are detected.

Only resolved targets render. Display labels are normalized as `Claude Code`,
`OpenCode`, and `Pi`; typed target/area keys, not display-label parsing, own the
hierarchy. Areas render in deterministic order:

- Claude Code: `Plugins`, `Agents`, `Commands`, `Skills`
- OpenCode: `Plugins`, `Agents`, `Commands`, `Skills`
- Pi: `Extensions`, `Prompts`, `Skills`

Healthy areas render one concise `[PASS]` row and never list installed files.
The report and JSON payload still retain the complete inspected asset facts for
diagnostics and later unhealthy-branch rendering.

When no integration target is resolved, the `Integrations` section renders:

`[FAIL] No integrations installed; run 'sce setup'`

The existing optional-workflow selection remains the source of truth for which
assets inspection expects. An unselected optional workflow produces no required
child fact or missing-file problem.

## Compatibility boundary

The compact text layout is intentionally a human-facing contract change. JSON
field names, identity/path/problem detail, readiness classification, exit-code
semantics, stream ownership, diagnosis read-only behavior, and fix behavior
remain unchanged. Scripts should use `--format json` rather than parse compact
text.

See also [doctor operator contract](agent-trace-hook-doctor.md) and [CLI command
surface](../cli/cli-command-surface.md).
