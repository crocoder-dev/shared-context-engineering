# Plan: commit-workflow

## Change summary

Restore `/commit` as a fourth canonical SCE workflow, modernized to the current command-plus-self-contained-skill package model. The workflow will preserve the old regular proposal path and `oneshot`/`skip` bypass behavior while giving the command and `sce-atomic-commit` skill an explicit ownership boundary and structured result contract.

The project-root `.pi/` package will remain the behavioral baseline, canonical Pkl will mirror it, and the OpenCode, Claude, and Pi renderers will emit the workflow with target-supported metadata. No new routing agent or Rust CLI behavior is introduced.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: The project-root `.pi/` surface contains a `/commit` prompt and self-contained `sce-atomic-commit` package whose regular mode requires staging confirmation, analyzes staged truth, returns atomic proposal and split guidance without committing, and preserves the plan-citation and context-guidance rules from the supplied workflow.
  - Validate: inspect `.pi/prompts/commit.md`, `.pi/skills/sce-atomic-commit/SKILL.md`, and every package-local reference named by the skill; confirm the regular-mode branch and structured result variants are complete and internally consistent.
- [x] AC2: `/commit oneshot` and `/commit skip` validate that staged content exists, request exactly one best-effort commit message without split/context gates, execute exactly one `git commit`, and report either the resulting commit hash or the unretried commit failure.
  - Validate: inspect the generated Pi prompt and skill package for exact first-token, case-insensitive bypass routing, the `No staged changes. Stage changes before commit.` stop, single-message handoff, best-effort plan citations, and success/failure termination rules.
- [x] AC3: OpenCode, Claude, and Pi generated payloads each contain the `/commit` command and one complete `sce-atomic-commit` package with all local references and target-supported metadata, while the existing three workflows remain unchanged and no new agent is generated.
  - Validate: generate a temporary payload with `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect each target inventory and compare the generated Pi commit command/skill package with the project-root `.pi/` baseline.
- [x] AC4: The canonical generator and metadata coverage enforce an exact four-command/eight-skill workflow inventory, including the new package references, and generation remains deterministic.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix run .#pkl-check-generated`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` where their exact three-workflow/seven-skill inventory or workflow-package terminology changes.
- Add focused durable context for the restored atomic commit workflow and update `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` (or a superseding decision) to record why `/commit` is restored after its earlier removal.

## Constraints and non-goals

- **In scope:** Project-root `.pi/` commit prompt and self-contained skill package; a canonical `workflow-commit.pkl` package; OpenCode, Claude, and Pi renderer metadata/wiring; generator and exact-inventory coverage updates.
- **Out of scope:** Rust CLI changes, Git hook behavior, a new OpenCode routing agent, restoration of `/handover` or the automated OpenCode profile, and compatibility generated trees.
- **Constraints:** Preserve the supplied regular and bypass semantics; keep staged changes as the source of truth; keep commands thin and the skill responsible for diff analysis, atomicity, and message wording; follow the current self-contained package/reference and ephemeral-generation architecture; use Nix-managed validation.
- **Non-goal:** Generalize this work into an arbitrary Git workflow engine or change the existing `/change-to-plan`, `/next-task`, or `/validate` lifecycle semantics.

## Assumptions

- “Modernize it to look and format like current ones” means adding `/commit` to the same project-root `.pi/` baseline and canonical cross-target Pkl workflow model used by the existing three workflows.
- The restored workflow is available directly as a command on all three targets and does not belong to either thin OpenCode Plan/Code routing agent.
- A package-local structured result contract will distinguish regular proposals, bypass single-message readiness, and blocked analysis; the command remains responsible for user prompting and `git commit` execution.
- The aliases `oneshot` and `skip` remain behaviorally identical and are recognized only as the case-insensitive first argument token.

## Task stack

- [x] T01: `Define the modern commit workflow baseline` (status:done)
  - Task ID: T01
  - Goal: Add the project-root `.pi/` `/commit` command and self-contained `sce-atomic-commit` skill package in the same orchestration/result-contract style as current workflows while preserving the supplied regular and bypass behavior.
  - Boundaries (in/out of scope): In — `.pi/prompts/commit.md`, `.pi/skills/sce-atomic-commit/SKILL.md`, and package-local result/reference documents needed for a structured command-to-skill handoff. Out — Pkl sources, generated payload wiring, durable context updates, and Git execution during implementation.
  - Dependencies: none
  - Done when: The command has explicit input and regular/bypass branches; command-vs-skill ownership is non-duplicative; the skill analyzes only staged truth and returns contract-valid proposal, bypass-message, or blocked results; regular mode remains proposal-only; bypass mode permits one command-owned commit; all skill references resolve within the package.
  - Verification notes (commands or checks): `git diff --check -- .pi/prompts/commit.md .pi/skills/sce-atomic-commit`; inspect the command, skill, and result contract together for every supplied regular/bypass rule and package-local reference.
  - Completed: 2026-07-28
  - Files changed: `.pi/prompts/commit.md`, `.pi/skills/sce-atomic-commit/SKILL.md`, `.pi/skills/sce-atomic-commit/references/commit-contract.yaml`, `.pi/skills/sce-atomic-commit/references/commit-message-style.md` (all new).
  - Evidence: Recovered the behavioral baseline from `git show 2a947b2^:.pi/prompts/commit.md` and `git show 2a947b2^:.pi/skills/sce-atomic-commit/SKILL.md`, then re-authored it in the current command-plus-self-contained-skill style modeled on `.pi/prompts/validate.md`, `.pi/skills/sce-validation/SKILL.md`, and `.pi/skills/sce-plan-review/references/readiness-contract.yaml`. `git add -N` + `git diff --check -- .pi/prompts/commit.md .pi/skills/sce-atomic-commit` -> exit 0 (index restored afterwards with `git reset`). `nix run nixpkgs#yq -- '.variants | keys' .pi/skills/sce-atomic-commit/references/commit-contract.yaml` -> exit 0, reporting `blocked`, `bypass_message`, `proposal`. Both references named by the skill resolve inside the package (`ls -R .pi/skills/sce-atomic-commit`). Cross-read confirms every supplied rule: exact case-insensitive first-token `oneshot`/`skip` routing, the `No staged changes. Stage changes before commit.` stop, the verbatim staging-confirmation prompt, skipped context gate and single best-effort message in bypass mode, one command-owned `git commit` reporting the hash or the unretried failure, and proposal-only regular mode with split guidance.
  - Notes: Deviation from the old text — the bypass overrides and message rules are stated once by their owner instead of being duplicated across the command and skill, as the task's non-duplicative-ownership done check requires. Additions the old workflow lacked: a structured `proposal`/`bypass_message`/`blocked` result contract and a package-local message-style reference, both of which T02 must mirror in Pkl. No Pkl, generated payload, or `context/` file outside this plan was touched, and no `git commit` ran.

- [x] T02: `Model the canonical commit workflow package` (status:done)
  - Task ID: T02
  - Goal: Add a canonical Pkl workflow package that mirrors the complete project-root commit baseline without changing emitted target inventories yet.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-commit.pkl` and only the minimal shared workflow-model changes needed to represent its command and self-contained skill documents. Out — renderer imports, target metadata, generator mappings, generated payloads, and existing workflow semantics.
  - Dependencies: T01
  - Done when: The focused Pkl module exposes one `commit` command and one complete `sce-atomic-commit` package; every package-relative document is deterministic and matches the corresponding `.pi/` baseline text; the module introduces no sibling-package dependency.
  - Verification notes (commands or checks): `nix develop -c pkl eval -f json config/pkl/base/workflow-commit.pkl`; extract and compare all exposed documents with `.pi/prompts/commit.md` and `.pi/skills/sce-atomic-commit/`; `git diff --check -- config/pkl/base/workflow-commit.pkl`.
  - Completed: 2026-07-28
  - Files changed: `config/pkl/base/workflow-commit.pkl` (new). `config/pkl/base/workflow-content.pkl` was not modified — the existing `WorkflowDocument`/`SkillPackage`/`WorkflowCommand`/`WorkflowPackage` classes represent the commit package without change.
  - Evidence: Modeled on `config/pkl/base/workflow-validate.pkl` (local `makeDocument`/`packageDocuments` helpers, one multiline `String` constant per document, one top-level `workflow`). `nix develop -c pkl eval -f json -o <tmp>/commit.json config/pkl/base/workflow-commit.pkl` -> exit 0, exposing `workflow.slug = commit`, `workflow.command.slug = commit`, `workflow.command.document.path = commit.md`, and exactly one skill package `sce-atomic-commit` with documents `SKILL.md`, `references/commit-contract.yaml`, `references/commit-message-style.md`. Each `.text` extracted with `jq -r` (which appends the single newline the generator adds via `"\(document.text)\n"`) `diff`s clean against `.pi/prompts/commit.md`, `.pi/skills/sce-atomic-commit/SKILL.md`, and both package references — four MATCH, zero DIFF. A second `pkl eval` is byte-identical to the first (`cmp` -> equal), so the module is deterministic. The only import is `workflow-content.pkl`; no sibling workflow package is imported. `git add -N` + `git diff --check -- config/pkl/base/workflow-commit.pkl` -> exit 0 (index restored with `git reset`).
  - Notes: Deviation from existing workflow modules — blank lines inside the multiline strings are emitted empty rather than carrying the 4-space block indent that `workflow-validate.pkl` and its siblings use. Pkl strips indentation identically either way and the emitted text is byte-identical to the baseline, but the existing style leaves whitespace-only lines that `git diff --check` would flag, and this task verifies with that command. No renderer, generator mapping, metadata inventory, or generated payload was touched, so target inventories are unchanged and the new module is not yet reachable from `config/pkl/generate.pkl`; T03 wires it in.

- [x] T03: `Render and enforce the fourth workflow` (status:done)
  - Task ID: T03
  - Goal: Wire the canonical commit package into OpenCode, Claude, and Pi rendering and update exact generation coverage for the expanded workflow matrix.
  - Boundaries (in/out of scope): In — target renderer imports/lists, OpenCode command skill-chain metadata without a new agent assignment, Claude command tool metadata, `config/pkl/generate.pkl` as needed, metadata coverage inventories, and focused generation documentation/tooling claims tied to exact counts. Out — edits to commit behavior authored in T01/T02, new agents, generated repository trees, Rust CLI, hooks, and unrelated assets.
  - Dependencies: T02
  - Done when: Temporary generation emits `/commit` plus its complete skill package for all three targets; Pi output matches the root baseline; OpenCode records `sce-atomic-commit` as the entry/only skill without adding an agent; Claude grants the Git-inspection/commit tool capability required by the workflow; exact coverage expects four commands, eight skill packages, and all package documents; retained workflows and non-Markdown outputs are unchanged.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/{pi-content,opencode-content,claude-content,metadata-coverage-check}.pkl`; generate to a temporary root and inspect exact target inventories, local reference resolution, Pi baseline parity, OpenCode metadata, Claude tool metadata, unchanged existing workflow documents, and absence of any added agent.
  - Completed: 2026-07-28
  - Files changed: `config/pkl/renderers/pi-content.pkl`, `config/pkl/renderers/opencode-content.pkl`, `config/pkl/renderers/claude-content.pkl`, `config/pkl/renderers/claude-metadata.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/README.md`. `config/pkl/generate.pkl` and `config/pkl/renderers/opencode-metadata.pkl` were not modified — the generator's emission loops are generic over the renderer output mappings, and no agent metadata changed.
  - Evidence: Each content renderer imports `../base/workflow-commit.pkl` and appends `commit.workflow` to its `workflows` listing. `opencode-content.pkl` adds a `commit` skill-chain entry (`entry-skill`/`skills` = `sce-atomic-commit` only) and makes the `agent:` front-matter line conditional via `commandAgentLine`, so `/commit` is emitted without an agent while the three routed commands keep theirs. `claude-metadata.pkl` grants `/commit` the same allowed-tools string as `/next-task` and `/validate`, including `Bash` for Git inspection and commit execution. `metadata-coverage-check.pkl` now expects four command slugs and 21 skill document paths across eight packages. `config/pkl/README.md` count and removed-surface claims updated from three/seven to four/eight. All four `nix develop -c pkl eval config/pkl/renderers/{pi-content,opencode-content,claude-content,metadata-coverage-check}.pkl` -> exit 0, with all seven inventory checks reporting `complete`. `nix run .#pkl-generate -- <tmp>` -> exit 0, emitting per target exactly four commands and eight skill packages, including `sce-atomic-commit/{SKILL.md,references/commit-contract.yaml,references/commit-message-style.md}`, which are the only two reference paths the skill names. Baseline comparison against a detached HEAD worktree generation -> `diff -rq` reports only six additions (the commit command and skill package in each of `.opencode`, `.claude`, `.pi`) and zero modifications, so retained workflows, agents, and non-Markdown outputs are byte-identical. `diff -r .pi/prompts` and `diff -r .pi/skills` against the generated Pi tree -> full match. Generated agent directory still contains exactly `Shared Context Code.md` and `Shared Context Plan.md`. `git add -N` + `git diff --check -- config/pkl/renderers config/pkl/README.md` -> exit 0 (index restored with `git reset`).
  - Notes: The renderer, metadata, coverage, and README edits were already present uncommitted in the working tree at the start of this task; they were verified in place rather than reauthored, and no gap required filling. The only structural deviation from the previous rendering model is that OpenCode's `agent:` line is now optional instead of required for every command, which was necessary because `/commit` belongs to neither routing agent; the retained commands' output is unchanged, as the baseline diff proves. Full-plan validation (`nix run .#pkl-check-generated`, `nix flake check`) was deliberately not run — it belongs to `/validate`.

## Open questions

None. The supplied old workflow fixes the behavior to preserve, and the repository’s current workflow-package conventions determine the modernization and cross-target scope.

## Validation Report

**Status:** validated  
**Date:** 2026-07-28

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation passed: 85 files, inventory sha256 `4cfd88cb…5d52b18`)
- `nix flake check` -> exit 1, then exit 0 (first run failed only because the new files were untracked, so the flake's Git source copy could not resolve `config/pkl/base/workflow-commit.pkl`; rerun after `git add -N` on the new commit-workflow paths passed all 5 checks, and the index was restored with `git reset`)
- `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` -> exit 0 (all 7 inventory checks report `complete`)
- `nix run .#pkl-generate -- <tmp>` -> exit 0 (twice, to a clean temporary root)
- `diff -rq <gen> <gen2>` -> exit 0 (byte-identical generations)
- `diff -r .pi/prompts <gen>/.pi/prompts`, `diff -r .pi/skills <gen>/.pi/skills` -> exit 0 (full Pi baseline parity)
- `diff -rq .opencode <gen>/.opencode`, `diff -rq .claude <gen>/.claude` -> additions only

### Scaffolding removed

- `<scratchpad>/gen`, `<scratchpad>/gen2` — temporary generation roots created for AC3/AC4 verification, outside the repository.

No temporary scaffolding remained inside the repository; the working tree contains only the plan's intended files.

### Success-criteria verification

- [x] AC1: Project-root `.pi/` contains a `/commit` prompt and self-contained `sce-atomic-commit` package with a complete regular-mode branch -> Inspected `.pi/prompts/commit.md`, `.pi/skills/sce-atomic-commit/SKILL.md`, and both package references. The regular path stops for the verbatim staging-confirmation prompt before invoking the skill, the skill reads only `git diff --cached`, and the regular branch is proposal-only with split guidance (SKILL steps 2/7) and never runs `git commit`. The plan-citation rule (SKILL step 5) and context-file guidance gating (`context_only`/`mixed`, SKILL step 6) are preserved. Both references named by the skill — `references/commit-contract.yaml` and `references/commit-message-style.md` — resolve inside the package, and the contract's `proposal`/`bypass_message`/`blocked` variants are internally consistent with the command's status branching.
- [x] AC2: `oneshot`/`skip` bypass validates staged content, requests one best-effort message without split/context gates, executes exactly one `git commit`, and reports hash or unretried failure -> Inspected the generated Pi prompt and skill package (byte-identical to the root baseline). Routing recognizes `oneshot`/`skip` only as an exact case-insensitive first whitespace-separated token; the bypass path runs `git diff --cached --quiet` and stops with exactly `No staged changes. Stage changes before commit.`; the skill stops grouping in bypass mode (one message, no split guidance) and skips the context-file gate entirely; plan citations are best-effort (infer when supported, otherwise omit, never stop or invent); the command runs `git commit` once and terminates on either the reported hash or the Git failure, with no retry, amend, or fallback.
- [x] AC3: All three generated payloads contain `/commit` and one complete `sce-atomic-commit` package with target-supported metadata, existing workflows unchanged, no new agent -> Temporary generation emits exactly 4 commands and 8 skill packages per target. `sce-atomic-commit` ships `SKILL.md` plus both references under `.opencode/skills/`, `.claude/skills/`, and `.pi/skills/`. Claude's `/commit` carries the same `allowed-tools` string as `/next-task` and `/validate` (including `Bash`); OpenCode's records `entry-skill`/`skills` = `sce-atomic-commit` only and emits no `agent:` line, while `/next-task` keeps `agent: "Shared Context Code"`. The generated agent directory still holds exactly `Shared Context Code.md` and `Shared Context Plan.md`. `diff -rq` against the repository's committed `.opencode`/`.claude` trees reports only the commit additions and zero modifications; the Pi tree matches the root baseline exactly.
- [x] AC4: Exact four-command/eight-skill coverage including the new package references, generation deterministic -> `metadata-coverage-check.pkl` declares `expectedCommandSlugs` of 4 (including `commit`) and 21 `expectedSkillDocumentPaths` across 8 packages (including all three `sce-atomic-commit` documents), enforced by `assertExactKeys`, which compares length and membership so unexpected keys fail too. Evaluation exits 0 with all 7 checks `complete`, and `nix run .#pkl-check-generated` exits 0. Two independent generations are byte-identical.

### Failed checks and follow-ups

- None.

### Residual risks

- The repository's committed `.opencode/` and `.claude/` trees do not yet contain `/commit` or the `sce-atomic-commit` package. `.pi/` is in parity, but the other two generated trees are stale relative to canonical Pkl. No repository check enforces that parity — `pkl-check-generated` verifies ephemeral generation only — so this is not a validation failure, but the trees should be regenerated before release.
- `nix flake check` only sees Git-tracked files. Until the new commit-workflow files are committed (or at least `git add`-ed), a plain `nix flake check` on this working tree fails on the missing `workflow-commit.pkl` import.
