# Plan: claude-markdown-workflow-results

## Change summary

Replace YAML-only phase result handoffs in the generated Claude workflow skill packages with Markdown contracts that preserve the same status variants and command-to-skill data. OpenCode and Pi keep their current YAML contracts, while Claude commands continue rendering the internal phase results into the existing user-facing workflow responses instead of exposing raw handoffs.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Every generated Claude skill that currently returns a YAML result (`sce-context-load`, `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, and `sce-atomic-commit`) instead returns one Markdown result with the same status variants and all fields required by its invoking command.
  - Validate: generate a temporary payload with `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect the five Claude skill entrypoints and their result-contract references for Markdown-only return instructions, complete status layouts, and command-readable labels.
- [x] AC2: Generated Claude workflow commands consume the Markdown phase handoffs and still render only their established user-facing clarification, readiness, execution, commit, and continuation responses; raw internal phase reports are not presented as the final workflow response.
  - Validate: inspect the four generated Claude commands together with the five converted skill contracts and confirm every status branch and referenced value has an unambiguous Markdown source and an existing user-facing rendering branch.
- [x] AC3: OpenCode and Pi retain their current YAML result contracts and workflow behavior byte-for-byte while only Claude receives the target-specific Markdown variants.
  - Validate: generate before/after payloads from the implementation base and working tree, then run `diff -rq` over `config/.opencode` and `config/.pi`; only `config/.claude` workflow Markdown may differ.
- [x] AC4: Exact metadata coverage accepts the target-specific Claude contract paths, rejects missing or extra workflow documents per target, and deterministic ephemeral generation passes.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix run .#pkl-check-generated`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` to describe Claude's target-specific Markdown phase-result contract while preserving the canonical shared workflow semantics.
- Reconcile the stale three-workflow/seven-skill inventory claims in `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` with the implemented four-workflow/eight-skill generator inventory.

## Constraints and non-goals

- **In scope:** Canonical Pkl workflow/result-contract modeling, Claude renderer overrides, Claude workflow skill references and instructions, exact target inventory coverage, and focused generation documentation.
- **Out of scope:** Changes to workflow status semantics, command sequencing, implementation gates, user-facing continuation layouts, Rust CLI behavior, Claude hooks/settings, or generated repository target trees.
- **Constraints:** Keep self-contained skill packages and deterministic package-local references; preserve the project-root `.pi/` baseline and OpenCode output; author changes in canonical Pkl and renderers rather than generated payloads; retain every field the invoking command needs to branch or render its response.
- **Non-goal:** Replace YAML contracts for OpenCode or Pi, or convert validation/context-sync reports that already use Markdown.

## Assumptions

- “Claude workflow” covers all five YAML-returning skills currently generated for Claude across `/change-to-plan`, `/next-task`, and `/commit`, not only the phase that exposed the reported result.
- Markdown contracts remain structured through fixed headings, status labels, and repeated labeled sections so invoking commands can consume them without YAML.
- `/validate`, `sce-validation`, `sce-task-context-sync`, and `sce-plan-context-sync` need no result-format conversion because their current handoffs are already Markdown.

## Task stack

- [x] T01: `Render Claude phase results as Markdown` (status:done)
  - Task ID: T01
  - Goal: Give every YAML-returning Claude workflow skill a complete target-specific Markdown result contract while preserving shared workflow semantics and all non-Claude outputs.
  - Boundaries (in/out of scope): In — target-specific result-contract modeling for the five affected skills, Claude skill entrypoint/reference rendering, command compatibility, exact per-target inventory checks, and focused Pkl generation documentation. Out — status/field redesign, OpenCode or Pi behavior changes, validation/context-sync report changes, generated target-tree edits, and runtime code.
  - Dependencies: none
  - Done when: A temporary generated Claude payload contains Markdown-only result instructions and `.md` contracts for all five affected skills; every command branch can source its required values; exact coverage passes with target-specific inventories; before/after comparison shows no OpenCode or Pi changes; focused generation checks pass.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/{claude-content,opencode-content,pi-content,metadata-coverage-check}.pkl`; generate temporary before/after payloads and inspect/diff the three target workflow trees; `nix run .#pkl-check-generated`; `git diff --check -- config/pkl`.
  - Completed: 2026-07-28
  - Files changed: `config/pkl/renderers/claude-workflow-results.pkl`, `config/pkl/renderers/claude-content.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/README.md`, `context/plans/claude-markdown-workflow-results.md`
  - Evidence: focused evaluation of `claude-content.pkl`, `opencode-content.pkl`, `pi-content.pkl`, and `metadata-coverage-check.pkl` passed; temporary HEAD/current payload comparison showed byte-identical OpenCode and Pi trees and only the five intended Claude skill packages changed; generated Claude packages contained all five `.md` contracts and no YAML instructions or YAML contract references; `nix run .#pkl-check-generated` passed with 85 files and inventory SHA-256 `f465ad7139a66f8530581186b8db77405afa7203be9a6ce9e6de9624e238cd0b`; `git diff --check -- config/pkl` passed.
  - Notes: Canonical Pi/OpenCode YAML contracts remain unchanged; Claude performs package-local path and content overrides in its renderer.

## Open questions

None. The reported failure identifies YAML as a Claude-specific transport problem, and the existing Markdown validation/synchronization reports establish a repository-local alternative without changing workflow semantics for other targets.

## Validation Report

**Status:** failed  
**Date:** 2026-07-28

### Commands run

- `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` -> exit 0 (exact target-specific metadata coverage passed)
- `nix run .#pkl-check-generated` -> exit 0 (85-file deterministic inventory passed with SHA-256 `f465ad7139a66f8530581186b8db77405afa7203be9a6ce9e6de9624e238cd0b`)
- `nix flake check` -> exit 1 (`sce-cli-generated-input` could not find the untracked imported module `config/pkl/renderers/claude-workflow-results.pkl` in the Git-backed flake source)
- `nix run .#pkl-generate -- "$(mktemp -d)"` with generated Claude path inspection -> exit 0 (all four workflow commands and all five converted skill packages were generated)
- temporary HEAD/current `pkl eval` generation, non-Claude `diff -rq`, and shell-only Claude contract/command inspection -> exit 0 (OpenCode and Pi were byte-identical; all five Markdown contracts exposed the required statuses; Claude commands exposed every converted handoff branch)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Every affected generated Claude skill returns a Markdown result with preserved statuses and command-readable fields -> generated-payload inspection confirmed Markdown-only entrypoints, `.md` contracts, and every required status for all five skills.
- [x] AC2: Claude commands consume Markdown handoffs while retaining user-facing rendering -> generated command inspection confirmed branches for every converted phase status and retained response-rendering instructions.
- [x] AC3: OpenCode and Pi remain byte-for-byte unchanged -> HEAD/current ephemeral generation followed by `diff -rq` passed for both target trees.
- [x] AC4: Exact metadata coverage and deterministic ephemeral generation pass -> metadata evaluation and `nix run .#pkl-check-generated` both passed.

### Failed checks and follow-ups

- `nix flake check`: the Git-backed flake source omitted the new untracked renderer, so `sce-cli-generated-input` failed with `Cannot find module .../config/pkl/renderers/claude-workflow-results.pkl`; required: add `config/pkl/renderers/claude-workflow-results.pkl` to the Git index (or otherwise make it part of the Git-backed flake source), then rerun final validation.

### Residual risks

- Repository-wide flake checks have not passed with the new renderer visible to the Git-backed Nix source.

### Retry

After repairs, rerun:

`/validate context/plans/claude-markdown-workflow-results.md`
