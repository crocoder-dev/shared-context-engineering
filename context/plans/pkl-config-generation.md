# Plan: pkl-config-generation

## 1) Change summary
Create a deterministic Pkl-based generation workflow that produces both configuration trees from one shared canonical source, with clear ownership boundaries for generated vs non-generated files.

## 2) Success criteria
- A single documented generation command updates both target trees from shared Pkl definitions.
- Generated file outputs are deterministic (stable content and paths across repeated runs).
- Scope is limited to authored config content (agents, commands, skills, shared library file); runtime artifacts remain unmanaged by Pkl generation.
- A repo check can detect when generated outputs are out of date.
- Regeneration and verification steps are documented for future sessions.

## 3) Constraints and non-goals
- In scope: planning and config-generation assets only.
- In scope: shared canonical content, per-target frontmatter/rendering differences, and output path mapping.
- Out of scope: changing runtime dependency artifacts and package manager lockfiles.
- Out of scope: changing application code.
- Non-goal: introducing additional generation systems beyond Pkl.

## 4) Task stack (T01..T08)
- [x] T01: Define generator architecture and file ownership boundary (status:done)
  - Task ID: T01
  - Goal: Lock shared-source approach, generated path coverage, and explicit exclusions.
  - Boundaries (in/out of scope):
    - In: generation design, target file matrix, generated/non-generated ownership list.
    - Out: implementation of generation modules.
  - Done when:
    - Canonical source approach and ownership boundaries are written and unambiguous.
    - Generated path matrix includes both target trees and file class coverage.
  - Verification notes (commands or checks):
    - Review matrix for one-to-one mapping completeness across planned generated outputs.
  - Evidence:
    - Decision record captured in `context/decisions/2026-02-28-pkl-generation-architecture.md` with canonical-source architecture, generated path matrix, and generated/non-generated ownership boundaries.

- [x] T02: Scaffold shared Pkl base content module(s) (status:done)
  - Task ID: T02
  - Goal: Create Pkl module(s) that hold shared canonical text blocks and reusable content primitives.
  - Boundaries (in/out of scope):
    - In: shared content definitions for generated authored files.
    - Out: target-specific frontmatter serialization and output mapping.
  - Done when:
    - Shared content module(s) exist with stable identifiers and no target-specific formatting.
  - Verification notes (commands or checks):
    - Static review confirms content primitives can be consumed by multiple target renderers.
  - Evidence:
    - Added shared canonical base module `config/pkl/base/shared-content.pkl` with stable content-unit IDs for all planned generated authored classes (3 agents, 6 commands, 9 skills, 1 library file).
    - Confirmed module is target-agnostic (no `.opencode` / `.claude` formatting or path mapping content), preserving T02 boundary.
    - Added `pkl` to `flake.nix` dev shell and validated module evaluation with `nix develop -c pkl eval config/pkl/base/shared-content.pkl`.

- [x] T03: Implement target-specific renderer/frontmatter helpers (status:done)
  - Task ID: T03
  - Goal: Add transformation helpers that apply target-specific metadata/frontmatter while reusing shared canonical content.
  - Boundaries (in/out of scope):
    - In: rendering helpers and formatting functions for each target.
    - Out: final output path emission.
  - Done when:
    - Renderer helpers produce valid target-formatted content from shared inputs.
  - Verification notes (commands or checks):
    - Spot-check rendered outputs for metadata schema correctness per target.
  - Evidence:
    - Added target-specific renderer helper modules at `config/pkl/renderers/opencode-content.pkl` and `config/pkl/renderers/claude-content.pkl` that transform canonical units into target frontmatter + markdown document structures for agents, commands, and skills.
    - Tightened renderer structure by extracting shared renderer contracts/descriptions to `config/pkl/renderers/common.pkl` and per-target metadata tables to `config/pkl/renderers/opencode-metadata.pkl` and `config/pkl/renderers/claude-metadata.pkl`, reducing duplication and isolating metadata concerns.
    - Added explicit metadata key-coverage assertions in `config/pkl/renderers/metadata-coverage-check.pkl` to fail fast when canonical slugs and metadata tables drift.
    - Verified metadata coverage check with `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`.
    - Verified both renderer modules evaluate successfully with `nix develop -c pkl eval config/pkl/renderers/opencode-content.pkl` and `nix develop -c pkl eval config/pkl/renderers/claude-content.pkl`.
    - Ran a lightweight repository build/check gate using `nix flake check --no-build`.

- [x] T04: Implement multi-file output mapping and generator entrypoint (status:done)
  - Task ID: T04
  - Goal: Define `output.files` mapping for all generated authored files and expose a single generation entrypoint.
  - Boundaries (in/out of scope):
    - In: multi-file mapping, entrypoint module, deterministic path keys.
    - Out: CI enforcement.
  - Done when:
    - One entrypoint evaluates to all planned generated files in both target trees.
    - Output map excludes declared non-generated artifacts.
  - Verification notes (commands or checks):
  - Run documented Pkl multi-file generation command and confirm files are emitted at expected paths.
  - Evidence:
    - Added generation entrypoint module `config/pkl/generate.pkl` with `output.files` mappings for all planned authored classes across both target trees (agents, commands, skills, shared lib).
    - Verified entrypoint evaluation with `nix develop -c pkl eval config/pkl/generate.pkl`.
    - Verified multi-file emission and expected path coverage with `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` (38 files emitted under generated scope paths).
    - Confirmed task-level checks with `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` and lightweight build gate `nix flake check --no-build`.

- [x] T05: Document generation and regeneration workflow (status:done)
  - Task ID: T05
  - Goal: Add concise docs for generation command, prerequisites, and ownership expectations.
  - Boundaries (in/out of scope):
    - In: command usage, scope boundaries, troubleshooting notes.
    - Out: policy changes beyond this generator.
  - Done when:
    - A contributor can regenerate outputs using only documented instructions.
  - Verification notes (commands or checks):
    - Dry-run docs review: execute listed steps in order and confirm no missing prerequisite.
  - Evidence:
    - Added `config/pkl/README.md` with prerequisite, ownership boundary, regeneration workflow, and troubleshooting guidance for contributors.
    - Executed documented workflow commands in order: `nix develop -c pkl eval config/pkl/generate.pkl`, `nix develop -c pkl eval -m context/tmp/pkl-generated config/pkl/generate.pkl`, `nix develop -c pkl eval -m . config/pkl/generate.pkl`, and `git status --short config/.opencode config/.claude`.
    - Ran task-level guard checks: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` and lightweight build gate `nix flake check --no-build`.

- [x] T06: Add stale-output detection check (status:done)
  - Task ID: T06
  - Goal: Add a repeatable check that fails when generated files differ from committed outputs.
  - Boundaries (in/out of scope):
    - In: script/check target definition and expected pass/fail behavior.
    - Out: broad CI platform redesign.
  - Done when:
    - Check reliably reports clean state after regeneration and drift when outputs are stale.
  - Verification notes (commands or checks):
    - Run regeneration then run drift check; capture pass/fail evidence for both clean and modified states.
  - Evidence:
    - Added deterministic stale-output check script at `config/pkl/check-generated.sh` that regenerates into a temporary directory and compares generated-owned paths against committed outputs using `git diff --no-index`.
    - Fixed broken regeneration output by synchronizing `config/pkl/base/shared-content.pkl` canonical bodies with the current `.opencode` authored command/agent/skill source content (frontmatter stripped), then regenerating both target trees.
    - Documented stale-output check command in `config/pkl/README.md` (`nix develop -c ./config/pkl/check-generated.sh`).
    - Captured clean-state pass evidence with `nix develop -c ./config/pkl/check-generated.sh` after regeneration.
    - Captured stale-state fail evidence by intentionally modifying the generated Claude `next-task` command file, running `nix develop -c ./config/pkl/check-generated.sh` (expected non-zero), then restoring with `nix develop -c pkl eval -m . config/pkl/generate.pkl`.
    - Ran task-level guard checks: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` and lightweight build gate `nix flake check --no-build`.

- [ ] T07: Optional generated-file safety marker (status:todo)
  - Task ID: T07
  - Goal: Add a lightweight header/marker strategy to discourage manual edits to generated files.
  - Boundaries (in/out of scope):
    - In: marker format and insertion strategy.
    - Out: enforcement tooling beyond warning-level guidance.
  - Done when:
    - Marker convention is applied consistently (or explicitly declined with rationale).
  - Verification notes (commands or checks):
    - Inspect sample generated files to verify marker presence/consistency.

- [ ] T08: Validation and cleanup (status:todo)
  - Task ID: T08
  - Goal: Run final end-to-end validation, ensure docs and generated outputs are aligned, and clean temporary artifacts.
  - Boundaries (in/out of scope):
    - In: full planned checks, final consistency review, cleanup.
    - Out: new feature work.
  - Done when:
    - All plan success criteria have evidence.
    - Generation, stale-output detection, and documentation all agree on current state.
    - Temporary planning/execution artifacts are cleaned.
  - Verification notes (commands or checks):
    - Run full planned validation checks and capture evidence in the plan update.

## 5) Open questions
- None. Scope choice confirmed: shared canonical source with authored-file generation only.
