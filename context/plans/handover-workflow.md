# Plan: handover-workflow

## Change summary

Add `/handover` as the fifth canonical SCE workflow across OpenCode, Claude, and Pi. The workflow has two argument-selected modes: `/handover` inspects the current session and repository state and writes a structured transition document under `context/handovers/`; `/handover context/handovers/<file>.md` loads an existing handover for continuation in another session.

This restores a previously removed workflow surface using the current thin-command, self-contained-skill, cross-target Pkl architecture. It does not add a new routing agent or automate implementation after a handover is loaded.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Running `/handover` with no arguments creates one Markdown file under `context/handovers/` containing populated `Current Task State`, `Decisions Made`, `Open Questions / Blockers`, and `Next Recommended Step` sections, labels inferred details as assumptions, and reports the written path.
  - Validate: inspect the generated Pi `/handover` prompt, `sce-handover` skill, output reference, and persisted-document template; confirm the writer branch owns context gathering, task-aligned naming, completeness checks, assumption labeling, one-file writing, and written-path output.
- [x] AC2: Running `/handover context/handovers/<file>.md` reads the named handover, validates that it is a handover file with all four required sections, presents it for continuation, and performs no implementation or repository edits.
  - Validate: inspect the generated Pi workflow for exact argument-based writer/loader routing, path and document validation, loaded output, and read-only loader boundaries.
- [x] AC3: OpenCode, Claude, and Pi generated payloads each expose a thin `/handover` command or prompt routed to exactly one self-contained `sce-handover` package containing only `SKILL.md` and `references/output.md`, without generating a new agent or phase-skill package.
  - Validate: generate a temporary payload with `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect the three target inventories, command-to-skill routes, package contents, and unchanged agent inventory.
- [x] AC4: Canonical generation and metadata coverage enforce the expanded five-workflow inventory, the exact generated artifact count, required handover behavior, and deterministic output.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`; `nix run .#pkl-check-generated`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Add a focused current-state handover workflow document under `context/sce/` and link it from `context/context-map.md`.
- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` for the five-workflow cross-target inventory, dual writer/loader contract, and `sce-handover` terminology.
- Record or reuse a superseding ADR for restoring `/handover` after `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` removed that surface.

## Constraints and non-goals

- **In scope:** Project-root Pi behavioral baseline; canonical Pkl handover workflow source; typed workflow catalog and shared composite wiring; OpenCode, Claude, and Pi command/prompt and skill-package generation; exact inventory/content checks; workflow generation documentation; task-transition documents under `context/handovers/`.
- **Out of scope:** A separate reader command or skill, a new OpenCode agent, Rust CLI behavior, automatic implementation after loading, cloud/session persistence, and changes to the existing planning, implementation, validation, or commit workflows.
- **Constraints:** `/handover` with no arguments is writer mode; one path argument is loader mode; commands remain thin and invoke exactly one workflow skill; the skill owns all routing, waits, file operations, validation, and output; generated target layouts remain ephemeral and canonical behavior remains Pkl-owned with the project-root `.pi/` baseline kept aligned.
- **Non-goal:** Generalize handovers into a session database, task scheduler, or automatic conversational-state restoration mechanism.

## Assumptions

- The loader argument must resolve to a Markdown file under `context/handovers/`; arbitrary repository files are rejected rather than treated as handovers.
- Writer mode uses `context/handovers/{plan_name}-{task_id}.md` when one active plan task is unambiguous, and a collision-safe timestamped `context/handovers/handover-{YYYY-MM-DD-HHMMSS}.md` fallback otherwise.
- Loading means presenting the stored task state and continuation guidance in the current session; it does not mark plan tasks complete, edit files, or begin the recommended step.
- The OpenCode command routes through the existing Shared Context Code agent because handover is an operational task-continuity workflow; no third routing role is introduced.

## Task stack

- [x] T01: `Define the handover workflow baseline` (status:done)
  - Task ID: T01
  - Goal: Add the project-root Pi `/handover` prompt and self-contained `sce-handover` package with complete writer and loader behavior.
  - Boundaries (in/out of scope): In — `.pi/prompts/handover.md`, `.pi/skills/sce-handover/SKILL.md`, `.pi/skills/sce-handover/references/output.md`, argument routing, the persisted handover template, writer completeness checks, loader validation, and terminal layouts. Out — canonical Pkl, other targets, renderer inventories, durable context synchronization, and implementation of any loaded next step.
  - Dependencies: none
  - Done when: Empty arguments select writer mode; one handover path selects read-only loader mode; writer mode gathers session/repository facts, labels assumptions, writes exactly one complete handover, and reports its path; loader mode validates and presents one complete handover without edits; blocked cases and all user-visible output have package-local layouts.
  - Verification notes (commands or checks): `git diff --check -- .pi/prompts/handover.md .pi/skills/sce-handover`; inspect both branches against AC1 and AC2 and confirm the package contains only `SKILL.md` and `references/output.md`.
  - Evidence: Added `.pi/prompts/handover.md` (thin command routing `$ARGUMENTS` to `sce-handover`) and `.pi/skills/sce-handover/{SKILL.md,references/output.md}`. `SKILL.md` parses empty args as writer mode and one path argument as loader mode; writer mode gathers session/repo facts, labels assumptions, names the file via the active-task or timestamped-fallback rule, embeds the four-section persisted-document template, and never overwrites an existing handover; loader mode validates the path is under `context/handovers/`, is Markdown, and has all four required sections before presenting it read-only. All terminal layouts (invalid usage, writer/loader blocked, writer/loader success) live in `references/output.md`. Package contains only `SKILL.md` and `references/output.md`.
  - Verification: `git diff --check -- .pi/prompts/handover.md .pi/skills/sce-handover` -> exit 0 (no whitespace conflicts); `find .pi/skills/sce-handover -type f` -> confirmed exactly `SKILL.md` and `references/output.md`; manual inspection of both branches against AC1 (writer produces `Current Task State`, `Decisions Made`, `Open Questions / Blockers`, `Next Recommended Step`, labels assumptions, writes one file, reports path) and AC2 (loader validates path/sections, presents for continuation, performs no edits).
  - Assumptions: "Active task" for the writer's naming rule is determined from conversation context plus `context/plans/*.md`, not a separate state file, since none exists in this repository.

- [x] T02: `Model the canonical handover workflow` (status:done)
  - Task ID: T02
  - Goal: Represent the complete handover baseline as a structured canonical Pkl workflow source without changing generated target inventories yet.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-handover.pkl` and only minimal shared workflow-content primitives required to model its command, dual-mode behavior, persisted-document template, and output layouts. Out — workflow catalog expansion, composite renderer registration, target emission, inventory counts, and edits to existing workflow semantics.
  - Dependencies: T01
  - Done when: The focused Pkl module evaluates deterministically, exposes one `handover` command plus one self-contained `sce-handover` structured composite source, and renders package-mode Pi documents byte-identically to the T01 baseline without sibling-package references.
  - Verification notes (commands or checks): `nix develop -c pkl eval -f json config/pkl/base/workflow-handover.pkl`; compare its rendered command, skill, and output reference with `.pi/prompts/handover.md` and `.pi/skills/sce-handover/`; `git diff --check -- config/pkl/base/workflow-handover.pkl`.
  - Evidence: Added `config/pkl/base/workflow-handover.pkl`, following the established `workflow-content.pkl` model used by `workflow-commit.pkl`/`workflow-next-task.pkl`. It defines a mode-invariant thin `handover.md` command (`model.WorkflowCommand`), a `structuredCommand` (`StructuredWorkflowDocument`) whose package-mode render assembles the full self-contained `sce-handover` `SKILL.md` (frontmatter, title, Purpose, User-visible output, Input, Workflow with Writer/Loader paths, Rules, and the inlined persisted-document-format section) and whose composite-mode render drops frontmatter/title/Purpose/User-visible-output (left for a future composite renderer to supply generically) and the persisted-document section, exposing only `## Input`/`## Workflow`/`## Rules`. The persisted-document template is a shared `renderPersistedFormatBody` constant, inlined for package mode and exposed as one `structuredComposite.internalDocuments` entry (`Handover document`) for composite mode, matching the `## Internal persisted-document format: {path}` convention `workflow-composite.pkl` uses elsewhere. `references/output.md` content is mode-invariant and exposed both as a package document and as the sole `structuredComposite.outputDocuments` entry. `workflow.skills` registers exactly one `sce-handover` `SkillPackage`, matching the "no sibling packages" constraint.
  - Verification: `nix develop -c pkl eval -f json config/pkl/base/workflow-handover.pkl` → evaluates cleanly; re-ran and diffed the two JSON outputs → byte-identical (deterministic). Extracted `workflow.command.document.text`, `workflow.skills["sce-handover"].documents["SKILL.md"].text`, and `documents["references/output.md"].text` and diffed each against `.pi/prompts/handover.md`, `.pi/skills/sce-handover/SKILL.md`, and `.pi/skills/sce-handover/references/output.md` respectively → identical apart from the expected single trailing newline `generate.pkl` appends at write time (`text = "\(document.text)\n"`), the same convention every sibling workflow file relies on. `git diff --check -- config/pkl/base/workflow-handover.pkl` → exit 0. `nix run .#pkl-check-generated` → passed, 52 files (unchanged from before this task), confirming the new unwired module doesn't disturb existing generation. Directly evaluated `structuredComposite.command.render.apply("composite","").text` and confirmed it starts at `## Input` and ends after `## Rules` with no frontmatter/Purpose text, and `structuredComposite.internalDocuments[0].path == "Handover document"` with the expected Layout/Rules body — confirming composite mode is ready for T03's renderer wiring without sibling-package references.
  - Notes: Catalog registration, composite-renderer wiring (`workflow-composite.pkl`), and target emission are explicitly deferred to T03 per this task's scope.

- [x] T03: `Render and enforce the fifth workflow` (status:done)
  - Task ID: T03
  - Goal: Register the handover workflow in the typed catalog and shared composite renderer, emit it for all three targets, and update exact generation contracts for the expanded matrix.
  - Boundaries (in/out of scope): In — `workflow-catalog.pkl`, `workflow-composite.pkl`, target routing/tool metadata, exact metadata and generation-contract checks, controlled fixtures or required-token assertions, generation documentation/counts, and synchronization of repository-root OpenCode/Claude/Pi workflow surfaces when expected by the existing generated-config workflow. Out — changing T01/T02 handover semantics, adding an agent, Rust CLI changes, and unrelated generated assets.
  - Dependencies: T02
  - Done when: Temporary generation emits `/handover` and `sce-handover/{SKILL.md,references/output.md}` for OpenCode, Claude, and Pi; every command invokes exactly `sce-handover`; OpenCode uses the existing Code role; target tools permit required read/write operations; exact inventory and artifact counts include five workflows; required-content checks cover both handover modes; existing workflow outputs and agent inventory remain unchanged.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/{pi-content,opencode-content,claude-content,metadata-coverage-check,generation-contract-check}.pkl`; generate to two temporary roots, compare them for byte identity, compare generated Pi handover files with the root baseline, inspect OpenCode/Claude metadata, and confirm no new agent or phase package appears.
  - Evidence: Added a `handover` `WorkflowRecord` to `config/pkl/base/workflow-catalog.pkl` (`shared-context-code` OpenCode role; Claude tools `Read, Glob, Grep, Write, Skill, Bash` — no `Task`/`Edit`/`Question`, matching handover's no-delegation, no-edit-of-existing-files, no-mid-run-wait contract). Wired `workflow-handover.pkl` into `config/pkl/renderers/workflow-composite.pkl` as a `handoverWorkflow` `CompositeWorkflow` registered in the shared `workflows` mapping; since handover's base module already exposes one complete, mode-invariant output reference as its sole `outputDocuments` entry, its `outputText` needs no additional composite-only header, unlike the four phase-based workflows. Content/metadata renderers (`pi-content.pkl`, `opencode-content.pkl`, `claude-content.pkl`, `opencode-metadata.pkl`, `claude-metadata.pkl`) and both check files are fully catalog/mapping-driven, so no further renderer edits were needed; updated three "four workflow packages" doc comments to "five". Added an `assertHandoverContent` check to `generation-contract-check.pkl` (`handover-package-content`) asserting the generated `sce-handover/SKILL.md` across all three targets contains both writer- and loader-mode markers (mode-selection sentences, both path headings, all four persisted-document section names, the no-overwrite rule, the read-only loader rule, and the no-sibling-invocation rule). As anticipated by T02's code comment, wiring handover into the generic composite renderer replaces its T01 package-mode bespoke Purpose/output text with the shared composite preamble (adds a "Composite control flow" section handover's package mode didn't need) — copied that generated form over the repository-root `.pi/skills/sce-handover/SKILL.md` baseline (`references/output.md` and `prompts/handover.md` were already byte-identical). Also added the previously-missing root `.claude/commands/handover.md` + `.claude/skills/sce-handover/{SKILL.md,references/output.md}` and `.opencode/command/handover.md` + `.opencode/skills/sce-handover/{SKILL.md,references/output.md}`, and updated `.opencode/agent/Shared Context Code.md` with the generic catalog-derived `"sce-handover": allow` skill-permission line — all copied verbatim from canonical generation output, keeping these repository-root dogfood mirrors in sync with the other four workflows as they already were.
  - Verification: `nix develop -c pkl eval config/pkl/renderers/{pi-content,opencode-content,claude-content,metadata-coverage-check,generation-contract-check}.pkl` → all evaluate cleanly; `metadata-coverage-check` shows `opencode-command-route-handover`/`claude-command-route-handover`/`pi-command-route-handover` all `complete`; `generation-contract-check` shows `artifact-paths: exactly 61 paths` (up from 52), `workflow-references: fully internalized`, `decision-invocation: synchronization-only`, `opencode-decision-permission: code agent only`, `decision-package-*: complete`, and the new `handover-package-content: covers both modes`. `nix run .#pkl-check-generated` → passed (`Ephemeral Pkl generation passed: 61 files`), which also runs the double-generation determinism diff and the three negative fixtures. Independently generated two temp roots via `pkl eval -m <dir> config/pkl/generate.pkl` and `diff -rq`'d them → byte-identical. Diffed every file under the generated temp root against every pre-existing git-tracked file in root `.pi`/`.claude`/`.opencode` → zero diffs (no regression to the other four workflows or the two-agent inventory). `diff` of `.opencode/agent/{Shared Context Code,Shared Context Plan}.md` against generated output → Code agent gained only the expected `"sce-handover": allow` line; Plan agent unchanged. Confirmed no new agent slug and no phase-package artifacts (`sce-context-load`, `sce-plan-authoring`, etc.) appear anywhere in generated output. Attempted `nix flake check`; it failed only because `config/pkl/base/workflow-handover.pkl` (added in T02) and the other canonical Pkl sources this task depends on remain untracked in git — Nix's flake source filter excludes untracked files, a pre-existing gap since T02 and orthogonal to this task's correctness (`pkl-check-generated`, which runs against the real working tree rather than a git-filtered store copy, already exercises the same checks and passed). Not part of T03's specified verification; staging/committing is deferred to the commit workflow.
  - Assumptions: Repository-root `.claude`/`.opencode` mirrors are in scope to update alongside `.pi`, since all three are checked-in, catalog-driven, and were already kept byte-identical to canonical generation for the other four workflows (verified empirically before editing).

## Open questions

None. The user selected a fifth cross-target workflow and specified one dual-mode `/handover` entrypoint: no arguments writes, while a handover path loads.

## Validation Report

**Status:** validated  
**Date:** 2026-07-30

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (61 files, deterministic ephemeral generation passed)
- `nix flake check` -> exit 0 (all checks passed, including `pkl-generated`, `cli-tests`, `cli-clippy`, `cli-generated-input`)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Writer mode creates one populated handover with all four sections, assumption labeling, and reports the path -> inspected `.pi/prompts/handover.md` and `.pi/skills/sce-handover/SKILL.md` (writer path steps 1-6): context gathering from conversation/`git status`/`git diff`/`context/plans/*.md`, task-aligned or timestamped naming with no-overwrite rule, persisted-document template with all four required sections plus `Assumptions`, completeness confirmation before write, and `Writer success` report with written path.
- [x] AC2: Loader mode validates and presents a handover read-only -> inspected the same file's loader path steps 1-3: path must exist under `context/handovers/` with `.md` extension, all four required sections must be present, presentation via `Loader success` layout, and explicit read-only rules (no edits, no plan-state changes, no automation of the next step).
- [x] AC3: All three targets expose a thin `/handover` routed to one self-contained `sce-handover` package -> generated a temporary payload via `nix run .#pkl-generate`; confirmed `.claude/commands/handover.md`, `.opencode/command/handover.md`, and `.pi/prompts/handover.md` each invoke `sce-handover` exactly once; confirmed each `sce-handover` package contains only `SKILL.md` and `references/output.md`; confirmed the OpenCode/Claude agent inventory is unchanged (`Shared Context Code`, `Shared Context Plan` only) with no new phase-skill packages.
- [x] AC4: Canonical generation and metadata coverage enforce the five-workflow inventory -> `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` shows all `*-command-route-handover` entries as `complete`; `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` shows `artifact-paths: exactly 61 paths`, `workflow-references: fully internalized`, and `handover-package-content: covers both modes`; `nix run .#pkl-check-generated` passed with 61 files.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
