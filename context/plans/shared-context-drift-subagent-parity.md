# Plan: shared-context-drift-subagent-parity

## 1) Change summary
Ensure the Shared Context Drift agent is configured as an internal OpenCode subagent (`mode: subagent`, `hidden: true`) and establish behavior-parity mapping for Claude Code so the same workflow remains non-user-facing and subagent-invoked.

## 2) Success criteria
- OpenCode generated output for Shared Context Drift includes frontmatter/state equivalent to `mode: subagent` and `hidden: true`.
- Claude-side generated output implements the closest supported behavior-parity pattern for an internal/non-primary agent flow (subagent invocation path preserved).
- Pkl canonical sources and renderer metadata remain deterministic and pass metadata coverage checks.
- Regeneration and stale-output parity checks pass with no unmanaged drift.
- Context docs capture current-state behavior and any explicit OpenCode vs Claude capability difference.

## 3) Constraints and non-goals
- In scope: planning and generated config/context artifacts under `config/` and `context/`.
- In scope: OpenCode frontmatter enforcement plus Claude parity mapping informed by current docs.
- Out of scope: application runtime feature changes unrelated to agent configuration.
- Out of scope: introducing new orchestration systems beyond existing Pkl generation pipeline.
- Non-goal: forcing unsupported Claude frontmatter keys if native equivalents do not exist.

## 4) Task stack (T01..T05)
- [x] T01: Confirm canonical metadata contract for Shared Context Drift (status:done)
  - Task ID: T01
  - Goal: Locate and normalize the canonical metadata/source entries that drive Shared Context Drift generation for both targets.
  - Boundaries (in/out of scope):
    - In: `config/pkl/base/*`, `config/pkl/renderers/*`, and generated target mapping references for the drift agent slug.
    - Out: changing unrelated agent/command/skill metadata.
  - Done when:
    - One canonical slug/key path for Shared Context Drift is confirmed and documented for both targets.
    - Ownership of where OpenCode and Claude frontmatter is derived is explicit.
  - Verification notes (commands or checks):
    - Inspect renderer metadata tables and coverage check inputs for the drift agent key.
  - Implementation notes:
    - Canonical slug/key path is `shared-content.pkl` agent key `shared.agents["shared-context-drift"]` (`config/pkl/base/shared-content.pkl`).
    - OpenCode frontmatter ownership is `config/pkl/renderers/opencode-content.pkl` + `config/pkl/renderers/opencode-metadata.pkl` (`agentDisplayNames`, `agentDescriptions`, `agentColors`, `agentPermissionBlocks`).
    - Claude frontmatter ownership is `config/pkl/renderers/claude-content.pkl` + `config/pkl/renderers/claude-metadata.pkl` (`agentDescriptions`, `agentColors`, `agentTools`).
    - Coverage wiring for both targets is enforced by `config/pkl/renderers/metadata-coverage-check.pkl` (`opencodeAgentCoverage` and `claudeAgentCoverage`).
    - Generated target mapping reference remains centralized in `config/pkl/generate.pkl` (`config/.opencode/agent/\(document.title).md` and `config/.claude/agents/\(slug).md`).

- [x] T02: Enforce OpenCode subagent-hidden configuration in generator inputs (status:done)
  - Task ID: T02
  - Goal: Update canonical/OpenCode metadata so generated Shared Context Drift output resolves to `mode: subagent` and `hidden: true`.
  - Boundaries (in/out of scope):
    - In: OpenCode renderer metadata/frontmatter mapping for Shared Context Drift.
    - Out: manual edits directly in generated output paths.
  - Done when:
    - Generated OpenCode Shared Context Drift artifact contains `mode: subagent` and `hidden: true`.
    - Metadata coverage guard still passes.
  - Verification notes (commands or checks):
    - `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - Inspect generated Shared Context Drift file under `config/.opencode/`.
  - Implementation notes:
    - Added `agentBehaviorBlocks` in `config/pkl/renderers/opencode-metadata.pkl` and set `shared-context-drift` to emit `mode: subagent` and `hidden: true`.
    - Updated `config/pkl/renderers/opencode-content.pkl` to include behavior block interpolation in agent frontmatter before permission mapping.
    - Regenerated outputs; `config/.opencode/agent/Shared Context Drift.md` now includes `mode: subagent` and `hidden: true` in generated frontmatter.

- [x] T03: Implement Claude behavior-parity mapping for internal subagent usage (status:done)
  - Task ID: T03
  - Goal: Apply the Claude-supported equivalent pattern so Shared Context Drift is treated as subagent/internal flow with behavior parity to OpenCode intent.
  - Boundaries (in/out of scope):
    - In: Claude agent metadata/frontmatter fields and instructions necessary to preserve internal invocation behavior.
    - Out: inventing unsupported Claude keys or changing unrelated Claude agents.
  - Done when:
    - Claude generated artifact uses supported fields and structure consistent with current Claude docs.
    - Parity intent (non-primary/manual usage minimized, subagent workflow preserved) is explicit in config/instructions.
  - Verification notes (commands or checks):
    - Re-check Claude docs references used for mapping decision.
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - Inspect generated Shared Context Drift file under `config/.claude/`.
  - Implementation notes:
    - Claude subagent docs confirm supported subagent frontmatter includes fields like `name`, `description`, `tools`, and `model`, with no `hidden`/`mode` equivalent; parity is encoded via supported description/body guidance.
    - Added `agentSystemPreambleBlocks` in `config/pkl/renderers/claude-metadata.pkl` and a shared `agentBodyBySlug` composition in `config/pkl/renderers/claude-content.pkl` to inject delegated/internal usage instructions for `shared-context-drift` only.
    - Updated `shared-context-drift` Claude description metadata to explicitly prefer delegated command/Task routing over primary/manual invocation.
    - Regenerated outputs; `config/.claude/agents/shared-context-drift.md` now contains delegation-mode guidance in both frontmatter description and body while remaining within Claude-supported fields.

- [x] T04: Update context to record current-state parity decision and rationale (status:done)
  - Task ID: T04
  - Goal: Record the finalized OpenCode vs Claude mapping and any capability gap as durable current-state context.
  - Boundaries (in/out of scope):
    - In: focused updates to `context/architecture.md`, `context/patterns.md`, and/or a new decision record if needed.
    - Out: historical narrative beyond required rationale and current-state behavior.
  - Done when:
    - Context states how Shared Context Drift is represented in both targets and why.
    - Future sessions can reproduce the mapping without rediscovery.
  - Verification notes (commands or checks):
    - Manual read-through: context matches generated behavior and references actual source files.
  - Implementation notes:
    - Updated `context/architecture.md` with a current-state parity section documenting canonical source ownership, OpenCode `mode: subagent` + `hidden: true` emission path, and Claude capability-gap mapping via delegated guidance.
    - Updated `context/patterns.md` with the reusable parity pattern: encode internal behavior per-target capability and validate against generated OpenCode/Claude artifacts.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run full checks, confirm no generated drift, and ensure plan/task evidence is complete.
  - Boundaries (in/out of scope):
    - In: generation checks, stale-output parity check, final context consistency pass.
    - Out: new feature additions.
  - Done when:
    - All success criteria have evidence.
    - Generated outputs are clean and deterministic.
    - Any temporary investigation artifacts under `context/tmp/` are removed or intentionally retained with rationale.
  - Verification notes (commands or checks):
    - `nix develop -c pkl eval config/pkl/generate.pkl`
    - `nix develop -c ./config/pkl/check-generated.sh`
    - `nix flake check --no-build`
  - Implementation notes:
    - Executed all three verification commands successfully on 2026-02-28; generation evaluation completed, stale-output parity check reported `Generated outputs are up to date.`, and `nix flake check --no-build` passed with only the expected incompatible-systems warning.
    - `context/tmp/t04-generated/` remains intentionally retained as prior task evidence material and does not introduce generated drift.

## 5) Open questions
- None.

## 6) Validation report (T05)
- Commands run:
  - `nix develop -c pkl eval config/pkl/generate.pkl` (exit 0)
  - `nix develop -c ./config/pkl/check-generated.sh` (exit 0; key output: `Generated outputs are up to date.`)
  - `nix flake check --no-build` (exit 0; key output: flake outputs evaluate cleanly, with expected warning that incompatible systems were omitted unless `--all-systems` is requested)
- Failed checks and follow-ups:
  - None.
- Success-criteria verification summary:
  - OpenCode generated Shared Context Drift output includes `mode: subagent` and `hidden: true` (implemented in T02; unchanged by T05 checks).
  - Claude-side generated output uses supported parity mapping for internal/delegated invocation (implemented in T03; unchanged by T05 checks).
  - Pkl canonical sources and renderer metadata remain deterministic under regeneration/check-generated validation.
  - Regeneration and stale-output parity checks passed with no unmanaged drift.
  - Context docs remain aligned with code truth (`context/architecture.md` and `context/patterns.md` still reflect current-state parity mapping).
- Temporary artifacts and cleanup:
  - `context/tmp/t04-generated/` intentionally retained as evidence material from prior task work; no cleanup required for T05 scope.
- Residual risks:
  - `nix flake check --no-build` does not validate omitted incompatible systems (`aarch64-darwin`, `aarch64-linux`, `x86_64-darwin`) in this environment.
