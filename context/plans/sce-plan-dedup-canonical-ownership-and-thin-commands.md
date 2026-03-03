# Plan: sce-plan-dedup-canonical-ownership-and-thin-commands

## 1) Change summary
Deduplicate shared SCE instruction content by centralizing cross-role doctrine in canonical reusable blocks, keeping role-specific Plan vs Code contracts separate, and converting high-duplication commands (`/next-task`, `/change-to-plan`, `/commit`) into thin orchestration wrappers over skill-owned behavior.

## 2) Success criteria
- Shared baseline doctrine (core principles, `context/` authority, quality posture) is defined once canonically and reused in both Shared Context Plan and Shared Context Code agent bodies.
- Role-specific mission, hard boundaries, and phase procedures remain separate for Plan vs Code (no agent merge).
- `next-task`, `change-to-plan`, and `commit` command bodies are trimmed to orchestration/gating responsibilities and no longer duplicate detailed skill contracts.
- Detailed behavior ownership is explicit and enforced in `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, `sce-context-sync`, and `sce-atomic-commit`.
- A durable dedup ownership table exists in `context/sce/` and is linked from `context/context-map.md`.
- Generated outputs are deterministic and up to date after regeneration.

## 3) Constraints and non-goals
- In scope: canonical source updates in `config/pkl/base/shared-content.pkl`, generated artifact parity, and context ownership/discoverability docs under `context/`.
- In scope: reducing textual duplication while preserving current behavior contracts and confirmation gates.
- Out of scope: merging Shared Context Plan and Shared Context Code into one agent.
- Out of scope: changing user-facing command names, adding new commands, or changing git policy semantics.
- Non-goal: removing all repetition; short role-local phrasing may remain when it improves readability.
- Non-goal: broad rewrites of unrelated skills/commands.

## 4) Task stack (T01..T07)
- [x] T01: Baseline duplication and ownership map finalization (status:done)
  - Task ID: T01
  - Goal: Produce a file-level map of duplicated instruction blocks and assign one canonical owner per behavior domain before edits.
  - Boundaries (in/out of scope):
    - In: `config/pkl/base/shared-content.pkl`, generated command/agent/skill artifacts, and existing overlap/decision docs in `context/sce/` and `context/decisions/`.
    - Out: modifying behavior text before ownership is finalized.
  - Done when:
    - Every major duplicated behavior has one owner and one or more reference-only consumers.
    - Intentional duplication (keep) vs removable duplication (dedup/remove) is explicitly labeled.
  - Verification notes (commands or checks):
    - Manual review of canonical and generated content for duplicate blocks.
    - Record output in `context/sce/dedup-ownership-table.md` (new or updated).
  - Verification evidence:
    - Canonical owner assignments captured in `context/sce/dedup-ownership-table.md` with explicit keep-vs-dedup labels.
    - Source/consumer traces include `config/pkl/base/shared-content.pkl`, generated OpenCode/Claude agent-command surfaces, and relevant SCE context decision/overlap docs.

- [x] T02: Extract shared Plan/Code baseline snippets into canonical reusable blocks (status:done)
  - Task ID: T02
  - Goal: Remove repeated Plan/Code baseline prose by defining shared canonical snippet blocks in Pkl and composing them into both agent bodies.
  - Boundaries (in/out of scope):
    - In: shared doctrine text only (core principles, `context/` authority, quality posture).
    - Out: role-specific mission, hard boundaries, startup, and procedure sequences.
  - Done when:
    - Shared blocks are defined once and consumed by both agent entries.
    - Agent role-local sections remain distinct and behaviorally unchanged.
  - Verification notes (commands or checks):
    - Inspect resulting agent canonical bodies for reduced duplication and unchanged role boundaries.
  - Verification evidence:
    - Added shared baseline snippet constants in `config/pkl/base/shared-content.pkl` (`sharedSceCorePrinciplesSection`, `sharedSceContextAuthoritySection`, `sharedSceQualityPosturePrefixBullets`, `sharedSceLongTermQualityBullet`) and composed them into both Shared Context Plan/Code canonical bodies.
    - Regenerated outputs and confirmed parity with `nix develop -c pkl eval -m . config/pkl/generate.pkl` and `nix run .#pkl-check-generated`.
    - Ran repository baseline checks with `nix flake check`.

- [x] T03: Thin `/next-task` to orchestration with skill-owned detail contracts (status:done)
  - Task ID: T03
  - Goal: Reduce command duplication by keeping `/next-task` focused on sequencing/gates while delegating detailed requirements to `sce-plan-review`, `sce-task-execution`, and `sce-context-sync`.
  - Boundaries (in/out of scope):
    - In: command-body simplification that preserves existing confirmation and implementation-stop semantics.
    - Out: changing the underlying behavior contracts inside the three phase skills unless required for clarity alignment.
  - Done when:
    - `/next-task` no longer restates full phase contracts.
    - Command still preserves required confirmation flow and final validation trigger behavior.
  - Verification notes (commands or checks):
    - Manual command-to-skill contract trace confirms no missing gate.
  - Verification evidence:
    - Simplified canonical `/next-task` command body in `config/pkl/base/shared-content.pkl` to orchestration-level gates and explicit delegation to `sce-plan-review`, `sce-task-execution`, and `sce-context-sync` without restating full skill contracts.
    - Regenerated command artifacts with `nix develop -c pkl eval -m . config/pkl/generate.pkl`, updating `config/.opencode/command/next-task.md` and `config/.claude/commands/next-task.md`.
    - Verified generated parity with `nix run .#pkl-check-generated` and ran repository baseline checks with `nix flake check`.

- [x] T04: Thin `/change-to-plan` to wrapper semantics over `sce-plan-authoring` (status:done)
  - Task ID: T04
  - Goal: Remove duplicated planning-gate detail from `/change-to-plan`, retaining only wrapper-level invocation and handoff obligations.
  - Boundaries (in/out of scope):
    - In: command simplification and explicit references to skill-owned clarification gate.
    - Out: weakening clarification strictness or plan output contract.
  - Done when:
    - `/change-to-plan` no longer duplicates skill-level ambiguity checks.
    - Plan creation confirmation and `/next-task` handoff remain explicit.
  - Verification notes (commands or checks):
    - Manual comparison of command text vs `sce-plan-authoring` for single-source ownership.
  - Verification evidence:
    - Simplified canonical `/change-to-plan` command body in `config/pkl/base/shared-content.pkl` to wrapper-level obligations and explicit delegation of clarification/plan-shape ownership to `sce-plan-authoring`.
    - Regenerated command artifacts with `nix develop -c pkl eval -m . config/pkl/generate.pkl`, updating `config/.opencode/command/change-to-plan.md` and `config/.claude/commands/change-to-plan.md`.
    - Verified generated parity with `nix run .#pkl-check-generated` and repository checks with `nix flake check`.

- [x] T05: Thin `/commit` to staging gate plus `sce-atomic-commit` delegation (status:done)
  - Task ID: T05
  - Goal: Keep `/commit` focused on staged-changes confirmation and output constraints while moving message-format/splitting detail ownership fully to `sce-atomic-commit`.
  - Boundaries (in/out of scope):
    - In: command simplification and explicit skill delegation.
    - Out: auto-committing behavior or changing proposal-only policy.
  - Done when:
    - `/commit` retains mandatory staged-confirmation gate and no-auto-commit policy.
    - Commit grammar and atomic splitting rules are owned in one place (`sce-atomic-commit`).
  - Verification notes (commands or checks):
    - Manual trace from command to skill confirms no duplicated style-contract prose remains in command.
  - Verification evidence:
    - Simplified canonical `/commit` command body in `config/pkl/base/shared-content.pkl` to retain staged-confirmation and proposal-only constraints while delegating commit grammar/splitting guidance to `sce-atomic-commit`.
    - Regenerated command artifacts with `nix develop -c pkl eval -m . config/pkl/generate.pkl`, updating `config/.opencode/command/commit.md` and `config/.claude/commands/commit.md`.
    - Verified generated parity with `nix run .#pkl-check-generated` and ran repository baseline checks with `nix flake check`.

- [x] T06: Context synchronization for dedup ownership and discoverability (status:done)
  - Task ID: T06
  - Goal: Persist final ownership boundaries and dedup policy in context so future edits have one obvious home.
  - Boundaries (in/out of scope):
    - In: `context/sce/dedup-ownership-table.md` and `context/context-map.md` updates; focused cross-links from relevant SCE workflow docs if needed.
    - Out: prose-heavy historical retrospectives in root context files unless cross-cutting policy changed.
  - Done when:
    - Ownership table is present, current-state oriented, and references canonical owner files.
    - `context/context-map.md` links to the ownership table.
  - Verification notes (commands or checks):
    - Manual read-through ensures ownership table matches final canonical sources.
  - Verification evidence:
    - Updated `context/sce/dedup-ownership-table.md` to current-state ownership semantics, replacing forward-looking T01/T02 wording with canonical owner references for shared snippets, thin command wrappers, and skill-owned detailed contracts.
    - Confirmed discoverability link remains explicit from `context/context-map.md` to `context/sce/dedup-ownership-table.md` with wording aligned to current thin-command ownership boundaries.
    - Performed required context sync review across `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/patterns.md`; no additional edits were required for T06 because current cross-cutting terminology and architecture contracts already match the implemented dedup state.

- [x] T07: Validation and cleanup (status:done)
  - Task ID: T07
  - Goal: Regenerate outputs, verify deterministic parity, and confirm no unresolved context drift from dedup changes.
  - Boundaries (in/out of scope):
    - In: generation, generated drift checks, and final context sync verification.
    - Out: unrelated feature work.
  - Done when:
    - Generated outputs are up to date.
    - Validation checks pass and success criteria have evidence.
    - Plan task statuses and verification evidence are updated.
  - Verification notes (commands or checks):
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - `nix run .#pkl-check-generated`
    - `nix flake check`
  - Verification evidence:
    - Regenerated canonical outputs with `nix develop -c pkl eval -m . config/pkl/generate.pkl`; generated targets were rewritten deterministically across OpenCode/Claude agent/command/skill trees and shared drift library outputs.
    - Confirmed generated parity with `nix run .#pkl-check-generated` (`Generated outputs are up to date.`).
    - Ran repository baseline validation with `nix flake check`; checks evaluated successfully, including `checks.x86_64-linux.cli-setup-command-surface`.
    - Completed mandatory context sync pass across `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md`; current-state dedup/thin-command ownership documentation already matched code truth, including durable feature discoverability links under `context/sce/` from `context/context-map.md`.

## 5) Open questions
- Whether command bodies should keep a short inline reminder of key gates (for readability) or rely exclusively on skill references after delegation.

## 6) Validation report (T07)

- Commands run:
  - `nix develop -c pkl eval -m . config/pkl/generate.pkl` (exit 0)
  - `nix run .#pkl-check-generated` (exit 0)
  - `nix flake check` (exit 0)
- Key outputs:
  - Generation completed and rewrote deterministic generated targets under `config/.opencode/**` and `config/.claude/**`.
  - Drift check reported: `Generated outputs are up to date.`
  - Flake checks evaluated successfully, including `checks.x86_64-linux.cli-setup-command-surface`.
- Failed checks and follow-ups:
  - None.
- Success-criteria verification summary:
  - Shared baseline doctrine is canonically defined once and reused across Plan/Code agent bodies.
  - Plan-vs-Code role separation remains intact (no merge), with decision/context evidence preserved.
  - `/next-task`, `/change-to-plan`, and `/commit` remain thin orchestration wrappers with detailed behavior delegated to canonical skills.
  - Skill ownership boundaries remain explicit in `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, `sce-context-sync`, and `sce-atomic-commit`.
  - Durable dedup ownership table remains present and discoverable from `context/context-map.md`.
  - Generated outputs/parity checks are passing on current code.
- Residual risks:
  - Open wording preference remains for how much gate-reminder text command wrappers should keep vs skill-only references (tracked under Open questions).
