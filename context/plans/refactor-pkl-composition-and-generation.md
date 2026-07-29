# Plan: refactor-pkl-composition-and-generation

## Change summary

Complete the next structural Pkl improvements after `strengthen-pkl-structure-foundation`: move repeated workflow document/package constructors into the shared model, replace the positionally coupled numbered context-sync fragments with named semantic sections, replace composite workflow prose rewriting with structured rendering one workflow at a time, and centralize deterministic generation plus inventory creation behind one producer contract.

The refactor preserves the accepted cross-target architecture and the exact 46-file generated payload. Canonical workflow semantics, target frontmatter, human-visible output, plugin and extension sources, schema behavior, Cargo `OUT_DIR` handoff, and packaging fallbacks must remain byte-compatible. The work changes how canonical content is represented and produced, not what users receive.

## Acceptance criteria

- [x] AC1: Workflow document construction and package-document mapping have one shared implementation, and no canonical workflow module retains a local `makeDocument` or `packageDocuments` equivalent.
  - Validate: evaluate every `config/pkl/base/workflow-*.pkl` module and search those modules for duplicate constructor definitions.
- [x] AC2: Task and plan context-sync documents are assembled from named semantic sections and role data rather than numbered common/task/plan fragments or manually interleaved positional lists, while their composed generated output remains byte-identical.
  - Validate: inspect `workflow-context-sync.pkl` for named sections and absence of numbered fragment identifiers; compare generated `sce-next-task` and `sce-validate` package trees before and after the refactor.
- [x] AC3: `workflow-composite.pkl` renders commands, phases, internal persisted-document formats, and output references from structured source values without `stripFrontmatterMarkers`, `internalize`, `internalizePhase`, or prose-wide replacement chains.
  - Validate: focused source search finds none of the removed helpers or their rewrite tables; direct renderer checks and the forbidden-reference generation contract pass.
- [x] AC4: Change-to-plan, next-task, validate, and commit are migrated independently, and each migration preserves that workflow's generated OpenCode, Claude, and Pi command/prompt plus two-file package bytes.
  - Validate: for each workflow migration, diff its paths across all three target trees against a retained pre-task temporary baseline.
- [x] AC5: One repository-owned producer contract performs Pkl evaluation, two-pass determinism comparison, generated payload inventory creation, and canonical-input inventory creation for repository Cargo builds, generated-output checks, packaging fallbacks, and the Nix generated-input derivation.
  - Validate: inspect all four consumers; focused search finds no independent implementation of the producer mechanics outside the producer and its tests.
- [x] AC6: The producer rejects nondeterministic generation and missing or stale canonical inputs, emits the existing `pkl-generated/`, `SHA256SUMS`, and `INPUTS.SHA256SUMS` handoff shape where required, and cleans temporary state on success, failure, and handled signals.
  - Validate: run producer/wrapper tests covering success, generation drift, input mutation, subprocess failure, and cleanup; validate the resulting handoff through `cli/build.rs` in the normal repository check path.
- [x] AC7: The finished generator emits exactly the same 46 paths and bytes as the pre-plan baseline, and all repository checks pass.
  - Validate: generate complete before/after temporary roots and run `diff -r`; run the full validation commands below.

### Full validation

- `bash scripts/test-run-cli-cargo.sh`
- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, and `context/patterns.md` to describe shared workflow constructors, named context-sync sections, structured composite rendering, and the single generation producer contract.
- Update `context/glossary.md` with structured workflow rendering and generated-input producer terminology if those names become canonical.
- Update `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md` through a superseding decision if replacing prose internalization changes rationale that should remain durable.
- Update `context/decisions/2026-07-27-ephemeral-pkl-build-generation.md` through a superseding decision if the producer contract materially changes its ownership description.
- Update `context/context-map.md` only when a new decision or focused durable context document is added.

## Constraints and non-goals

- **In scope:** Shared Pkl workflow constructors; `workflow-context-sync.pkl`; structured workflow content/rendering primitives; the four canonical workflow modules and shared composite renderer; generator input declaration; deterministic Pkl producer scripts and tests; `check-generated.sh`, Cargo wrapper, packaging fallback helper, and Nix generated-input integration; directly affected documentation and durable context.
- **Out of scope:** Workflow behavior or wording changes; target inventory changes; new targets or workflows; plugin, Pi extension, settings, policy, schema, setup, or CLI runtime behavior; changing the published crate fallback shape; replacing vendored Pkl dependencies; broad Pkl directory reorganization.
- **Constraints:** Preserve all generated bytes and exact paths; migrate one workflow at a time; keep intermediate commits evaluable with already-migrated workflows using structured rendering and remaining workflows using a narrowly isolated transitional legacy path; remove that path after commit migration; run Pkl and Cargo validation through Nix-owned repository entrypoints.
- **Non-goal:** Build a general Markdown parser or a templating framework for arbitrary documents. Structured rendering needs only semantic constructs used by the four current workflows.

## Assumptions

- `context/plans/strengthen-pkl-structure-foundation.md` is the implemented baseline, so this plan can consume `workflow-catalog.pkl` and the exact generation-contract checks it introduced.
- Byte identity applies to the complete generated payload. Canonical source formatting may change substantially when content becomes structured.
- Structured rendering will represent semantic differences—frontmatter, phase references, internal-state contracts, output references, and directive vocabulary—as typed values or rendering functions before Markdown assembly; renaming the existing replacement helpers or replacing prose with symbolic-string substitution would not satisfy the request.
- The producer may leave Nix fileset construction declarative, but the input manifest and generation/inventory algorithm must have one machine-readable owner that shell and Nix consumers share rather than parallel hand-maintained implementations.

## Task stack

- [x] T01: `Move workflow constructors into the shared Pkl model` (status:done)
  - Task ID: T01
  - Goal: Give `workflow-content.pkl` canonical helpers for creating documents and deterministic package-document mappings, then remove the four local helper pairs.
  - Boundaries (in/out of scope): In — `workflow-content.pkl` helper API and constructor call-site updates in the four canonical workflow modules. Out — document text, package inventory, context-sync structure, composite rendering, and generation scripts.
  - Dependencies: none
  - Done when: All workflow modules consume the shared helpers, no local equivalent remains, direct module evaluations pass, and the complete generated payload matches a pre-task baseline.
  - Verification notes (commands or checks): evaluate `workflow-content.pkl` and each `workflow-{change-to-plan,next-task,validate,commit}.pkl`; focused search for `local makeDocument|local packageDocuments`; generate and `diff -r` temporary payloads; run `nix run .#pkl-check-generated`.
  - Completed: 2026-07-29
  - Files changed: `config/pkl/base/workflow-content.pkl`, `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/base/workflow-next-task.pkl`, `config/pkl/base/workflow-validate.pkl`, `config/pkl/base/workflow-commit.pkl`
  - Evidence: Direct Pkl evaluation passed for the shared model and all four workflow modules; focused search found no local constructor definitions; `diff -r` reported no differences between complete pre-task and post-task generated payloads; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`.
  - Notes: Shared helpers are hidden module members so direct model evaluation remains renderable while imported workflow modules can consume the canonical implementation.

- [x] T02: `Replace numbered context-sync skill fragments with named sections` (status:done)
  - Task ID: T02
  - Goal: Rebuild task/plan context-sync `SKILL.md` assembly around named semantic skill sections and role-owned data instead of numbered common and role fragment triples.
  - Boundaries (in/out of scope): In — context-sync skill frontmatter, purpose, inputs, workflow, boundaries, and completion sections plus shared role data needed to render them. Out — sync report rendering, workflow semantics, composite renderer changes, and generated wording.
  - Dependencies: T01
  - Done when: Skill assembly is ordered by named semantic sections, task/plan differences are explicit role fields or section renderers, numbered skill fragment identifiers and positional interleaving are gone, and both generated workflow package subtrees are byte-identical.
  - Verification notes (commands or checks): evaluate `workflow-context-sync.pkl`; compare extracted task and plan skill text before/after; diff generated `sce-next-task` and `sce-validate` packages across all targets; run `nix run .#pkl-check-generated`.
  - Completed: 2026-07-29
  - Files changed: `config/pkl/base/workflow-context-sync.pkl`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`
  - Evidence: Direct Pkl JSON evaluation passed; extracted task and plan `SKILL.md` bytes matched the supplied direct-module baseline; recursive diffs for `sce-next-task` and `sce-validate` packages matched the supplied OpenCode, Claude, and Pi baseline subtrees; focused search found no numbered skill-fragment identifiers or positional skill assembly while confirming numbered report fragments remain; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: Context-sync skills now render explicit role-owned frontmatter, purpose, input, workflow, boundaries, and completion sections in semantic order. Report assembly and composite rendering remain unchanged for T03 and later tasks.

- [x] T03: `Replace numbered context-sync report fragments with named sections` (status:done)
  - Task ID: T03
  - Goal: Complete the context-sync refactor by expressing report layouts as named semantic sections driven by the same role model.
  - Boundaries (in/out of scope): In — task/plan sync report headings, status layouts, changed-context summaries, verification, and continuation sections; removal of obsolete numbered report fragments and manual interleaving lists. Out — report wording, output ownership, composite rendering, or changes to task/plan synchronization policy.
  - Dependencies: T02
  - Done when: No numbered common/task/plan fragment identifier remains in `workflow-context-sync.pkl`, both complete packages evaluate from named sections, and generated `sce-next-task` and `sce-validate` output bytes are unchanged.
  - Verification notes (commands or checks): focused search for numeric fragment naming and positional lists; direct Pkl evaluation; before/after extraction comparison for both sync reports; generated subtree diff; `nix run .#pkl-check-generated`.
  - Completed: 2026-07-29
  - Files changed: `config/pkl/base/workflow-context-sync.pkl`
  - Evidence: Direct Pkl JSON evaluation matched the retained pre-task module baseline byte-for-byte; focused search found no numbered common/task/plan report fragment identifiers, `reportFragments`, or positional newline assembly while confirming named introduction, synced, no-context-change, blocked, rules, and report renderers; recursive diffs for generated `sce-next-task` and `sce-validate` packages matched retained OpenCode, Claude, and Pi baseline subtrees; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: A typed `SyncReportRole` now carries lifecycle-specific report values while shared named renderers assemble each semantic report variant and rule section without numbered interleaving.

- [x] T04: `Introduce structured rendering with change-to-plan` (status:done)
  - Task ID: T04
  - Goal: Add the minimal structured document/rendering model and migrate change-to-plan so its composite command, phases, persisted plan template, and output references render directly without prose rewriting.
  - Boundaries (in/out of scope): In — typed frontmatter/body/semantic-reference primitives, a transitional composite seam supporting structured and legacy workflows, `workflow-change-to-plan.pkl`, and the change-to-plan composite definition. Out — migration of next-task, validate, or commit; removal of the transitional legacy path; any generated wording change.
  - Dependencies: T03
  - Done when: Change-to-plan uses only structured rendering, its three-target command/prompt and package paths are byte-identical, and the other workflows continue evaluating through one isolated legacy adapter.
  - Verification notes (commands or checks): direct model/base/composite evaluations; focused check that change-to-plan does not pass through legacy internalization; diff only change-to-plan paths for OpenCode, Claude, and Pi against a retained baseline; run generation contract and `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-content.pkl`, `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/renderers/workflow-composite.pkl`
  - Evidence: Direct Pkl evaluation passed for the shared model, change-to-plan base module, composite renderer, and all three target content renderers; the generation contract passed; focused source inspection confirmed change-to-plan supplies `structuredComposite` directly while next-task, validate, and commit alone use `legacyComposite`; a recursive diff of complete pre-task and post-task generated roots reported no differences; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: The shared model now distinguishes package and composite render modes with typed frontmatter, body, semantic-reference, structured-document, and composite-source values. Change-to-plan renders its command, internal phases, persisted plan template, and output references from those values without prose internalization; the transitional rewrite path is isolated behind `legacyComposite` for the three workflows scheduled in T05–T07.

- [x] T05: `Migrate next-task to structured rendering` (status:done)
  - Task ID: T05
  - Goal: Express next-task phase sequencing, state contracts, implementation gate references, and sync output composition through the structured renderer.
  - Boundaries (in/out of scope): In — `workflow-next-task.pkl`, next-task composite assembly, and structured constructs proven necessary by this workflow. Out — validate and commit migration, context-sync policy changes, and user-visible text changes.
  - Dependencies: T04
  - Done when: Next-task no longer uses the legacy internalization path, all next-task generated paths across three targets are byte-identical, and change-to-plan remains unchanged.
  - Verification notes (commands or checks): direct focused evaluations; compare next-task target subtrees against the pre-task baseline and recheck change-to-plan; run generation contract and `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-next-task.pkl`, `config/pkl/base/workflow-context-sync.pkl`, `config/pkl/renderers/workflow-composite.pkl`
  - Evidence: Direct Pkl evaluation passed for the shared model, context-sync module, next-task module, composite renderer, all three target content renderers, and the generation contract; focused source inspection confirmed next-task supplies `structuredComposite` directly while only validate and commit use `legacyComposite`; a recursive diff of complete pre-task and post-task generated roots reported no differences, including explicit change-to-plan package regression diffs across OpenCode, Claude, and Pi; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: Next-task now renders its command, plan-review and task-execution phases, state-contract and implementation-gate references, task context-sync phase, and output references from mode-aware structured values. The shared context-sync model exposes the task role as a structured phase without changing package wording or policy; the transitional legacy path remains isolated to validate and commit for T06–T07.

- [x] T06: `Migrate validate to structured rendering` (status:done)
  - Task ID: T06
  - Goal: Express validation phases, failed-validation state, persisted validation-report format, plan-sync references, and output composition through the structured renderer.
  - Boundaries (in/out of scope): In — `workflow-validate.pkl`, validate composite assembly, and narrowly required structured constructs. Out — commit migration, validation behavior changes, and plan context-sync policy changes.
  - Dependencies: T05
  - Done when: Validate no longer uses legacy internalization, its generated paths are byte-identical across all targets, and previously migrated workflows remain unchanged.
  - Verification notes (commands or checks): direct focused evaluations; validate subtree diff plus regression diffs for change-to-plan and next-task; run generation contract and `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-validate.pkl`, `config/pkl/base/workflow-context-sync.pkl`, `config/pkl/renderers/workflow-composite.pkl`
  - Evidence: Direct Pkl evaluation passed for the context-sync module, validate module, composite renderer, and all three target content renderers; the generation contract passed with exactly 46 artifact paths and fully internalized workflow references; focused source inspection confirmed validate supplies `structuredComposite` directly while commit alone uses `legacyComposite`; recursive complete-root and explicit validate, change-to-plan, and next-task diffs against the retained pre-task baseline reported no differences; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: Validate now renders its command, validation phase, failed-validation state, persisted plan-file validation report, plan context-sync phase, and output references from mode-aware structured values. The shared context-sync model exposes the plan role as a structured phase without changing package wording or policy; the transitional legacy path remains isolated to commit for T07.

- [x] T07: `Migrate commit and remove prose internalization` (status:done)
  - Task ID: T07
  - Goal: Migrate commit to structured rendering and delete the transitional prose-rewrite implementation once all four workflows render structurally.
  - Boundaries (in/out of scope): In — `workflow-commit.pkl`, commit composite assembly, removal of `stripFrontmatterMarkers`, `internalize`, `internalizePhase`, rewrite tables, and the legacy adapter. Out — commit workflow semantics, commit-message style content, and unrelated renderer cleanup.
  - Dependencies: T06
  - Done when: Every workflow is structurally rendered; no prose-wide internalization helper or replacement chain remains; all four workflow subtrees and the complete generated payload are byte-identical.
  - Verification notes (commands or checks): focused search for removed helpers and rewrite strings; direct evaluation of all workflow modules and renderers; complete before/after generated-tree diff; generation contract; `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-commit.pkl`, `config/pkl/renderers/workflow-composite.pkl`
  - Evidence: Direct Pkl evaluation passed for the shared model, context-sync module, all four workflow modules, composite renderer, all three target content renderers, and the generation contract; focused source inspection found no `stripFrontmatterMarkers`, `internalize`, `internalizePhase`, `legacyComposite`, or prose rewrite chain outside vendored dependency code; a recursive diff of complete pre-task and post-task generated roots reported no differences; `nix run .#pkl-check-generated` passed with 46 files and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `git diff --check` passed.
  - Notes: Commit now renders its command, atomic-commit phase, contract references, internal-state vocabulary, and message-style output from mode-aware structured values. Every composite workflow requires a structured source, so the nullable legacy seam and all renderer-wide prose rewriting have been removed.

- [x] T08: `Create the canonical generated-input producer` (status:done)
  - Task ID: T08
  - Goal: Introduce one tested producer that owns canonical input discovery, two-pass Pkl generation, determinism comparison, payload inventory, input inventory, and temporary-state cleanup, then make the Cargo wrapper consume it.
  - Boundaries (in/out of scope): In — one machine-readable generator-input declaration, producer script/library, focused producer tests, and simplification of `scripts/run-cli-cargo.sh`. Out — `check-generated.sh`, packaging helper, Nix migration, `cli/build.rs` handoff shape, and generated content.
  - Dependencies: none
  - Done when: The producer creates the existing validated generated-input directory shape; `run-cli-cargo.sh` delegates generation/inventory work to it; focused tests cover deterministic success, drift, missing/input mutation, subprocess failure, and cleanup; wrapper behavior remains stable.
  - Verification notes (commands or checks): run the new producer test suite and `bash scripts/test-run-cli-cargo.sh`; inspect one handoff's `pkl-generated/`, `SHA256SUMS`, and `INPUTS.SHA256SUMS`; run a narrow wrapper-driven CLI build through the documented Nix command if needed.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/generator-inputs.txt`, `scripts/produce-cli-generated-input.sh`, `scripts/test-produce-cli-generated-input.sh`, `scripts/run-cli-cargo.sh`, `scripts/test-run-cli-cargo.sh`
  - Evidence: `bash scripts/test-produce-cli-generated-input.sh` passed deterministic success, generation drift, missing declared input, canonical input mutation, Pkl subprocess failure, and temporary cleanup cases; `bash scripts/test-run-cli-cargo.sh` passed argument forwarding, regeneration, refreshed inventories, Cargo status propagation, and wrapper cleanup; an inspected real producer handoff contained `pkl-generated/`, `SHA256SUMS` with 46 payload entries, and `INPUTS.SHA256SUMS` with 31 canonical input entries; `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` completed successfully; `nix run .#pkl-check-generated` retained the exact 46-file payload and inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; Bash syntax checks and `git diff --check` passed.
  - Notes: `config/pkl/generator-inputs.txt` is the machine-readable input owner. The producer snapshots its expanded inventory before generation, evaluates twice into private staging roots, rejects payload drift or in-flight input changes, emits the existing Cargo handoff shape atomically, and cleans staging through exit and handled-signal traps. The Cargo wrapper now owns only Cargo invocation plus its temporary handoff lifetime.

- [x] T09: `Route checks and packaging through the producer` (status:done)
  - Task ID: T09
  - Goal: Remove duplicate generation/inventory mechanics from the generated-output check and package-fallback preparation while preserving each consumer's domain-specific assertions and static-asset staging.
  - Boundaries (in/out of scope): In — `config/pkl/check-generated.sh`, `scripts/prepare-cli-generated-assets.sh`, producer options needed by those callers, and their focused tests. Out — Nix derivation changes, package fallback layout changes, removal of generation-contract Pkl checks, and static hook/migration/schema ownership.
  - Dependencies: T08
  - Done when: Both callers delegate canonical Pkl generation and inventories to the producer; `check-generated.sh` still runs metadata/contract/negative checks and forbidden-path assertions; packaging still adds static assets and emits its combined checksum inventory without reimplementing Pkl determinism.
  - Verification notes (commands or checks): run producer and wrapper tests; `nix develop -c ./config/pkl/check-generated.sh`; prepare a temporary package fallback twice and compare it; `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/check-generated.sh`, `scripts/prepare-cli-generated-assets.sh`, `scripts/test-check-generated.sh`, `scripts/test-prepare-cli-generated-assets.sh`
  - Evidence: Producer, Cargo-wrapper, generated-output delegation, and package-fallback delegation tests passed; the generated-output check delegated one producer run while retaining metadata, contract, negative-fixture, required-path, and forbidden-path assertions; two real package fallbacks prepared through the producer were recursively identical and their combined inventories verified; `nix develop -c ./config/pkl/check-generated.sh` and `nix run .#pkl-check-generated` passed with 46 files and the unchanged inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; Bash syntax checks, focused duplicate-mechanics searches, and `git diff --check` passed.
  - Notes: The generated-output check projects the producer-owned payload inventory to its established report path format without rehashing generated files. Packaging moves the producer-validated Pkl tree into the unchanged fallback layout, retains the producer's Pkl checksums, adds only static-asset checksums, and does not publish the repository-only canonical-input inventory.

- [x] T10: `Route the Nix generated-input derivation through the producer` (status:done)
  - Task ID: T10
  - Goal: Make `cliGeneratedInput` invoke the same producer and input declaration as shell consumers, removing the last independent two-pass generation and inventory implementation.
  - Boundaries (in/out of scope): In — producer-compatible Nix source selection/runtime inputs, `cliGeneratedInput`, focused flake checks, and removal of superseded inline generation/inventory shell. Out — Crane package topology, Cargo dependency caching policy, release packaging behavior, and unrelated flake simplification.
  - Dependencies: T09
  - Done when: Nix builds the same handoff through the shared producer; Pkl remains absent from Cargo derivations; dependency-only and format derivations remain independent of generated inputs; focused search finds no duplicate producer algorithm; the 46-file payload and complete repository checks remain unchanged.
  - Verification notes (commands or checks): build the focused `cli-generated-input` and `pkl-generated` checks, inspect their handoff inventories, run duplicate-mechanics searches, compare generated output with the pre-plan baseline, then run `nix run .#pkl-check-generated` and `nix flake check`.
  - Completed: 2026-07-30
  - Files changed: `flake.nix`
  - Evidence: `cliGeneratedInput` now invokes `scripts/produce-cli-generated-input.sh` from a producer-compatible Nix source instead of implementing generation, comparison, and inventories inline; the `pkl-generated` check source includes the producer and patches its sandbox shebang. Focused `cli-generated-input` and `pkl-generated` builds passed; the Nix handoff contained 46 valid payload checksums and 31 valid canonical-input checksums; focused source search found no duplicate producer algorithm in `flake.nix`; `nix run .#pkl-check-generated` passed with the unchanged 46-file inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; `nix flake check` and `git diff --check` passed.
  - Notes: The producer publishes to a writable derivation-local staging path before the validated handoff is moved to `$out`. Pkl remains confined to the pre-Cargo producer derivation, while `cargoDepsArgs` and `cli-fmt` remain independent of the generated-input handoff.

## Open questions

None. The request identifies four concrete refactors, the implemented foundation plan supplies their prerequisite catalog and contract checks, and existing decisions fix the required generated behavior and handoff boundaries.

## Validation Report

**Status:** validated  
**Date:** 2026-07-30

### Commands run

- `bash scripts/test-run-cli-cargo.sh` -> exit 0 (Cargo wrapper handoff, forwarding, status propagation, and cleanup tests passed)
- `nix run .#pkl-check-generated` -> exit 0 (46 generated files matched the canonical contract; inventory SHA-256 remained `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`)
- `nix flake check` -> exit 0 (all compatible-system flake checks passed)
- `bash scripts/test-produce-cli-generated-input.sh` -> exit 0 (determinism, drift, missing input, input mutation, subprocess failure, and cleanup cases passed)
- `bash scripts/test-check-generated.sh` -> exit 0 (generated-output check delegation tests passed)
- `bash scripts/test-prepare-cli-generated-assets.sh` -> exit 0 (package-fallback producer delegation tests passed)
- `nix develop -c sh -c 'for module in config/pkl/base/workflow-content.pkl config/pkl/base/workflow-context-sync.pkl config/pkl/base/workflow-change-to-plan.pkl config/pkl/base/workflow-next-task.pkl config/pkl/base/workflow-validate.pkl config/pkl/base/workflow-commit.pkl config/pkl/renderers/workflow-composite.pkl config/pkl/renderers/opencode-content.pkl config/pkl/renderers/claude-content.pkl config/pkl/renderers/pi-content.pkl config/pkl/renderers/generation-contract-check.pkl; do pkl eval -f json "$module" >/dev/null || exit 1; done'` -> exit 0 (shared models, all workflows, all target renderers, and the generation contract evaluated directly)
- Focused source inspections for local constructors, numbered context-sync fragments, prose-internalization helpers, and producer consumers -> exit 0 (only shared constructor definitions and expected call sites remain; forbidden helpers/fragments are absent; all four producer consumers delegate to the canonical script)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Shared workflow constructors -> `workflow-content.pkl` owns `makeDocument` and `packageDocuments`; direct evaluations passed and canonical workflow modules contain only shared-model call sites.
- [x] AC2: Named context-sync composition -> `SyncRole` and `SyncReportRole` provide named semantic sections; numbered fragment searches were empty and the unchanged generated inventory confirms byte preservation.
- [x] AC3: Structured composite rendering -> renderer inspection showed direct structured command, phase, persisted-document, and output rendering; forbidden internalization helpers are absent and direct renderer/contract evaluations passed.
- [x] AC4: Independent workflow migrations -> T04-T07 retain per-workflow three-target baseline diff evidence, and final generation retained the exact accepted 46-file inventory hash.
- [x] AC5: Single producer contract -> Cargo wrapper, generated-output check, packaging fallback, and Nix generated-input derivation all invoke `scripts/produce-cli-generated-input.sh`; inspection found no second producer algorithm.
- [x] AC6: Producer rejection and cleanup behavior -> focused producer and consumer tests passed all specified success, drift, mutation, failure, handoff-shape, and cleanup cases; the normal flake path accepted the handoff.
- [x] AC7: Exact payload and repository checks -> generated output remained 46 files with baseline inventory SHA-256 `e3b340b0daae030bbcead618514fe256e7f577eb9ee2a032eba7d6045e647777`; every full-validation command passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
