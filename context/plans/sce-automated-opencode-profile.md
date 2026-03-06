# Plan: sce-automated-opencode-profile

## 1) Change summary
Create a non-interactive OpenCode configuration profile at `config/automated/.opencode/**` by adding automated Pkl variants that remove yes/no or accept/confirm gates while preserving the current manual profile at `config/.opencode/**`.

## 2) Success criteria
- `config/.opencode/**` generation behavior remains unchanged for the current manual workflow.
- A new generated tree exists at `config/automated/.opencode/**` with deterministic outputs.
- Automated profile permissions remove `ask`-style gating in agent frontmatter policy blocks.
- Automated profile command/skill/agent bodies remove explicit interactive approval or confirmation prompts and replace them with deterministic behavior.
- Pkl metadata coverage and generation checks pass after introducing the automated variant.
- Documentation explains manual vs automated profile ownership and regeneration behavior.

## 3) Constraints and non-goals
- In scope: Pkl canonical content, renderer metadata/content variants, generation output mapping, and docs for the new automated profile.
- In scope: deterministic non-interactive behavior for SCE planning/execution/drift flows in automated outputs.
- Out of scope: changing application code outside config generation assets.
- Out of scope: removing existing interactive safety gates from the current manual profile.
- Non-goal: redesigning SCE architecture; this change only introduces a parallel automated profile.

## 4) Task stack (T01..T06)
- [x] T01: Define automated profile contract and deterministic gate policy (status:done)
  - Task ID: T01
  - Goal: Codify the exact manual-vs-automated split, deterministic fallback policies, and blocked vs auto-continue behavior before implementation.
  - Boundaries (in/out of scope):
    - In: policy decisions for permission gates, readiness behavior, missing-context handling, and scope-expansion handling in automation mode.
    - Out: file-level implementation changes.
  - Done when:
    - A documented policy block exists in the plan (or linked context file) covering all identified gate classes.
    - The policy includes deterministic defaults for plan selection, drift mode, and missing-detail handling.
  - Verification notes (commands or checks):
    - Plan review confirms every previously identified interactive gate category has an explicit automated policy.
  - Evidence:
    - Created `context/sce/automated-profile-contract.md` with 10 gate categories (P1-P10)
    - Documented deterministic defaults table for all scenarios
    - Defined automated profile constraints (7 safety constraints)
    - Policy covers: permission gates, bootstrap approval, clarification gate, implementation stop, readiness confirmation, multi-task approval, scope expansion, commit staging, drift fix application, plan selection

- [x] T02: Add automated OpenCode metadata variant with non-interactive permissions (status:done)
  - Task ID: T02
  - Goal: Add `opencode-automated-metadata.pkl` that mirrors current metadata coverage while switching interactive permission values to deterministic non-interactive values.
  - Boundaries (in/out of scope):
    - In: automated metadata tables for agent permission blocks and any automated-specific frontmatter needs.
    - Out: canonical manual metadata behavior changes.
  - Done when:
    - Automated metadata composes valid permission blocks for all shared SCE agents.
    - No manual metadata entries are altered for current `config/.opencode/**` generation.
  - Verification notes (commands or checks):
    - Evaluate metadata coverage checks including automated variant coverage assertions.
  - Evidence:
    - Created `config/pkl/renderers/opencode-automated-metadata.pkl` with P1 permission transformations
    - Applied: `default: ask → allow`, `external_directory: ask → block`, `doom_loop: ask → block`, `skill["*"]: ask → allow`
    - All 3 agents (shared-context-plan, shared-context-code, shared-context-drift) have automated permission blocks
    - Pkl evaluation successful
    - Metadata coverage check passed
    - Manual metadata unchanged (verified via git status)
    - Generator runs successfully with new metadata file present

- [x] T03: Add automated shared-content variant with deterministic replacements for interactive prompts (status:done)
  - Task ID: T03
  - Goal: Create automated content source(s) that preserve structure and intent while replacing ask/confirm gates with deterministic execution behavior.
  - Boundaries (in/out of scope):
    - In: automated variants of affected agent/command/skill canonical bodies and deterministic policy constants.
    - Out: modifications to existing manual canonical content used by current generation targets.
  - Done when:
    - All mapped interactive gate blocks in SCE agents/commands/skills have automated replacements.
    - Automated content still satisfies one-task boundaries, validation requirements, and context-sync obligations.
  - Verification notes (commands or checks):
    - Diff review confirms automated variant performs targeted gate replacement only and retains core SCE constraints.
  - Evidence:
    - Created `config/pkl/base/shared-content-automated.pkl` (812 lines)
    - Applied P2-P10 gate policy replacements:
      - P2 (Bootstrap): "ask once for approval" → "stop with error requiring manual bootstrap"
      - P3 (Clarification): "ask 1-3 targeted questions" → "stop with structured error listing unresolved items"
      - P4 (Implementation stop): "pause and prompt" → "log intent and proceed without waiting"
      - P5 (Readiness): "ask explicit confirmation" → "auto-pass when conditions met, auto-block otherwise"
      - P6 (Multi-task): "confirm explicit approval" → "not supported, stop with error"
      - P7 (Scope expansion): "stop and ask for approval" → "stop with structured error"
      - P8 (Commit staging): "prompt the user" → "skip prompt, validate staged content"
      - P9 (Drift fix): "ask user what to do" → "auto-apply with logging"
      - P10 (Plan selection): "ask user to choose" → "stop with error if ambiguous"
    - All 22 ContentUnit definitions preserved (3 agents, 7 commands, 10 skills, 1 library)
    - Pkl evaluation successful
    - Manual shared-content.pkl unchanged (verified via git status)
    - No interactive gate patterns remain (verified via grep)
    - All content unit keys match manual version (verified via diff)

- [x] T04: Add automated renderer wiring and generate `config/automated/.opencode/**` outputs (status:done)
  - Task ID: T04
  - Goal: Add automated renderer entrypoints and generator mappings so automated artifacts are emitted alongside existing manual outputs.
  - Boundaries (in/out of scope):
    - In: new renderer module(s), `generate.pkl` output mapping additions, and automated library copy mapping.
    - Out: changing existing manual output paths or removing current generated paths.
  - Done when:
    - Generator emits automated agents/commands/skills/library under `config/automated/.opencode/**`.
    - Manual generation targets continue to render to existing locations unchanged.
  - Verification notes (commands or checks):
    - Run generator eval and confirm both manual and automated target trees are produced with expected path coverage.
  - Evidence:
    - Created `config/pkl/renderers/opencode-automated-content.pkl` mirroring manual renderer structure
    - Updated `config/pkl/generate.pkl` with automated output mappings for agents/commands/skills/lib
    - Generator successfully produced both manual and automated outputs
    - Automated output structure: 3 agents, 7 commands, 10 skills, 1 lib file
    - Manual outputs unchanged (verified via `git diff config/.opencode/`)
    - Parity check passed: `nix run .#pkl-check-generated`
    - Flake checks passed: `nix flake check`
    - Automated profile has non-interactive permissions (`default: allow` instead of `ask`)
    - Automated profile includes "(automated profile)" marker in agent bodies

- [x] T05: Update parity checks and documentation for dual-profile generation ownership (status:done)
  - Task ID: T05
  - Goal: Ensure coverage/parity checks and contributor docs include automated profile expectations.
  - Boundaries (in/out of scope):
    - In: metadata coverage check updates, generated-ownership docs, and regeneration instructions referencing automated outputs.
    - Out: CI policy redesign beyond what is needed to keep generated outputs in sync.
  - Done when:
    - Coverage checks fail fast when automated metadata/content mappings drift.
    - `config/pkl/README.md` explains both profile outputs and deterministic regeneration workflow.
  - Verification notes (commands or checks):
    - Execute coverage checks and stale-output parity check to confirm automated paths are included.
  - Evidence:
    - Updated `config/pkl/check-generated.sh` to include automated profile paths in parity check array (4 new paths)
    - Updated `config/pkl/README.md` with dual-profile documentation:
      - Added "Manual vs Automated profiles" section explaining purpose, behavior, use cases, and sources for each profile
      - Updated ownership boundary section to include `config/automated/.opencode/**`
      - Updated regeneration workflow documentation to mention both profiles
      - Updated `git status` example to include `config/automated/.opencode`
    - Updated `config/pkl/renderers/metadata-coverage-check.pkl` with automated profile coverage validation:
      - Imported `shared-content-automated.pkl` and `opencode-automated-metadata.pkl`
      - Added `opencodeAutomatedAgentCoverage` for all 3 agents
      - Added `opencodeAutomatedCommandCoverage` for all 7 commands
      - Added `opencodeAutomatedSkillCoverage` for all 10 skills
    - All verification checks passed:
      - `nix run .#pkl-check-generated` passed with automated paths included
      - `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` evaluated successfully with automated coverage checks
      - `nix flake check` passed all repository checks

- [x] T06: Validation and cleanup (status:done)
  - Task ID: T06
  - Goal: Run final validation, verify success criteria evidence, and ensure no temporary artifacts remain.
  - Boundaries (in/out of scope):
    - In: full planned checks, final generated output verification, and plan evidence updates.
    - Out: additional feature work beyond automated profile introduction.
  - Done when:
    - All success criteria have evidence recorded in the plan.
    - Generation/parity checks pass with automated outputs present.
    - Context references remain current and concise.
  - Verification notes (commands or checks):
    - Run full repository validation baseline for this scope (`pkl` evals/checks and flake checks) and record exit codes/key outputs.
  - Evidence:
    - Full validation suite passed:
      - Generator evaluation: SUCCESS
      - Parity check (`nix run .#pkl-check-generated`): "Generated outputs are up to date"
      - Metadata coverage check: SUCCESS
      - Flake checks (`nix flake check`): All checks passed
    - No temporary artifacts in context/tmp/ (only .gitignore present)
    - Automated profile structure verified:
      - 3 agents in config/automated/.opencode/agent/
      - 7 commands in config/automated/.opencode/command/
      - 10 skills in config/automated/.opencode/skills/
      - 1 lib file (drift-collectors.js)
    - All 6 success criteria have evidence in plan:
      - SC1 (manual profile unchanged): T04 evidence
      - SC2 (automated profile exists): T04 evidence + T06 verification
      - SC3 (permissions non-interactive): T02 + T04 evidence
      - SC4 (bodies deterministic): T03 evidence
      - SC5 (checks pass): T05 evidence + T06 validation
      - SC6 (docs complete): T05 evidence
    - Context references verified current and concise (overview.md, glossary.md, context-map.md all reference automated profile)

## 5) Open questions
- None. Scope and architecture direction are confirmed: preserve manual canonical behavior and add a parallel automated variant.

## 6) Final Validation Report

### Commands run
1. `nix develop -c pkl eval config/pkl/generate.pkl` - Exit code: 0 (SUCCESS)
2. `nix run .#pkl-check-generated` - Exit code: 0 (SUCCESS, "Generated outputs are up to date")
3. `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` - Exit code: 0 (SUCCESS)
4. `nix flake check` - Exit code: 0 (SUCCESS, all checks passed)
5. `cargo fmt --check` (cli/) - Exit code: 0 (SUCCESS, no formatting issues)
6. `cargo clippy --all-targets --all-features` (cli/) - Exit code: 0 (SUCCESS, warnings only)

### Key outputs
- All Pkl generation, parity, and coverage checks passed
- Automated profile structure verified: 3 agents, 7 commands, 10 skills, 1 lib file
- Manual profile unchanged (verified via parity check)
- No temporary artifacts in context/tmp/
- Context files (overview.md, glossary.md, context-map.md) reference automated profile correctly

### Failed checks and follow-ups
- None. All validation checks passed successfully.

### Success-criteria verification summary
✅ **SC1**: Manual profile unchanged - Evidence in T04 (git diff verification, parity check passed)
✅ **SC2**: Automated profile exists - Evidence in T04 + T06 (structure verified: 3/7/10/1)
✅ **SC3**: Non-interactive permissions - Evidence in T02 (permission mappings) + T04 (verification)
✅ **SC4**: Deterministic behavior - Evidence in T03 (P2-P10 gate replacements, grep verification)
✅ **SC5**: Checks pass - Evidence in T05 + T06 (all generation/parity/coverage checks passed)
✅ **SC6**: Documentation complete - Evidence in T05 (README updated with dual-profile docs)

### Residual risks
None. All success criteria met with comprehensive evidence.

### Plan status
✅ **All tasks complete (T01-T06)**
✅ **All success criteria verified with evidence**
✅ **All validation checks passed**
✅ **Context files current and aligned**
