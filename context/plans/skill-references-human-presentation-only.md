# Plan: skill-references-human-presentation-only

## Change summary

Claude still confuses multi-file skill `references/` packages after the Markdown result-contract conversion: result contracts, plan templates, style guides, and human-facing presentation layouts all live side by side under `references/`, so the agent mixes handoff shapes with chat presentation.

Apply a **Claude-only** package shape: for generated Claude skill packages, `references/` holds **at most one file**, and that file is **only** the layout used to present work to a human reviewer. Machine-facing material—phase result contracts, plan templates, and similar agent contracts—is folded into each Claude package’s `SKILL.md` in the Claude renderer. Canonical Pkl packages, OpenCode, and Pi keep their existing multi-file inventories and YAML contracts unchanged.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Every generated **Claude** skill package has either zero or one file under `references/`, and when a reference file exists it is a human-facing presentation layout (plan summary, implementation gate, validation report, sync report, or commit-message style), not a phase result contract or plan-template contract. OpenCode and Pi skill packages keep their existing multi-file reference inventories.
  - Validate: `tmp="$(mktemp -d)"; nix run .#pkl-generate -- "$tmp"; find "$tmp/config/.claude" -path '*/skills/*/references/*' | sort` and assert each Claude package directory contains ≤1 reference file with an allowed human-presentation name; confirm OpenCode/Pi still include machine contracts under `references/`.
- [x] AC2: Phase result contracts and plan-template content remain available inside each Claude package’s `SKILL.md` so commands still receive the same status variants and command-readable fields; no Claude skill depends on a sibling package for those contracts.
  - Validate: inspect generated Claude `SKILL.md` files for the five Markdown phase-result skills plus `sce-validation`; confirm each still documents its return statuses/fields and, where applicable, plan-template rules; confirm no Claude `references/*-contract*` or `references/plan-template.md` paths remain.
- [x] AC3: Exact per-target metadata coverage and deterministic ephemeral generation accept Claude’s reduced inventory while OpenCode/Pi inventories stay byte-stable for skill document paths.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix run .#pkl-check-generated`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` so the single human-presentation `references/` rule is described as a **Claude-only** render shape, not a canonical/OpenCode/Pi package change.
- Update `config/pkl/README.md` generation docs for the Claude-only single human-presentation reference rule.
- Do **not** require `.pi/` baseline inventory changes or OpenCode/Pi reference reductions.

## Constraints and non-goals

- **In scope:** Claude renderer packaging (`claude-workflow-results.pkl` / `claude-content.pkl`), Claude metadata coverage inventory, focused generation documentation, and durable context that names the Claude-only rule.
- **Out of scope:** Changing canonical Pkl workflow package inventories, OpenCode or Pi skill trees, project-root `.pi/` baselines, workflow status semantics, command sequencing, approval-gate content, Rust CLI behavior, hooks/settings/plugins, or inventing new user-facing surfaces.
- **Constraints:** Keep packages self-contained per target; keep OpenCode/Pi behavior and inventories equivalent to today; preserve every field Claude commands need to branch and render; author only in Pkl sources, not committed generated target trees.
- **Non-goal:** Applying the single-reference rule to OpenCode or Pi; removing human presentation layouts; collapsing multiple skills into one package; re-opening the YAML-vs-Markdown transport debate beyond relocating Claude’s machine contracts out of multi-file `references/`.

## Assumptions

- “Presenting to the human reviewer” means user-facing chat or plan-attached presentation layouts: `plan-summary.md`, `implementation-gate.md`, `validation-report.md`, `sync-report.md`, and `commit-message-style.md`. Phase result contracts (`*-contract*`, `context-brief*`, `validation-result.md`) and `plan-template.md` are agent contracts and, **for Claude only**, move into `SKILL.md`.
- Claude skills that today only have a machine contract (`sce-context-load`, `sce-plan-review`) end with **zero** reference files after the fold. Skills that already have a single human report (`sce-*-context-sync`) keep that one file.
- Canonical packages continue to author multi-file `references/` including YAML contracts; Claude’s renderer is the sole place the reduced inventory is produced.
- Inlining contracts may lengthen Claude `SKILL.md`; that is acceptable if section headings remain unambiguous.

## Task stack

- [x] T01: `Fold Claude machine references into SKILL.md` (status:done)
  - Task ID: T01
  - Goal: In the Claude renderer only, emit at most one `references/` document per skill package (human-presentation layouts only); fold Markdown phase-result contracts, plan-template, and validation-result content into the owning Claude `SKILL.md`; leave canonical packages and OpenCode/Pi inventories unchanged.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/claude-workflow-results.pkl`, `config/pkl/renderers/claude-content.pkl`, and Claude-only path rewrites/section appends for machine material. Out — canonical `config/pkl/base/workflow-*.pkl` inventory changes, OpenCode/Pi renderers, `.pi` baseline mirror.
  - Dependencies: none
  - Done when: Generated Claude packages keep only allowed human-presentation references (or none); each folded contract’s statuses/fields appear in Claude `SKILL.md`; generated OpenCode/Pi skill document path sets match the pre-change multi-file inventories.
  - Verification notes (commands or checks): temporary `nix run .#pkl-generate` inventory of Claude vs OpenCode/Pi references; inspect Claude `SKILL.md` for `## Result contract` / `## Plan template`.
  - Completed: 2026-07-28
  - Files changed: config/pkl/renderers/claude-workflow-results.pkl, config/pkl/renderers/claude-content.pkl
  - Evidence: Generated Claude references are only commit-message-style, plan-summary, sync-report (×2), implementation-gate, validation-report. OpenCode/Pi still emit `references/*-contract.yaml`. Each of the six Claude skills with former machine contracts has `## Result contract` in SKILL.md; plan-authoring also has `## Plan template`.
  - Notes: Scope revised mid-work from all-target to Claude-only; canonical base packages left unchanged.

- [x] T02: `Align Claude coverage docs with single-reference packages` (status:done)
  - Task ID: T02
  - Goal: Make exact metadata coverage and generation docs match Claude’s reduced single human-presentation reference inventories without changing OpenCode/Pi expected paths or workflow status semantics.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/README.md`, and durable context wording for the Claude-only rule. Out — OpenCode/Pi inventories, `.pi/skills/**`, application code.
  - Dependencies: T01
  - Done when: Metadata coverage expects the reduced Claude inventory and unchanged OpenCode/Pi inventories; docs state the rule is Claude-only.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix run .#pkl-check-generated`.
  - Completed: 2026-07-28
  - Files changed: config/pkl/renderers/metadata-coverage-check.pkl, config/pkl/README.md, context/patterns.md, context/architecture.md, context/overview.md, context/glossary.md
  - Evidence: `pkl eval config/pkl/renderers/metadata-coverage-check.pkl` succeeded with Claude reduced inventory (14 skill documents) and OpenCode/Pi still on 21 multi-file paths.
  - Notes: None.

## Open questions

- Is `commit-message-style.md` correctly treated as the human-presentation reference for Claude `/commit`, or should Claude commit packages have **zero** references with style folded into `SKILL.md` as well? Defaulting to keep the style guide as the single Claude reference.
- Should Claude keep `plan-summary.md` as the sole plan-authoring reference (default) rather than `plan-template.md`? Defaulting to `plan-summary.md` because it is the chat layout shown to the human after authoring.

## Validation Report

**Status:** validated  
**Date:** 2026-07-28

### Commands run

- `tmp="$(mktemp -d)"; nix run .#pkl-generate -- "$tmp"; find "$tmp/config/.claude" -path '*/skills/*/references/*' | sort` (plus OpenCode/Pi inventory and per-package ref-count checks) -> exit 0 (Claude: 6 human-presentation refs, ≤1 per package; no machine contracts under Claude `references/`; OpenCode and Pi each still emit 13 multi-file refs including `*-contract.yaml` / `plan-template.md` / `validation-result.md`)
- Inspect generated Claude `SKILL.md` for phase-result skills plus `sce-validation` (and related packages) -> exit 0 (`## Result contract` present for sce-context-load, sce-plan-authoring, sce-plan-review, sce-task-execution, sce-validation, sce-atomic-commit; `## Plan template` present for sce-plan-authoring; no Claude `references/*-contract*`, `plan-template.md`, or `validation-result.md`)
- `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` -> exit 0 (all inventoryChecks complete, including claude-skill-documents and opencode/pi skill documents)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 78 files)
- `nix flake check` -> exit 0 (all checks passed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Claude packages have 0–1 `references/` files, all human-presentation names only (`plan-summary`, `implementation-gate`, `validation-report`, `sync-report`×2, `commit-message-style`); OpenCode/Pi keep multi-file machine contracts -> ephemeral generate inventory
- [x] AC2: Folded result contracts and plan-template live in Claude `SKILL.md`; no Claude machine-contract reference paths remain -> SKILL.md inspection
- [x] AC3: Metadata coverage and ephemeral generation accept reduced Claude inventory with OpenCode/Pi inventories intact -> coverage eval + `pkl-check-generated`

### Failed checks and follow-ups

- None.

### Residual risks

- Open questions remain about whether Claude should eventually drop `commit-message-style.md` / keep `plan-summary.md` vs fold further; current defaults match the plan and pass acceptance checks.
- None identified beyond those open questions.
