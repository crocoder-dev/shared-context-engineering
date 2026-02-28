# Plan: config-opencode-sync-via-flake

## 1) Change summary
Add a deterministic Nix flake command that (1) regenerates `config/`, (2) replaces `config/` by deleting the current tree and restoring the regenerated tree, and then (3) syncs `config/.opencode/` into repository-root `.opencode/`. The workflow is intentionally destructive for `config/` and should be safe by construction through staged regeneration before replacement.

## 2) Success criteria
- A single flake entrypoint exists and runs as `nix run .#sync-opencode-config`.
- Running the command fully replaces `config/` with regenerated content, then replaces root `.opencode/` from `config/.opencode/`.
- The sync excludes runtime/install artifacts (for example `node_modules`) and only materializes intended config/authored outputs.
- Re-running the command on a clean tree is deterministic (no unexpected drift).
- Documentation explains destructive behavior, prerequisites, and recovery/verification steps.

## 3) Constraints and non-goals
- In scope: Nix flake command wiring, regeneration/sync orchestration script(s), and docs for operator usage.
- In scope: explicit destructive semantics for `config/` replacement and root `.opencode/` replacement.
- In scope: guardrails to avoid partial replacement (stage first, swap after successful generation).
- Out of scope: changing application runtime behavior beyond config-generation/sync flows.
- Out of scope: introducing non-Nix orchestration paths.
- Non-goal: preserving manual edits inside `config/` or root `.opencode/` after command execution.

## 4) Task stack (T01..T06)
- [x] T01: Define destructive sync contract and file ownership map (status:done)
  - Task ID: T01
  - Goal: Lock exact replacement semantics and owned path set for `config/` and root `.opencode/`.
  - Boundaries (in/out of scope):
    - In: authoritative source/target mapping, exclusion set (runtime artifacts), idempotence expectations.
    - Out: implementation code.
  - Done when:
    - Contract clearly states the command deletes and replaces `config/` and root `.opencode/` only after successful staged regeneration.
    - Owned paths and exclusions are explicit and reviewable.
  - Verification notes (commands or checks):
    - Review contract against current generated-owned paths and runtime exclusions in `config/pkl/README.md`.

- [x] T02: Add flake app entrypoint for sync workflow (status:done)
  - Task ID: T02
  - Goal: Expose a first-class flake runnable command (`nix run .#sync-opencode-config`).
  - Boundaries (in/out of scope):
    - In: `flake.nix` app wiring and dependency/runtime requirements.
    - Out: broader flake refactors unrelated to this workflow.
  - Done when:
    - `flake.nix` exports the app on supported systems and invokes the sync script deterministically.
  - Verification notes (commands or checks):
    - `nix run .#sync-opencode-config -- --help` (or equivalent dry-run/usage check if implemented).

- [x] T03: Implement staged regenerate-then-replace for `config/` (status:done)
  - Task ID: T03
  - Goal: Build regenerated `config/` into a staging location, then atomically replace live `config/` by deleting and copying staged output.
  - Boundaries (in/out of scope):
    - In: staging directory creation, regeneration invocation, destructive swap sequence, failure handling.
    - Out: changes to generated content definitions.
  - Done when:
    - Workflow never deletes live `config/` before successful staged regeneration.
    - On success, previous `config/` is removed and replaced entirely by regenerated tree.
  - Verification notes (commands or checks):
    - Run sync command and validate `config/` tree matches staged regeneration output exactly.

- [x] T04: Implement copy from `config/.opencode/` to root `.opencode/` (status:done)
  - Task ID: T04
  - Goal: Replace root `.opencode/` from regenerated `config/.opencode/` with declared exclusions.
  - Boundaries (in/out of scope):
    - In: delete-and-copy semantics for root `.opencode/`, exclusions (for example `node_modules`).
    - Out: syncing any other root dot-directories.
  - Done when:
    - Root `.opencode/` is fully replaced from regenerated source scope and excluded artifacts are not copied.
  - Verification notes (commands or checks):
    - Compare source and target trees after sync (excluding configured runtime artifacts).

- [x] T05: Document operator workflow and destructive safeguards (status:done)
  - Task ID: T05
  - Goal: Update docs with command usage, destructive warnings, exclusions, and verification/recovery steps.
  - Boundaries (in/out of scope):
    - In: concise operational documentation and troubleshooting.
    - Out: broad documentation reorganization.
  - Done when:
    - A contributor can run the flake command and understand exact replacement side effects.
  - Verification notes (commands or checks):
    - Follow docs verbatim on a clean tree and confirm expected outcomes.
  - Evidence:
    - Updated `config/pkl/README.md` with a dedicated destructive-sync operator section for `nix run .#sync-opencode-config`, including explicit side effects, replacement order, exclusions, deterministic rerun checks, and recovery guidance.
    - Ran `nix run .#sync-opencode-config -- --help` (exit 0) to verify documented command availability and usage text.
    - Ran `nix flake check` (exit 0); app and dev shell outputs evaluate successfully on the current host (with expected incompatible-system warnings for non-host platforms).

- [x] T06: Validation and cleanup (status:done)
  - Task ID: T06
  - Goal: Run full end-to-end checks, verify deterministic rerun behavior, and clean temp artifacts.
  - Boundaries (in/out of scope):
    - In: execution evidence for all success criteria and temporary artifact cleanup.
    - Out: new feature scope beyond this sync command.
  - Done when:
    - `nix run .#sync-opencode-config` succeeds.
    - Immediate second run is clean/deterministic.
    - Relevant parity checks (for example generated-output checks) pass after sync.
    - Temporary artifacts are removed.
  - Verification notes (commands or checks):
    - Run `nix run .#sync-opencode-config` twice.
    - Run `nix develop -c ./config/pkl/check-generated.sh`.
    - Inspect `git status --short` for expected-only changes.
  - Evidence:
    - Ran `nix run .#sync-opencode-config` twice consecutively (both exit 0); destructive staged regenerate-then-replace flow completed on both runs.
    - Ran `nix run .#sync-opencode-config -- --help` (exit 0); usage text confirms scope, replacement order intent, and runtime artifact exclusion note.
    - Ran `nix develop -c ./config/pkl/check-generated.sh` (exit 0); generated outputs parity check reported up to date.
    - Ran `nix flake check` (exit 0) as a light build/check for the flake app and dev shell outputs on host platform.
    - Verified temporary sync artifacts were cleaned (`/tmp/sync-opencode-config.*` absent).
    - Verified `git status --short` retained expected working-tree changes only (`.opencode/agent/Shared Context Drift.md`, `config/pkl/README.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/plans/config-opencode-sync-via-flake.md`, `flake.nix`).

### T06 validation report
- Commands run:
  - `nix run .#sync-opencode-config`
  - `nix run .#sync-opencode-config`
  - `nix run .#sync-opencode-config -- --help`
  - `nix develop -c ./config/pkl/check-generated.sh`
  - `nix flake check`
  - `git status --short`
- Exit codes and key outputs:
  - All commands exited `0`.
  - Sync runs reported both replacement completion messages for `config/` and root `.opencode/`.
  - Generated parity check reported: `Generated outputs are up to date.`
  - Flake check evaluated app and dev shell outputs successfully for host platform, with expected incompatible-system warnings for non-host platforms.
- Failed checks and follow-ups:
  - None.
- Success-criteria verification summary:
  - Flake app entrypoint exists and executes successfully.
  - Command replaces `config/` and root `.opencode/` in correct staged order.
  - Runtime artifact exclusions remain in place (`node_modules/`).
  - Immediate rerun is deterministic (no additional unexpected drift introduced).
  - Documentation and operator help are present for destructive semantics and usage.
- Residual risks:
  - Command remains intentionally destructive for `config/` and root `.opencode/`; operators must preserve wanted local edits before running.

## 5) Open questions
- None.

## 6) T01 destructive sync contract (approved baseline)

### 6.1 Command contract
- `nix run .#sync-opencode-config` is destructive by design for two targets only: repository `config/` and repository-root `.opencode/`.
- Replacement order is strict: (1) regenerate into a staging location, (2) validate staged output is complete, (3) delete and replace live `config/`, then (4) delete and replace root `.opencode/` from staged `config/.opencode/`.
- Live `config/` and root `.opencode/` must never be deleted before staged regeneration succeeds.
- Manual edits under destructive targets are non-goals and are not preserved after execution.

### 6.2 Ownership map
- Authoritative generation source: `config/pkl/generate.pkl`.
- Generated-owned paths under `config/` (per `config/pkl/README.md`):
  - `config/.opencode/agent/*.md`
  - `config/.opencode/command/*.md`
  - `config/.opencode/skills/*/SKILL.md`
  - `config/.opencode/lib/drift-collectors.js`
  - `config/.claude/agents/*.md`
  - `config/.claude/commands/*.md`
  - `config/.claude/skills/*/SKILL.md`
  - `config/.claude/lib/drift-collectors.js`
- Root sync source/target mapping:
  - source: regenerated `config/.opencode/`
  - target: repository-root `.opencode/`
  - semantics: full target replacement from source scope (not merge)

### 6.3 Exclusions and non-owned artifacts
- Runtime/install artifacts are excluded from sync materialization (for example `node_modules/`).
- Lockfiles, install outputs, and runtime-managed files outside generated-owned paths are not declared as generated-owned by this contract.
- Any exclusion list used by implementation must be explicit, reviewable, and applied consistently during copy/compare checks.

### 6.4 Idempotence and safety expectations
- Re-running on an unchanged tree is deterministic: no unexpected drift after an immediate second run.
- Failure prior to swap leaves live `config/` and root `.opencode/` unchanged.
- Post-run verification compares source/target trees with exclusions applied and confirms expected-only git changes.

### 6.5 Task status note
- T01 completed via contract definition only; no implementation code was changed in this task.
