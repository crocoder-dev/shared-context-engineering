# Plan: update-sce-skill-orchestration

## Change summary

Update the canonical workflow-rendering sources so SCE skills retain exclusive ownership of SCE workflow control flow while unrelated skills remain usable as helper capabilities during the active step. Replace broad skill prohibitions in canonical workflow Purpose/Rules content with SCE-scoped wording, and add one shared target-neutral helper-composition rule that preserves phase order, gates, waits, writes, validation, stops, and output contracts.

Update the catalog-derived OpenCode agent permission rendering so non-SCE skills are allowed by default, arbitrary `sce-*` skills are denied, and each agent's explicitly owned SCE workflows (plus the existing Code-agent `sce-decision` exception) are allowed after the wildcard deny. Preserve Claude's generic `Skill` tool exposure and Pi's extension/runtime behavior. Strengthen generated-output contract checks and inspect temporary OpenCode, Claude, and Pi payloads without modifying ephemeral generated trees.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Every generated SCE workflow skill scopes workflow-control prohibitions to SCE skills/packages/commands and explicitly permits relevant non-SCE helper skills while stating that helper use returns control to the active step without weakening its invariants; no generated workflow contains an unscoped `Never invoke another skill`-style prohibition.
  - Validate: Generate a temporary payload and inspect every `SKILL.md` under `.opencode/skills/`, `.claude/skills/`, and `.pi/skills/`; run focused searches for unscoped prohibition text and required helper-rule language.
- [x] AC2: Generated OpenCode Plan and Code agents allow ordinary non-SCE skills without prompting, deny arbitrary `sce-*` skills, and allow only their catalog-derived owned SCE workflows after the deny rule; the Code agent retains the synchronization-only `sce-decision` permission.
  - Validate: Inspect temporary `.opencode/agent/` output and assert the ordered `skill` entries for both agents; run the generated contract checks.
- [x] AC3: Claude generated commands continue to expose `Skill` where catalog metadata requires it and have no new SCE-specific permission allowlist; Pi receives the corrected workflow text and its generated extension is unchanged.
  - Validate: Inspect temporary `.claude/commands/`, `.claude/skills/`, `.pi/prompts/`, `.pi/skills/`, and `.pi/extensions/sce/index.ts`; compare the extension content to its canonical source and check Claude command frontmatter for `Skill`.
- [x] AC4: Existing workflow control-flow semantics and generated artifact shape remain unchanged apart from the intended wording and OpenCode permission changes.
  - Validate: Review the temporary generated diff and run `nix run .#pkl-check-generated` plus `nix flake check`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`
- `nix run .#pkl-generate -- "$(mktemp -d)"` followed by inspection of `.opencode/agent/`, `.opencode/skills/`, `.claude/commands/`, `.claude/skills/`, `.pi/prompts/`, and `.pi/skills/` in the temporary output.

### Context sync

- `context/overview.md` — current cross-target workflow orchestration and OpenCode routing/permission behavior.
- `context/architecture.md` — shared composite-rendering and target-boundary ownership.
- `context/patterns.md` — canonical helper-skill composition and OpenCode permission derivation rules.
- `context/glossary.md` — terminology for SCE workflow ownership versus non-SCE helper capability composition, if needed.
- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/sce/brownfield-workflow.md`
- `context/sce/handover-workflow.md`

## Constraints and non-goals

- **In scope:** Canonical Pkl workflow prose and shared composite preamble; catalog-derived OpenCode agent skill permissions; generated-output contract assertions needed to prevent regression; temporary generated-output inspection and related durable-context synchronization.
- **Out of scope:** Generated `.opencode`, `.claude`, and `.pi` trees; Rust CLI behavior; Claude permission restrictions; Pi extension policy enforcement, tracing, or runtime behavior; workflow phase order, gates, waits, stop conditions, write boundaries, validation logic, output layouts, command routing, or same-session resume semantics.
- **Constraints:** Edit canonical `config/pkl/` sources only; keep shared helper wording target-neutral and single-owned; derive explicit OpenCode SCE permissions from `workflow-catalog.pkl`; keep permission rule order with `skill: "*": allow`, `sce-*`: deny, then explicit allows; preserve Claude's catalog-derived `Skill` tool metadata and Pi's empty target-specific frontmatter.
- **Non-goal:** Do not weaken SCE determinism by allowing arbitrary SCE workflow chaining or by turning helper skills into workflow handoffs.

## Assumptions

- The request describes new work rather than an update to an existing plan; no matching plan exists under `context/plans/`.
- The existing `sce-decision` Code-agent permission remains the only non-catalog internal SCE exception, and its synchronization-only gate is unchanged.
- Existing generation-contract infrastructure is the appropriate place for cross-target wording and permission invariants; no new test framework is needed.

## Task stack

- [x] T01: `Scope canonical workflow orchestration rules and add helper composition` (status:done)
  - Implementation evidence: Scoped canonical workflow prohibitions to SCE skills/packages/commands across the composite renderer, workflow bodies, brownfield/handover rules, and context-sync rules; added the shared target-neutral helper composition rule preserving return-to-step, phase order, gates, waits, writes, validation, stops, and terminal output.
  - Task ID: T01
  - Goal: Update canonical workflow Purpose/Rules prose and the shared composite preamble so SCE skills forbid only SCE workflow chaining while relevant non-SCE helper skills are explicitly composable and must return control to the active workflow step without changing its control-flow or output invariants.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/workflow-composite.pkl`, `config/pkl/base/workflow-content.pkl`, the canonical workflow modules containing direct broad prohibitions (including change-to-plan, commit, brownfield, and handover), and any narrowly necessary source-level wording references. Out — OpenCode permission rendering, generated files, workflow behavior beyond the wording distinction, Claude/Pi runtime code, and durable context edits.
  - Dependencies: none
  - Done when: Every canonical Purpose/Rules prohibition is explicitly SCE-scoped; the shared composite preamble contains one target-neutral helper-skill composition rule covering return-to-step and preservation of step order, gates, waits, writes, validation, stops, and terminal output; package-mode brownfield and handover wording is aligned; no unrelated helper skill can reasonably be read as forbidden.
  - Verification notes (commands or checks): Focused search across `config/pkl/` for broad prohibition phrases; evaluate affected Pkl modules through the Nix dev shell; inspect the source diff for unchanged gates, waits, boundaries, and output contracts.
  - Verification evidence: Focused Pkl evaluations passed for the affected workflow sources and `workflow-composite.pkl`; temporary generation produced all 18 workflow `SKILL.md` files, with no unscoped prohibition matches and the helper-composition rule present in all 18; `git diff --check` passed.

- [x] T02: `Derive ordered OpenCode SCE skill permissions from the workflow catalog` (status:done)
  - Implementation evidence: Updated OpenCode agent skill permissions to allow non-SCE skills by default, deny arbitrary `sce-*` skills, and retain catalog-derived Plan/Code workflow allows after the deny rule, including the Code-only `sce-decision` exception.
  - Task ID: T02
  - Goal: Change OpenCode agent permission rendering to allow non-SCE skills by default, deny arbitrary `sce-*` skills, and place catalog-derived explicit workflow permissions after the deny rule for Plan and Code agents.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/opencode-metadata.pkl` and focused OpenCode metadata/rendering assertions if required to express the ordering. Out — manually maintained workflow inventories, Claude allowed-tool metadata, Pi rendering, OpenCode command routing, and any change to the valid owned-workflow set or `sce-decision` synchronization exception.
  - Dependencies: T01
  - Done when: Temporary generated Plan and Code agents contain `skill: "*": allow`, `sce-*`: deny, then only their catalog-derived explicit SCE allows; Plan allows only `sce-change-to-plan`, Code allows its catalog workflows plus `sce-decision`; normal non-SCE helper skills do not prompt and arbitrary SCE chaining remains denied.
  - Verification notes (commands or checks): Evaluate `opencode-metadata.pkl` and `opencode-content.pkl`; generate a temporary payload and inspect both `.opencode/agent/` files for exact permission order and entries.
  - Verification evidence: `nix develop -c pkl eval config/pkl/renderers/opencode-metadata.pkl` and `opencode-content.pkl` passed; temporary generation showed the exact ordered permissions for Plan and Code agents.

- [x] T03: `Guard and verify cross-target generated orchestration contracts` (status:done)
  - Implementation evidence: Added cross-target contract assertions for the shared helper-composition rule, SCE-scoped workflow prohibitions, and exact catalog-derived OpenCode skill permission ordering; updated the handover content guard to the scoped wording; added negative fixtures for each new failure mode and wired them into the generated-output check; corrected existing fixture failures to use the supported Pkl `throw` diagnostic path.
  - Task ID: T03
  - Goal: Extend canonical generation-contract checks to require the helper-skill composition rule and the corrected SCE-scoped wording across all generated workflow skills, and to enforce the ordered OpenCode permission invariant while retaining existing inventory and behavior guards.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/generation-contract-check.pkl`, its existing focused fixtures/check wiring only where needed, and temporary-output inspection scripts or commands used by the verification record. Out — changing workflow semantics, adding generated artifacts, modifying target runtimes/extensions, or introducing a second workflow catalog.
  - Dependencies: T01, T02
  - Done when: Generation fails for an unscoped skill prohibition, missing helper-composition rule, arbitrary SCE permission allowance, or wrong OpenCode rule order; generation still accepts the existing synchronization-only `sce-decision` exception and exact artifact inventory; all three target outputs are inspected from an explicit temporary root with no repository generated trees created.
  - Verification notes (commands or checks): Run `nix run .#pkl-check-generated`; run `nix run .#pkl-generate -- "$(mktemp -d)"` and inspect `.opencode/agent/`, `.opencode/skills/`, `.claude/commands/`, `.claude/skills/`, `.pi/prompts/`, `.pi/skills/`, and `.pi/extensions/sce/index.ts`; run `nix flake check`.
  - Verification evidence: `nix run .#pkl-check-generated` passed with the exact 101-file inventory and all new orchestration/permission contracts; all nine negative Pkl fixtures failed with their expected diagnostics; temporary generation inspected six helper-bearing workflow skills per target, two ordered OpenCode permission agents, six Claude commands with `Skill`, and a byte-identical Pi extension; `git diff --check` and `nix flake check` passed.

## Open questions

None. The requested ownership distinction, target behavior, permission ordering, source-of-truth boundary, and verification commands are explicit; remaining wording and assertion details are local implementation choices constrained by the existing renderer architecture.

## Validation Report

**Status:** validated  
**Date:** 2026-08-07

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation passed with the exact 101-file inventory and all generated-output contracts)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix run .#pkl-generate -- <temporary-dir>` followed by generated-output inspection -> exit 0 (all requested OpenCode, Claude, and Pi output directories passed inspection; 18 workflow skills had scoped helper wording, permissions were ordered exactly, all six Claude commands exposed `Skill`, and the Pi extension matched its canonical source)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Every generated SCE workflow skill scopes workflow-control prohibitions to SCE skills/packages/commands and explicitly permits relevant non-SCE helper skills while stating that helper use returns control to the active step without weakening its invariants; no generated workflow contains an unscoped `Never invoke another skill`-style prohibition. -> Temporary generation inspected all 18 workflow `SKILL.md` files; helper and return-to-step wording were present, with no unscoped prohibition.
- [x] AC2: Generated OpenCode Plan and Code agents allow ordinary non-SCE skills without prompting, deny arbitrary `sce-*` skills, and allow only their catalog-derived owned SCE workflows after the deny rule; the Code agent retains the synchronization-only `sce-decision` permission. -> Temporary Plan and Code agent output matched the exact ordered permission blocks.
- [x] AC3: Claude generated commands continue to expose `Skill` where catalog metadata requires it and have no new SCE-specific permission allowlist; Pi receives the corrected workflow text and its generated extension is unchanged. -> All six generated Claude commands exposed `Skill`; the generated Pi extension was byte-identical to `config/lib/pi-plugin/sce-pi-extension.ts`.
- [x] AC4: Existing workflow control-flow semantics and generated artifact shape remain unchanged apart from the intended wording and OpenCode permission changes. -> Temporary generated output inspection found the expected artifact shape and only the intended wording and OpenCode permission differences; both repository validation commands passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
