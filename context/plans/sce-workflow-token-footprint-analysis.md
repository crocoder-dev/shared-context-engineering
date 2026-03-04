# Plan: sce-workflow-token-footprint-analysis

## 1) Change summary
Analyze the SCE Plan (`/change-to-plan`) and Execute (`/next-task`) workflows end-to-end, inventory every participating agent/command/skill in each workflow, define a static token-footprint analysis method plus token-reduction options, and add implementation work for a deterministic TypeScript token-count script that operationalizes the T03 counting schema without changing workflow gates or prompt text.

## 2) Success criteria
- A complete, workflow-scoped inventory exists for Plan and Execute that lists all participating agents, commands, skills, and key context artifacts with ownership boundaries.
- A ranked token-reduction proposal set exists with impact/risk notes and explicit keep-vs-reduce guidance for each major prompt surface.
- A static token accounting method is documented and repeatable for this repo, including measurement inputs, counting procedure, and evidence format.
- A TypeScript static token-count script exists (for example `scripts/token-count-workflows.ts`) that reads the canonical T03 surface manifest (markdown-backed or checked-in JSON manifest), scopes text by rule (`entire-file` vs `canonicalBody subsection`), counts tokens with `o200k_base` (fallback `cl100k_base`), and emits deterministic report output.
- Report output includes required per-surface fields and run-level summary fields from T03, plus Plan/Execute totals, combined total, optional baseline deltas, and run metadata (timestamp, git SHA, tokenizer).
- Report artifacts are written to `context/tmp/`, usage is documented (`context/` or `README`), and verification evidence records exact command(s) run and artifact path(s).
- The plan does not require a hard numeric reduction target yet.

## 3) Constraints and non-goals
- In scope: workflow analysis and planning artifacts under `context/` for Shared Context Plan/Code, `/change-to-plan`, `/next-task`, and linked skills.
- In scope: static token estimation design, documentation, and deterministic local script implementation for T03 schema accounting.
- Out of scope: implementing workflow text edits, command/skill rewrites, or canonical Pkl changes in this planning session.
- Out of scope: runtime/observed token telemetry estimation; this remains a later phase.
- Out of scope: changing mandatory workflow gates, role boundaries, or prompt rewrites.
- Non-goal: setting a mandatory reduction threshold (for example 20%) at this stage.

## 4) Task stack (T01..T07)
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

- [x] T02: Map token-heavy prompt surfaces and duplication hotspots (status:done)
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
  - Implementation evidence:
    - Added ranked hotspot table and keep-vs-reduce classification (including explicit safety-critical markings) in `context/sce/workflow-token-footprint-inventory.md` under section `T02: Token-heavy prompt surfaces and duplication hotspots`.

- [x] T03: Define static token accounting method and evidence template (status:done)
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
  - Implementation evidence:
    - Added `T03: Static token accounting method and evidence template` to `context/sce/workflow-token-footprint-inventory.md`, including exact counted surface manifest, tokenizer assumptions, deterministic procedure, required report schema fields, evidence template, and known limitations.

- [x] T04: Propose reduction strategies with trade-offs and rollout order (status:done)
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
  - Implementation evidence:
    - Added `T04: Token-reduction strategy set, trade-offs, and rollout order` to `context/sce/workflow-token-footprint-inventory.md` with a prioritized proposal matrix (impact/risk/affected artifacts), explicit rollout sequencing, and preserve-as-is guardrail constraints.

- [x] T05: Define script inputs/outputs and manifest extraction contract (status:done)
  - Task ID: T05
  - Goal: Lock the deterministic interface for the workflow token-count script so implementation has unambiguous schema and extraction rules.
  - Boundaries (in/out of scope):
    - In: selecting manifest source strategy (`context/sce/workflow-token-footprint-inventory.md` parsing and/or checked-in JSON manifest), exact field mapping to T03 schema, deterministic report file naming/path under `context/tmp/`, baseline-delta input behavior.
    - Out: implementing prompt/gate text changes, altering workflow ownership boundaries, or introducing runtime telemetry collection.
  - Done when:
    - Script contract is explicit for all T03 surfaces, including `scope_rule`, `conditional`, and tokenizer fallback behavior.
    - Report schema mapping to T03 required fields is fully specified for per-surface rows and run-level totals.
    - Artifact path contract under `context/tmp/` is deterministic and implementation-ready.
  - Verification notes (commands or checks):
    - Manual traceability check from T03 schema table in `context/sce/workflow-token-footprint-inventory.md` to implementation contract fields.
  - Implementation evidence:
    - Added `T05: Script input/output and manifest extraction contract` section to `context/sce/workflow-token-footprint-inventory.md`, including canonical manifest strategy, extraction rules, tokenizer/baseline behavior, report schema mapping, and deterministic `context/tmp/token-footprint/` artifact contract.
    - Added canonical machine-readable manifest `context/sce/workflow-token-footprint-manifest.json` covering all T03 surfaces with explicit `scope_rule` and `conditional` fields.
    - Synced discoverability references in `context/context-map.md`, `context/overview.md`, and `context/glossary.md`.

- [x] T06: Implement TypeScript static token-count workflow script (status:done)
  - Task ID: T06
  - Goal: Build and wire a deterministic TypeScript script that computes per-surface and aggregate token counts for every workflow surface in the T03 manifest.
  - Boundaries (in/out of scope):
    - In: add `scripts/token-count-workflows.ts` (or repo-conventional equivalent), load manifest from canonical T03 inventory (or checked-in JSON), extract counted text (`entire-file` vs `canonicalBody subsection`), count with `o200k_base` fallback `cl100k_base`, emit deterministic report artifacts to `context/tmp/`, and add package script entry/document usage.
    - Out: changing workflow prompt content, changing mandatory workflow gates, or rewriting command/skill contracts.
  - Done when:
    - Script runs locally and emits all T03 required fields (`surface_id`, `workflow`, `artifact_class`, `path`, `scope_rule`, `tokenizer`, `tokens`, nullable baseline/delta fields, `conditional`, and run summary totals/metadata).
    - Report includes Plan subtotal, Execute subtotal, combined total, optional baseline deltas, timestamp, git SHA, tokenizer, and run ID.
    - At least one deterministic artifact is created under `context/tmp/` and usage instructions are documented in `context/` or `README`.
    - Verification notes capture exact run command(s) and produced artifact path(s).
  - Verification notes (commands or checks):
    - Execute the script through the documented command (`npm`/`bun` script or direct TS runtime invocation) and confirm report schema completeness.
    - Validate deterministic output contract by re-running without source changes and confirming stable structure/field set.
  - Implementation evidence:
    - Added TypeScript implementation at `evals/token-count-workflows.ts` with manifest-driven extraction (`entire-file`, `canonical-body-subsection`), tokenizer resolution (`o200k_base` fallback `cl100k_base`), optional baseline delta handling, and deterministic report emission.
    - Added Bun script entry `token-count-workflows` in `evals/package.json` and installed `js-tiktoken` dependency (`evals/bun.lock`).
    - Generated required artifacts at `context/tmp/token-footprint/workflow-token-count-latest.json`, `context/tmp/token-footprint/workflow-token-count-latest.md`, and archival JSON `context/tmp/token-footprint/workflow-token-count-t06-initial.json`.
    - Verification commands run:
      - `bun run token-count-workflows --run-id t06-initial` (from `evals/`)
      - `bun run token-count-workflows` (from `evals/`)
      - `bunx tsc --noEmit` (from `evals/`)
      - `bun run token-count-workflows --baseline context/tmp/token-footprint/workflow-token-count-t06-initial.json --run-id t06-baseline-check` (from `evals/`)
      - `nix run .#pkl-check-generated`
      - `nix flake check`

- [ ] T07: Validation and cleanup (status:todo)
  - Task ID: T07
  - Goal: Validate that analysis artifacts plus script implementation outputs are coherent, complete, and ready for downstream reduction work.
  - Boundaries (in/out of scope):
    - In: completeness checks across T01-T06 outputs, verification evidence hygiene, final plan cleanup, and context-sync requirements for execution sessions.
    - Out: further optimization or prompt-surface rewrites beyond approved implementation scope.
  - Done when:
    - All success criteria map to completed task artifacts, including T06 script evidence and `context/tmp/` report artifacts.
    - Final notes include exact verification commands, artifact paths, and residual limitations/assumptions (if any).
    - Execution-phase baseline and context-sync expectations are explicit for follow-on tasks.
  - Verification notes (commands or checks):
    - Manual traceability check from success criteria to T01-T06 outputs.
    - Execution-phase validation baseline: `nix run .#pkl-check-generated` and `nix flake check` plus required `sce-context-sync` completion.

## 5) Open questions
- None. Scope is confirmed to SCE workflow artifacts, static estimation only, and no hard reduction target at this stage.
