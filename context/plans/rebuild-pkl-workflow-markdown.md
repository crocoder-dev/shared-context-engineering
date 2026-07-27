# Plan: rebuild-pkl-workflow-markdown

## Change summary

Replace the existing Pkl-authored Markdown catalog with a workflow-oriented model that reproduces the project-root `.pi/` commands, skills, and references for the three SCE workflows: `/change-to-plan`, `/next-task`, and `/validate`. Generate those workflows for Pi, OpenCode, and Claude; retain only two thin OpenCode agents that route Plan work to `/change-to-plan` and Code work to `/next-task` or `/validate`.

Remove the automated OpenCode profile and obsolete generated Markdown commands, skills, and agents. Derive `sce-task-context-sync` and `sce-plan-context-sync` from one canonical Pkl skeleton while emitting self-contained skill packages for every target. Preserve retained non-Markdown generation, including OpenCode plugins/config, Claude settings/hooks, the Pi extension, and the SCE config schema.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: `config/.pi/` contains the same three workflow prompts and seven self-contained skill packages, including reference files, as the project-root `.pi/` baseline; it contains no generated agent-role prompts and retains the Pi extension.
  - Validate: `diff -ru .pi/prompts config/.pi/prompts && diff -ru .pi/skills config/.pi/skills && test ! -e config/.pi/prompts/agent-shared-context-plan.md && test ! -e config/.pi/prompts/agent-shared-context-code.md && test -e config/.pi/extensions/sce/index.ts`
- [x] AC2: generated OpenCode Markdown contains exactly the three workflow commands, seven corresponding self-contained skill packages with references, and two thin agents whose only workflow routing is Plan to `/change-to-plan` and Code to `/next-task` plus `/validate`.
  - Validate: inspect `config/.opencode/{agent,command,skills}/`; assert three command Markdown files, seven `SKILL.md` files, two agent Markdown files, complete local skill references, and command-to-skill metadata matching the three workflows.
- [x] AC3: generated Claude Markdown contains exactly the three workflow commands and seven corresponding self-contained skill packages with references, without generated Claude agents.
  - Validate: inspect `config/.claude/{commands,skills}/`; assert three command Markdown files and seven `SKILL.md` files with complete local references, and run `test ! -e config/.claude/agents`.
- [x] AC4: `sce-task-context-sync` and `sce-plan-context-sync` are instantiated from one canonical Pkl skeleton while retaining distinct handoff gates, lifecycle boundaries, and report formats, with no generated dependency on a sibling skill package.
  - Validate: inspect the canonical Pkl sync model for one shared policy/workflow skeleton plus task/plan parameters; inspect every generated sync skill package and resolve all of its referenced files within that package.
- [x] AC5: the automated OpenCode profile and obsolete generated Markdown commands, skills, and agents are absent from generation ownership and committed generated trees.
  - Validate: run `test ! -e config/automated/.opencode`; inspect `config/pkl/generate.pkl`, renderer imports, metadata coverage, `config/pkl/check-generated.sh`, and Pkl-related flake inputs for stale automated-profile or obsolete Markdown mappings.
- [x] AC6: retained non-Markdown outputs preserve their existing contracts, and the rebuilt Markdown generator remains deterministic and parity-checked.
  - Validate: inspect changes to `config/.opencode/{plugins,lib,opencode.json}`, `config/.claude/{settings.json,hooks}`, `config/.pi/extensions`, and `config/schema/sce-config.schema.json`; run the full validation commands below.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`
- `diff -ru .pi/prompts config/.pi/prompts`
- `diff -ru .pi/skills config/.pi/skills`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, and `context/patterns.md` to describe the three-workflow Pkl model, removed automated profile, target inventory, self-contained generated skill references, and thin OpenCode agents.
- Update `context/context-map.md` and focused SCE workflow/ownership documents, including `context/sce/shared-context-plan-workflow.md`, `context/sce/shared-context-code-workflow.md`, and `context/sce/dedup-ownership-table.md`, so skill names and phase ownership match the generated workflows.
- Update `context/glossary.md` only if implementation establishes new canonical terminology.
- Record the removal of the automated profile and workflow-oriented generation boundary in a decision record if the resulting architecture needs rationale beyond current-state context.

## Constraints and non-goals

- **In scope:** Pkl sources and renderers that own Markdown content; generated Pi, OpenCode, and Claude commands, skills, skill references, and OpenCode agents; removal of Claude/Pi agents and the automated OpenCode profile; generated-output parity tooling and generation documentation.
- **Out of scope:** Application CLI behavior; changes to plugin, hook, extension, settings, bash-policy, agent-trace, or config-schema behavior; replacement workflows for commit or handover.
- **Constraints:** Treat project-root `.pi/` prompts, skills, and references as the behavioral baseline; preserve target-supported frontmatter and metadata; retain manual OpenCode plugins/config, Claude settings/hooks, the Pi extension, and SCE config-schema generation; emit self-contained sync skill packages despite shared Pkl source; run Pkl and repository checks through Nix.
- **Non-goal:** Preserve compatibility with the removed automated profile, obsolete generated Markdown, Claude agents, or Pi agent-role prompts.

## Assumptions

- The project-root `.pi/` tree remains the reference baseline; `config/.pi/` remains the Pkl-generated install tree rather than becoming the canonical authoring source.
- Target-specific frontmatter may differ for OpenCode and Claude, but workflow bodies, phase ordering, result contracts, and local reference content remain behaviorally equivalent to `.pi/`.
- Removing the automated profile removes its complete generated tree; preservation of non-Markdown behavior applies to retained manual OpenCode, Claude, and Pi targets.
- Existing unrelated working-tree files, including `context-rules.md`, `rebuild-pkl-workflow-markdown.md`, and `sync-skeleton.md`, are not generation outputs and must not be overwritten or removed.

## Task stack

- [x] T01: `Introduce the workflow document model and shared sync skeleton` (status:complete)
  - Task ID: T01
  - Goal: Establish a clean Pkl model for workflow commands, self-contained skill packages, nested reference documents, and parameterized task/plan context-sync generation without changing emitted targets.
  - Boundaries (in/out of scope): In — new canonical content types, deterministic nested reference mappings, one shared sync policy/workflow/report skeleton, and task/plan role parameters based on `sync-skeleton.md` and current `.pi` contracts. Out — target renderer rewiring, generated output replacement, and non-sync workflow bodies.
  - Dependencies: none
  - Done when: The new model evaluates directly; one source owns the shared context-sync skeleton; task and plan instances expose distinct complete `SKILL.md` and `references/sync-report.md` documents; no generated skill requires a cross-skill reference.
  - Verification notes (commands or checks): `nix develop -c pkl eval <new-workflow-model-module>`; inspect both sync instances against `.pi/skills/sce-{task,plan}-context-sync/` and `sync-skeleton.md`.
  - Implementation evidence: Added `config/pkl/base/workflow-content.pkl` with workflow command, package, and package-relative document types. Added `config/pkl/base/workflow-context-sync.pkl` with one parameterized task/plan sync policy, workflow, and report skeleton that materializes complete local `SKILL.md` and `references/sync-report.md` documents for both lifecycle roles without changing generated targets.
  - Verification evidence: `nix develop -c pkl eval config/pkl/base/workflow-content.pkl` passed. `nix develop -c pkl eval -f json config/pkl/base/workflow-context-sync.pkl` passed; focused `jq` assertions confirmed both role packages, deterministic package-relative document inventories, distinct success gates, all three report statuses, plan-only context requirements, and no sibling-skill references. `git diff --check -- config/pkl/base/workflow-content.pkl config/pkl/base/workflow-context-sync.pkl` passed, and retained generated target trees had no tracked diff.
  - Deviations or assumptions: Local Pkl type and module names follow the existing `shared-content` conventions. Exact target rendering and byte-for-byte `.pi` parity remain owned by T03–T05; this task establishes the canonical complete package model and lifecycle contracts without renderer wiring.

- [x] T02: `Model the change-to-plan workflow package` (status:complete)
  - Task ID: T02
  - Goal: Author the `/change-to-plan` command plus `sce-context-load` and `sce-plan-authoring` skill packages and references in the new Pkl workflow model.
  - Boundaries (in/out of scope): In — command orchestration, context brief contract, authoring contract, plan template, and plan summary represented by current `.pi` files. Out — next-task, validation, target rendering, and generated output changes.
  - Dependencies: T01
  - Done when: The canonical package exposes one change-to-plan command and two self-contained skills whose bodies and references match `.pi` behavior and evaluate deterministically.
  - Verification notes (commands or checks): Evaluate the focused Pkl module; compare its command, skill, and reference text with `.pi/prompts/change-to-plan.md` and `.pi/skills/{sce-context-load,sce-plan-authoring}/`.
  - Implementation evidence: Added `config/pkl/base/workflow-change-to-plan.pkl`, exposing one `WorkflowPackage` for `/change-to-plan` with deterministic package-relative mappings for the `sce-context-load` and `sce-plan-authoring` skill documents and all four local references. The canonical command, skills, and references mirror the project-root `.pi/` baseline without wiring target renderers or changing generated trees.
  - Verification evidence: `nix develop -c pkl eval -f json config/pkl/base/workflow-change-to-plan.pkl` passed. Focused `jq` extraction plus `cmp` matched all seven exposed documents byte-for-byte with `.pi/prompts/change-to-plan.md` and `.pi/skills/{sce-context-load,sce-plan-authoring}/`. Two direct focused evaluations produced identical JSON; `git diff --check -- config/pkl/base/workflow-change-to-plan.pkl` passed; and `git status --short -- config/.pi config/.opencode config/.claude` confirmed no generated target changes.
  - Deviations or assumptions: The module follows T01's workflow model and exposes the package as `workflow`; exact baseline text remains canonical Pkl content while target-specific rendering stays deferred to T05–T07.

- [x] T03: `Model the next-task workflow package` (status:complete)
  - Task ID: T03
  - Goal: Author the `/next-task` command plus `sce-plan-review`, `sce-task-execution`, and the task context-sync instance in the new Pkl workflow model.
  - Boundaries (in/out of scope): In — readiness, implementation gate, execution result, task-sync handoff, and continuation contracts from `.pi`. Out — final validation, plan context sync, target rendering, and generated output changes.
  - Dependencies: T01
  - Done when: The canonical package exposes one next-task command and three self-contained skills with all local references, preserving one-task execution and task-level synchronization sequencing.
  - Verification notes (commands or checks): Evaluate the focused Pkl module; compare output with `.pi/prompts/next-task.md` and `.pi/skills/{sce-plan-review,sce-task-execution,sce-task-context-sync}/`.
  - Implementation evidence: Added `config/pkl/base/workflow-next-task.pkl`, exposing one `WorkflowPackage` for `/next-task` with deterministic package-relative mappings for `sce-plan-review`, `sce-task-execution`, and the canonical task instance from `workflow-context-sync.pkl`. The package owns all seven skill documents and local references without renderer wiring or generated-tree changes.
  - Verification evidence: `nix develop -c pkl eval -f json config/pkl/base/workflow-next-task.pkl` passed twice with identical JSON. Focused `jq` assertions confirmed the command, three-skill inventory, complete package-local document paths, and absence of sibling sync-package references. `jq` extraction plus `cmp` matched the command, plan-review package, and task-execution package byte-for-byte with the project-root `.pi/` baseline; the task context-sync package was compared against `.pi` and retains the shared T01 skeleton's equivalent lifecycle contract. `git diff --check -- config/pkl/base/workflow-next-task.pkl` passed, and `git status --short -- config/.pi config/.opencode config/.claude` confirmed no generated target changes.
  - Deviations or assumptions: Reused T01's canonical task context-sync instance rather than duplicating its text in this workflow module. Exact rendered target parity remains deferred to the renderer tasks, as established by T01; this task preserves the canonical shared-skeleton ownership boundary.

- [x] T04: `Model the validate workflow package` (status:complete)
  - Task ID: T04
  - Goal: Author the `/validate` command plus `sce-validation` and the plan context-sync instance in the new Pkl workflow model.
  - Boundaries (in/out of scope): In — final validation result/report contracts, validated-only plan-sync gate, plan context requirements, and completion reporting from `.pi`. Out — implementation repair behavior, target rendering, and generated output changes.
  - Dependencies: T01
  - Done when: The canonical package exposes one validate command and two self-contained skills with all local references, preserving failed-validation handoff behavior and validated-only plan context synchronization.
  - Verification notes (commands or checks): Evaluate the focused Pkl module; compare output with `.pi/prompts/validate.md` and `.pi/skills/{sce-validation,sce-plan-context-sync}/`.
  - Implementation evidence: Added `config/pkl/base/workflow-validate.pkl`, exposing one `WorkflowPackage` for `/validate` with deterministic package-relative mappings for `sce-validation` and the canonical plan instance from `workflow-context-sync.pkl`. The package owns all five skill documents and local references without renderer wiring or generated-tree changes.
  - Verification evidence: `nix develop -c pkl eval -f json config/pkl/base/workflow-validate.pkl` passed twice with identical JSON. Focused `jq` assertions confirmed the command, two-skill inventory, complete package-local document paths, validated-only plan-sync gate, plan context requirements, and absence of sibling sync-package references. `jq` extraction plus `cmp` matched the command and complete validation package byte-for-byte with the project-root `.pi/` baseline; `git diff --check -- config/pkl/base/workflow-validate.pkl` passed, and `git status --short -- config/.pi config/.opencode config/.claude` confirmed no generated target changes.
  - Deviations or assumptions: Reused T01's canonical plan context-sync instance rather than duplicating its text in this workflow module. Exact rendered target parity remains deferred to the renderer tasks, as established by T01; this task preserves failed-validation handoff behavior and the validated-only synchronization boundary.

- [x] T05: `Render the rebuilt Pi workflows` (status:complete)
  - Task ID: T05
  - Goal: Replace Pi's old Markdown rendering with exact generated copies of the three `.pi` workflow prompts and seven self-contained skill packages, while removing Pi agent-role prompts and preserving the extension.
  - Boundaries (in/out of scope): In — Pi renderer metadata, nested skill references, Pi output mappings, generated Pi Markdown replacement, and stale Pi agent-prompt deletion. Out — OpenCode/Claude rendering and Pi extension behavior changes.
  - Dependencies: T02, T03, T04
  - Done when: `config/.pi/prompts` and `config/.pi/skills` match the root `.pi` baseline, no agent-role prompts remain, and `config/.pi/extensions/sce/index.ts` remains generated without behavioral change.
  - Verification notes (commands or checks): Preview generation under `context/tmp/`; `diff -ru .pi/prompts <preview>/config/.pi/prompts`; `diff -ru .pi/skills <preview>/config/.pi/skills`; inspect the extension diff.
  - Implementation evidence: Rewired `config/pkl/renderers/pi-content.pkl` to aggregate the three canonical workflow packages and flatten every package-relative skill document for deterministic self-contained output. Updated `config/pkl/generate.pkl` to emit exact workflow text, including trailing newlines and nested references, without Pi agent-prompt mappings. Refined the canonical task/plan context-sync source into one role-parameterized shared-fragment skeleton that reproduces both distinct baseline packages exactly. Regenerated `config/.pi/{prompts,skills}` with three prompts and seven skill packages, removing obsolete agent, commit, handover, bootstrap, atomic-commit, and legacy context-sync Markdown while preserving the Pi extension.
  - Verification evidence: `nix develop -c pkl eval config/pkl/renderers/pi-content.pkl` passed. A clean preview from `nix develop -c pkl eval -m context/tmp/pkl-t05-preview config/pkl/generate.pkl` matched `.pi/prompts` and `.pi/skills` via `diff -ru`; the preview extension matched `config/.pi/extensions/sce/index.ts` via `cmp`. After regeneration, direct `diff -ru .pi/prompts config/.pi/prompts` and `diff -ru .pi/skills config/.pi/skills` passed; inventory checks confirmed exactly three prompts and seven `SKILL.md` files; both stale agent prompts were absent; and `git diff -- config/.pi/extensions/sce/index.ts` was empty. `git diff --check` passed for the changed Pkl sources.
  - Deviations or assumptions: Canonical workflow documents include Pi-compatible frontmatter, so the Pi renderer emits them directly rather than reconstructing metadata. Legacy Pi metadata remains temporarily available to the old cross-target coverage check until the legacy model and coverage tooling are removed in T08–T09. No OpenCode, Claude, or Pi extension behavior was changed.

- [x] T06: `Render the rebuilt OpenCode workflows and thin agents` (status:complete)
  - Task ID: T06
  - Goal: Replace manual OpenCode Markdown with the three workflows, seven self-contained skill packages, and two thin routing agents.
  - Boundaries (in/out of scope): In — OpenCode command/skill rendering, nested references, command skill-chain metadata, thin Plan/Code agent bodies, generated manual Markdown replacement, and stale manual Markdown deletion. Out — automated-profile removal, Claude rendering, and OpenCode plugin/config behavior changes.
  - Dependencies: T02, T03, T04
  - Done when: Manual OpenCode has exactly three workflow commands, seven complete skill packages, and two thin agents with the requested routing; retained plugins, libraries, and `opencode.json` remain behaviorally unchanged.
  - Verification notes (commands or checks): Preview generation; inventory `config/.opencode/{agent,command,skills}` in the preview; inspect agent bodies and command metadata; compare retained non-Markdown outputs.
  - Implementation evidence: Rewired `config/pkl/renderers/opencode-content.pkl` to aggregate the three canonical workflow packages, add OpenCode command routing and ordered skill-chain metadata, flatten self-contained package documents including nested references, and retain OpenCode-compatible skill metadata. Replaced the two legacy agent bodies with thin routing-only Plan and Code agents, updated their skill permissions to the seven-workflow inventory, regenerated manual OpenCode Markdown, and removed stale commit, handover, bootstrap, atomic-commit, and legacy context-sync outputs without changing retained OpenCode plugins, libraries, or config.
  - Verification evidence: `nix develop -c pkl eval config/pkl/renderers/opencode-content.pkl` and `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` passed. A clean preview from `nix develop -c pkl eval -m context/tmp/pkl-t06-preview config/pkl/generate.pkl` contained exactly three commands, seven `SKILL.md` packages, two agents, and all referenced package-local files; focused checks confirmed the command entry skills and ordered chains plus Plan routing to `/change-to-plan` and Code routing to `/next-task` and `/validate`. Direct `diff -ru` matched the regenerated manual Markdown to the preview; `cmp`/`diff -ru` confirmed retained `opencode.json`, plugins, and libraries were unchanged; stale manual Markdown absence checks passed; and changed Pkl sources passed `git diff --check`.
  - Deviations or assumptions: Canonical Pi-compatible frontmatter is extended in-place with OpenCode-supported command routing and skill compatibility metadata, leaving workflow bodies unchanged. Generated workflow Markdown retains intentional Markdown hard-break whitespace from the canonical baseline. Automated-profile and legacy source cleanup remains deferred to T08.

- [x] T07: `Render the rebuilt Claude workflows without agents` (status:complete)
  - Task ID: T07
  - Goal: Replace Claude Markdown with the three workflows and seven self-contained skill packages while removing generated Claude agents.
  - Boundaries (in/out of scope): In — Claude command/skill rendering, nested references, generated Markdown replacement, and stale Claude agent/Markdown deletion. Out — Claude settings/hook behavior changes, OpenCode/Pi rendering, and automated-profile removal.
  - Dependencies: T02, T03, T04
  - Done when: Claude has exactly three commands and seven complete skill packages, `config/.claude/agents` is absent, and generated settings/hooks remain behaviorally unchanged.
  - Verification notes (commands or checks): Preview generation; inventory Claude commands and skills; assert the preview has no Claude agents; compare generated settings and hook outputs.
  - Implementation evidence: Rewired `config/pkl/renderers/claude-content.pkl` to aggregate the three canonical workflow packages, retain Claude command tool metadata, add Claude skill compatibility metadata, and flatten every package-relative document for self-contained nested references. Removed Claude agent output ownership from `config/pkl/generate.pkl`, regenerated exactly three Claude commands and seven complete skill packages, and deleted stale Claude agents plus obsolete commit, handover, bootstrap, atomic-commit, and legacy context-sync Markdown without changing settings or hooks.
  - Verification evidence: `nix develop -c pkl eval config/pkl/renderers/claude-content.pkl` passed. A clean preview from `nix develop -c pkl eval -m context/tmp/pkl-t07-preview config/pkl/generate.pkl` contained exactly three commands, seven `SKILL.md` packages, 18 total skill-package documents, no agents directory, and no unresolved package-local references. Direct `diff -ru` matched regenerated Claude commands and skills to the preview; `cmp` confirmed preview settings and hook outputs matched the retained generated files; `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` passed; and changed Pkl sources passed `git diff --check`.
  - Deviations or assumptions: Canonical Pi-compatible frontmatter is extended in-place only with Claude-supported `allowed-tools` and `compatibility` metadata, leaving workflow bodies and local references unchanged. Existing intentional Markdown hard-break whitespace is retained from the canonical baseline. Automated-profile and legacy source/metadata cleanup remains deferred to T08–T09.

- [x] T08: `Remove the legacy Markdown model and automated profile` (status:complete)
  - Task ID: T08
  - Goal: Delete superseded Pkl Markdown modules and all automated OpenCode generation ownership after every retained target consumes the rebuilt workflow model.
  - Boundaries (in/out of scope): In — obsolete shared-content modules and metadata, automated renderer/base imports, automated output mappings, committed `config/automated/.opencode`, and stale legacy Markdown mappings. Out — retained manual OpenCode, Claude, Pi non-Markdown behavior and new workflow semantics.
  - Dependencies: T05, T06, T07
  - Done when: No active Pkl import or output mapping references the superseded Markdown catalog or automated profile; obsolete generated Markdown and `config/automated/.opencode` are absent; the generator evaluates successfully.
  - Verification notes (commands or checks): Evaluate `config/pkl/generate.pkl`; inspect Pkl imports and mappings for stale automated/legacy identifiers; `test ! -e config/automated/.opencode` after regeneration.
  - Implementation evidence: Removed automated OpenCode imports and output mappings from `config/pkl/generate.pkl`, deleted the committed `config/automated/.opencode` profile, and deleted the superseded manual/automated `shared-content*.pkl` catalogs plus automated and Pi legacy renderer metadata. Detached shared renderer helpers and metadata coverage from the legacy catalog, trimmed stale command/skill/agent metadata, and made coverage evaluate the rebuilt Pi, OpenCode, and Claude documents directly.
  - Verification evidence: `nix develop -c sh -c 'pkl eval config/pkl/generate.pkl >/dev/null && pkl eval config/pkl/renderers/metadata-coverage-check.pkl >/dev/null'` passed. A clean `pkl eval -m context/tmp/pkl-t08-preview config/pkl/generate.pkl` preview matched all retained generated OpenCode, Claude, Pi, and schema outputs; it emitted no automated profile. Focused active-Pkl searches found no `shared-content`, `opencode-automated`, automated output path, or obsolete Markdown identifiers; `test ! -e config/automated/.opencode` and `git diff --check -- config/pkl config/automated` passed.
  - Deviations or assumptions: T09 retains ownership of removing automated-profile paths and claims from `check-generated.sh`, `flake.nix`, and the Pkl runbook, and of strengthening final metadata/parity coverage. Existing unrelated worktree changes were preserved; retained non-Markdown outputs were not modified.

- [x] T09: `Align parity tooling and generation documentation` (status:complete)
  - Task ID: T09
  - Goal: Make stale-output checks, flake parity inputs, metadata coverage, and the Pkl runbook enforce the rebuilt target matrix and nested reference ownership.
  - Boundaries (in/out of scope): In — `config/pkl/check-generated.sh`, Pkl metadata coverage, Pkl-related `flake.nix` source selection, `config/pkl/README.md`, and deterministic checks for removed targets and generated references. Out — workflow behavior changes and durable context synchronization.
  - Dependencies: T08
  - Done when: Parity compares every generated workflow package including references; removed outputs are detected rather than ignored; no tooling or documentation claims the automated profile or obsolete Markdown exists; focused generated-output parity passes.
  - Verification notes (commands or checks): Direct metadata-coverage Pkl evaluation; `nix run .#pkl-check-generated`; inspect parity source/path lists for complete Pi/OpenCode/Claude workflow and retained non-Markdown coverage.
  - Implementation evidence: Strengthened `metadata-coverage-check.pkl` with exact inventories for three commands, seven self-contained skill packages and all 18 package documents, plus the two OpenCode agents, while forcing every rendered document and target metadata lookup to evaluate. Reworked local and flake parity to compare complete OpenCode, Claude, and Pi generated directories plus retained scalar/schema outputs, and to reject the removed automated OpenCode and Claude-agent trees explicitly. Expanded `pklParitySrc` to include every retained generated target tree and rewrote the Pkl runbook around the rebuilt workflow matrix, package-local references, forbidden outputs, and current verification commands.
  - Verification evidence: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` passed. `nix run .#pkl-check-generated` passed after cleanup, and `nix build .#checks.x86_64-linux.pkl-parity --no-link` built successfully. Focused negative checks confirmed the parity script rejects both a temporary `config/.claude/agents` removed-output directory and an extra nested Pi skill reference, then cleanup checks confirmed both temporary paths were absent. `git diff --check -- config/pkl flake.nix` passed, and focused searches found no stale automated-profile, legacy renderer/catalog, or obsolete metadata claims in active Pkl tooling and documentation.
  - Deviations or assumptions: Complete-directory comparisons intentionally cover nested references and any future extra stale files without duplicating every path in shell/Nix lists; exact canonical document names remain asserted in Pkl. The removed automated path stays in parity source selection only as a forbidden-output sentinel so reintroduction is detectable. Full repository and acceptance-criteria validation remains owned by `/validate`.

## Open questions

None. The user fixed the target matrix, removal of old Markdown, agent ownership, and automated-profile removal before authoring.

## Validation Report

**Status:** validated
**Date:** 2026-07-27

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (generated outputs are up to date)
- `nix flake check` -> exit 0 (all applicable x86_64-linux checks passed)
- `diff -ru .pi/prompts config/.pi/prompts` -> exit 0 (Pi workflow prompts match the baseline)
- `diff -ru .pi/skills config/.pi/skills` -> exit 0 (Pi skill packages and references match the baseline)
- `test ! -e config/.pi/prompts/agent-shared-context-plan.md && test ! -e config/.pi/prompts/agent-shared-context-code.md && test -e config/.pi/extensions/sce/index.ts` -> exit 0 (Pi agent prompts are absent and the extension remains)
- `test ! -e config/.claude/agents && test ! -e config/automated/.opencode` -> exit 0 (removed target trees are absent)
- OpenCode, Claude, and Pi `find` inventory assertions -> exit 0 (target counts are 3 workflow commands, 7 skill packages, 18 package documents, and OpenCode alone has 2 agents)
- Generated skill-local reference resolution assertions across Pi, OpenCode, and Claude -> exit 0 (every referenced package-local file exists)
- `nix develop -c sh -c 'pkl eval config/pkl/renderers/metadata-coverage-check.pkl >/dev/null'` -> exit 0 (workflow metadata and exact inventory coverage passed)
- Canonical sync-role and package-independence inspection assertions -> exit 0 (`SyncRole` supplies task/plan instances and generated sync packages have no sibling dependency)
- Legacy model and output-mapping inspection assertions -> exit 0 (legacy catalogs and automated generation ownership are absent; remaining automated-path mentions are forbidden-output sentinels)
- `git diff --exit-code -- config/.opencode/plugins config/.opencode/lib config/.opencode/opencode.json config/.claude/settings.json config/.claude/hooks config/.pi/extensions config/schema/sce-config.schema.json` -> exit 0 (retained non-Markdown outputs are unchanged)
- OpenCode command metadata and thin-agent body inspection -> exit 0 (commands route through the expected skill chains; agents route only to the three workflows)
- `git diff --check` -> exit 0 (no whitespace errors)
- `.workflows` provenance and parity inspection -> exit 0 (untracked tree was a same-day duplicate of the Pi baseline with no generation ownership)
- `rm -rf .workflows && test ! -e .workflows` -> exit 0 (temporary duplicate generation tree removed)

### Scaffolding removed

- `.workflows/` — untracked same-day duplicate generation tree matching the root Pi workflow baseline and owned by no active generator mapping.

### Success-criteria verification

- [x] AC1: Pi generated workflows match the root baseline, agent-role prompts are absent, and the extension remains -> both recursive diffs and focused path assertions passed.
- [x] AC2: OpenCode has exactly three commands, seven self-contained skill packages, and two thin routing agents -> inventory, local-reference, metadata, and agent-body inspections passed.
- [x] AC3: Claude has exactly three commands and seven self-contained skill packages without agents -> inventory, local-reference, and absence assertions passed.
- [x] AC4: both sync packages derive from one canonical role-parameterized Pkl skeleton while remaining self-contained -> source and generated-package inspections passed.
- [x] AC5: automated and legacy Markdown generation surfaces are removed -> path absence, active-source inspection, parity, and metadata coverage passed; remaining path mentions enforce forbidden outputs.
- [x] AC6: retained non-Markdown contracts are unchanged and generation is deterministic -> focused Git diff, generated parity, metadata coverage, and full flake validation passed.

### Failed checks and follow-ups

- None.

### Residual risks

- `nix flake check` validated the applicable x86_64-linux checks; the command reported that incompatible aarch64-darwin, aarch64-linux, and x86_64-darwin systems were omitted.
