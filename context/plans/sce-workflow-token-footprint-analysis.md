# Plan: sce-workflow-token-footprint-analysis

## 1) Change summary
Analyze the SCE Plan (`/change-to-plan`) and Execute (`/next-task`) workflows end-to-end, inventory every participating agent/command/skill in each workflow, and define a static token-footprint analysis method plus token-reduction options that lower prompt/context overhead without changing role boundaries or safety gates.

## 2) Success criteria
- A complete, workflow-scoped inventory exists for Plan and Execute that lists all participating agents, commands, skills, and key context artifacts with ownership boundaries.
- A ranked token-reduction proposal set exists with impact/risk notes and explicit keep-vs-reduce guidance for each major prompt surface.
- A static token accounting method is documented and repeatable for this repo, including measurement inputs, counting procedure, and evidence format.
- The plan does not require a hard numeric reduction target yet.

## 3) Constraints and non-goals
- In scope: workflow analysis and planning artifacts under `context/` for Shared Context Plan/Code, `/change-to-plan`, `/next-task`, and linked skills.
- In scope: static token estimation design and documentation (artifact-based, no live execution requirement).
- Out of scope: implementing workflow text edits, command/skill rewrites, or canonical Pkl changes in this planning session.
- Out of scope: runtime/observed token telemetry estimation; this will be handled later.
- Non-goal: setting a mandatory reduction threshold (for example 20%) at this stage.

## 4) Task stack (T01..T05)
- [x] T01: Build canonical workflow inventory for Plan and Execute (status:done)
  - Task ID: T01
  - Goal: Produce a complete matrix of agents, commands, skills, and context docs used by `/change-to-plan` and `/next-task`, including ownership and invocation boundaries.
  - Boundaries (in/out of scope):
    - In: `context/sce/*.md` workflow docs, canonical command/skill ownership guidance, generated-surface references needed for coverage.
    - Out: changing any workflow behavior while inventorying.
  - Done when:
    - Plan workflow and Execute workflow each have explicit component lists (agents/commands/skills) with role descriptions.
    - Cross-workflow shared vs role-specific components are labeled.
  - Verification notes (commands or checks):
    - Manual cross-check against `context/sce/shared-context-plan-workflow.md`, `context/sce/shared-context-code-workflow.md`, and dedup ownership artifacts.
  - Implementation evidence:
    - Canonical inventory artifact created at `context/sce/workflow-token-footprint-inventory.md` with workflow-scoped participant matrix, ownership boundaries, and shared-vs-role-specific labels.

- [ ] T02: Map token-heavy prompt surfaces and duplication hotspots (status:todo)
  - Task ID: T02
  - Goal: Identify where token overhead accumulates across agents/commands/skills and classify each hotspot as intentional guardrail text vs reducible duplication.
  - Boundaries (in/out of scope):
    - In: structural analysis of repeated instruction blocks, orchestration-vs-skill duplication, and cross-target parity duplication risks.
    - Out: editing source prompts during analysis.
  - Done when:
    - A hotspot table exists with source location, reason for token cost, and keep/reduce recommendation.
    - Safety-critical text that must remain verbose is explicitly marked.
  - Verification notes (commands or checks):
    - Manual consistency review against `context/sce/plan-code-overlap-map.md` and `context/sce/dedup-ownership-table.md`.

- [ ] T03: Define static token accounting method and evidence template (status:todo)
  - Task ID: T03
  - Goal: Specify a deterministic static-token estimation workflow for SCE prompt artifacts, including counting scope, tokenizer choice assumptions, and report schema.
  - Boundaries (in/out of scope):
    - In: static estimation method, reproducibility notes, and evidence capture format in context docs/plan artifacts.
    - Out: runtime token observation and production telemetry collection.
  - Done when:
    - The method states exact inputs (which files/sections are counted), counting steps, and output fields (per-surface tokens, totals, and deltas).
    - Assumptions and known limitations of static estimates are documented.
  - Verification notes (commands or checks):
    - Manual dry-run review confirms the method can be repeated by a future session without ambiguity.

- [ ] T04: Propose reduction strategies with trade-offs and rollout order (status:todo)
  - Task ID: T04
  - Goal: Produce a prioritized set of token-reduction strategies that preserve SCE safety and role separation while reducing unnecessary prompt footprint.
  - Boundaries (in/out of scope):
    - In: strategy design (for example thin-wrapper tightening, canonical snippet reuse, redundancy pruning, and context-map narrowing rules).
    - Out: executing text reductions in canonical files.
  - Done when:
    - Proposal list is ranked by expected impact and implementation risk.
    - Each proposal includes affected artifacts, rationale, and regression risk notes.
  - Verification notes (commands or checks):
    - Manual review confirms proposals preserve mandatory gates (clarification, readiness, implementation stop, context sync).

- [ ] T05: Validation and cleanup (status:todo)
  - Task ID: T05
  - Goal: Validate that inventory, hotspot analysis, static accounting method, and reduction strategy outputs are coherent, complete, and ready for execution handoff.
  - Boundaries (in/out of scope):
    - In: completeness/consistency checks, final plan cleanup, and context sync verification requirements for downstream execution.
    - Out: implementing reduction changes.
  - Done when:
    - All success criteria are mapped to at least one completed task artifact.
    - Final outputs include a clear implementation starting point and no unresolved blocking ambiguity.
    - Validation notes include full-check and context-sync verification expectations for execution sessions.
  - Verification notes (commands or checks):
    - Manual traceability check from success criteria to task outputs.
    - Execution-phase validation baseline to run when implementation occurs: `nix run .#pkl-check-generated` and `nix flake check` plus required `sce-context-sync` completion.

## 5) Open questions
- None. Scope is confirmed to SCE workflow artifacts, static estimation only, and no hard reduction target at this stage.
