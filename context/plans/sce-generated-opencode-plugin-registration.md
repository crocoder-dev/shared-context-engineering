# Plan: sce-generated-opencode-plugin-registration

## Change summary

Add canonical Pkl-owned OpenCode plugin registration for SCE-generated plugins so the generated `opencode.json` manifests include the existing SCE bash-policy plugin automatically for both manual and automated profiles. Per current code truth and Context7 guidance, Claude hook enablement already uses generated `settings.json` hook registration, so this plan does not add an OpenCode-style Claude plugin-registration track.

## Success criteria

- Canonical Pkl sources define the OpenCode plugin registration for the existing SCE-generated plugin instead of relying on generated-file edits.
- Regenerated `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json` include the documented OpenCode `plugin` field for the SCE-generated plugin while preserving the existing MCP registration.
- Generated OpenCode package/runtime ownership stays coherent for the registered plugin path and does not broaden scope to non-SCE plugins.
- Durable context reflects the current generated-plugin registration contract and the reason Claude needs no parallel registration change.
- Regeneration, parity, and repo validation checks are captured in the final task.

## Constraints and non-goals

- In scope: canonical Pkl sources, renderer/generator wiring, generated OpenCode manifests, and context updates required for the generated-plugin contract.
- In scope: only SCE-generated OpenCode plugins already emitted by this repo, starting with `sce-bash-policy.js`.
- Constraint: edit canonical sources under `config/pkl/`; do not hand-edit `config/.opencode/**` or `config/automated/.opencode/**` as primary sources.
- Constraint: preserve the current OpenCode MCP registration and existing generated package dependency support.
- Constraint: use the documented OpenCode `plugin` manifest field confirmed via Context7.
- Out of scope: third-party or user-supplied plugin registration, new OpenCode plugin behavior, Claude plugin parity work, and changes to Claude hook runtime unless current code truth proves the existing generated `settings.json` hook registration is broken.

## Assumptions

- The only plugin to register in this change is the existing generated SCE bash-policy plugin path.
- OpenCode plugin registration should use the generated manifest field `plugin` rather than a repo-specific extension.
- Claude Code's current generated hook enablement in `config/.claude/settings.json` is sufficient unless implementation uncovers a documented compatibility issue.

## Task stack

- [x] T01: `Define canonical SCE OpenCode plugin registration source` (status:done)
  - Task ID: T01
  - Goal: Add a canonical Pkl-owned representation for the existing SCE-generated OpenCode plugin registration so renderer code can emit one consistent plugin list without duplicating literals.
  - Boundaries (in/out of scope): In - `config/pkl/` shared constants/data structures and minimal renderer-facing plumbing for the existing SCE plugin registration. Out - generated manifest edits, new plugin runtime behavior, third-party plugins, and Claude hook changes.
  - Done when: One canonical Pkl source defines the SCE-generated OpenCode plugin entry and both manual/automated renderers can consume it without restating plugin literals separately.
  - Verification notes (commands or checks): Inspect canonical Pkl sources to confirm one source of truth for the SCE plugin entry; regenerate outputs and confirm both OpenCode renderers consume that source.
  - Completed: 2026-03-18
  - Files changed: `config/pkl/base/opencode.pkl`, `config/pkl/renderers/common.pkl`, `config/pkl/renderers/opencode-content.pkl`, `config/pkl/renderers/opencode-automated-content.pkl`, `context/plans/sce-generated-opencode-plugin-registration.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added one canonical SCE OpenCode plugin-registration source plus shared renderer-facing plumbing; manifest emission remains scoped to T02, and context sync required a minimal root architecture update because T01 introduced a new shared `config/pkl/base/opencode.pkl` module.

- [x] T02: `Render SCE plugin registration into OpenCode manifests` (status:done)
  - Task ID: T02
  - Goal: Update the manual and automated OpenCode renderers so generated `opencode.json` files include the documented `plugin` field for the existing SCE-generated plugin while preserving current MCP configuration.
  - Boundaries (in/out of scope): In - `config/pkl/renderers/opencode-content.pkl`, `config/pkl/renderers/opencode-automated-content.pkl`, generated `config/.opencode/opencode.json`, and generated `config/automated/.opencode/opencode.json`. Out - package dependency redesign, plugin runtime code changes, non-OpenCode targets, and unrelated manifest cleanup.
  - Done when: Both generated OpenCode manifests include the SCE plugin registration and still serialize the existing `mcp` block correctly from canonical sources.
  - Verification notes (commands or checks): Regenerate Pkl outputs; inspect `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`; run `nix run .#pkl-check-generated`.
  - Completed: 2026-03-18
  - Files changed: `config/pkl/renderers/opencode-content.pkl`, `config/pkl/renderers/opencode-automated-content.pkl`, `config/.opencode/opencode.json`, `config/automated/.opencode/opencode.json`, `context/plans/sce-generated-opencode-plugin-registration.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; inspected generated `plugin` + preserved `mcp` blocks in both OpenCode manifests; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Renderers now emit the canonical SCE-generated plugin path as the OpenCode `plugin` array for both manual and automated profiles; this is a verify-only context-sync change because durable plugin-ownership docs are scoped to T03.

- [x] T03: `Document generated plugin ownership and Claude boundary` (status:done)
  - Task ID: T03
  - Goal: Update durable context so future sessions know OpenCode plugin registration is generated from canonical Pkl sources, limited to SCE-generated plugins, and Claude hook enablement remains settings-driven rather than OpenCode-style plugin registration.
  - Boundaries (in/out of scope): In - focused current-state context updates in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and/or a focused `context/sce/` contract file if needed. Out - speculative Claude redesign, historical narration, and unrelated documentation cleanup.
  - Done when: Durable docs identify the generated OpenCode plugin-registration paths, point edits back to canonical Pkl ownership, and explicitly note that Claude currently uses generated hook registration via `settings.json` so no parallel plugin-registration contract is introduced here.
  - Verification notes (commands or checks): Review updated context against generated outputs and existing Claude settings wiring; confirm plugin ownership and the Claude boundary are discoverable from current-state docs.
  - Completed: 2026-03-18
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/generated-opencode-plugin-registration.md`, `context/plans/sce-generated-opencode-plugin-registration.md`
  - Evidence: inspected generated `plugin` fields in `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`; inspected generated Claude hook registration in `config/.claude/settings.json`; `nix run .#pkl-check-generated`; `nix flake check` (fails in existing CLI test `services::hooks::tests::prompt_capture_flow_persists_and_queries_end_to_end` with git ref write error during sandboxed commit setup)
  - Notes: Important-change context sync updated shared root docs plus a focused `context/sce/` contract file so generated OpenCode plugin ownership and the Claude hook-based boundary are directly discoverable.

- [x] T04: `Validate generation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Run final regeneration and validation, confirm generated parity, and ensure no temporary scaffolding remains before closing the plan.
  - Boundaries (in/out of scope): In - final regeneration, parity checks, repo validation, cleanup verification, and final context-sync verification for the generated-plugin contract. Out - new functionality beyond fixes required by failed validation.
  - Done when: Generated outputs are in sync, validation evidence is captured, any temporary scaffolding is removed, and current-state context matches the finalized OpenCode plugin-registration contract.
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-03-18
  - Files changed: `context/plans/sce-generated-opencode-plugin-registration.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Regeneration completed without introducing new generated drift, parity passed, flake validation passed, and no task-specific temporary scaffolding was required for cleanup.

## Open questions

- None. Scope is fixed to SCE-generated OpenCode plugins in canonical Pkl sources, and Claude is intentionally left on the existing generated hook-registration path unless implementation evidence shows breakage.

## Readiness

- ready_for_implementation: yes
- recommended_next_task: none
- blockers: none
- ambiguity: low
- missing_acceptance_criteria: none

## Handoff

- plan_name: `sce-generated-opencode-plugin-registration`
- plan_path: `context/plans/sce-generated-opencode-plugin-registration.md`
- next command: `none - final task completed`

## Validation Report

### Commands run

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` -> exit 0; regeneration completed and rewrote generated-owned outputs deterministically from canonical Pkl sources.
- `nix run .#pkl-check-generated` -> exit 0; reported `Generated outputs are up to date.`
- `nix flake check` -> exit 0; evaluated and ran `cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity` successfully.

### Temporary scaffolding

- No task-specific temporary scaffolding or debug-only files were introduced by this plan.

### Success-criteria verification

- [x] Canonical Pkl sources define the OpenCode plugin registration for the existing SCE-generated plugin instead of relying on generated-file edits -> confirmed by `config/pkl/base/opencode.pkl` and shared renderer wiring in `config/pkl/renderers/common.pkl`.
- [x] Regenerated `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json` include the documented OpenCode `plugin` field while preserving MCP registration -> confirmed by generated manifests containing `plugin: ["./plugins/sce-bash-policy.js"]` and the existing `mcp.sce` block.
- [x] Generated OpenCode package/runtime ownership stays coherent for the registered plugin path and does not broaden scope to non-SCE plugins -> confirmed by the focused contract in `context/sce/generated-opencode-plugin-registration.md` and unchanged generated plugin ownership under `config/.opencode/plugins/` and `config/automated/.opencode/plugins/`.
- [x] Durable context reflects the current generated-plugin registration contract and the reason Claude needs no parallel registration change -> confirmed by `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and `context/sce/generated-opencode-plugin-registration.md`.
- [x] Regeneration, parity, and repo validation checks are captured in the final task -> confirmed in `T04` evidence and this validation report.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified within the approved scope.
