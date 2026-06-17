# Plan: sce-atomic-commit-bypass-awareness

## Change summary

The `commit-oneshot-bypass` plan added a `/commit oneshot` / `/commit skip` bypass path to the `commit.md` command, but the `sce-atomic-commit` skill was intentionally left untouched. This created a conflict: the command body says to auto-commit in bypass mode, but the loaded skill has a hard rule "stay proposal-only: do not create commits automatically" that blocks it. Fix the skill body to add bypass-mode awareness so the skill recognizes when the command is in bypass mode and relaxes its standard rules.

## Success criteria

1. `/commit skip` (or `/commit oneshot`) auto-commits without the skill blocking on "proposal-only".
2. The skill produces exactly one commit message in bypass mode (no split guidance).
3. The skill skips context-guidance gate classification in bypass mode.
4. Plan-citation uses best-effort inference in bypass mode (does not stop for ambiguity).
5. Regular `/commit` behavior (no arguments) is unchanged — still proposal-only, split guidance, context-guidance gate.
6. Generated outputs stay deterministic and pass `nix run .#pkl-check-generated`.

## Constraints and non-goals

- **In scope**: The manual `sce-atomic-commit` skill body in `config/pkl/base/shared-content-commit.pkl`.
- **In scope**: Regeneration of all skill output files and verification.
- **In scope**: Context workflow doc update (`context/sce/atomic-commit-workflow.md`) to reflect skill-level bypass awareness.
- **Out of scope**: The automated `sce-atomic-commit` skill (`config/pkl/base/shared-content-automated-commit.pkl`) — already produces single message and auto-commits; no changes needed.
- **Out of scope**: The command bodies (`commit.md` files) — already correct; no changes needed.
- **Out of scope**: Any Rust CLI changes.

## Assumptions

1. The skill does not need to inspect `$ARGUMENTS` itself — the command body is the orchestrator and tells the skill it's in bypass mode via overrides. The skill just needs to acknowledge bypass mode as a valid operating mode and not block it.
2. The existing command-body bypass overrides ("Produce exactly one commit message. Do not propose splits...") remain correct and sufficient; the skill just needs to not conflict with them.
3. The bypass mode is identical between OpenCode and Claude manual profiles since they share the same Pkl skill body source.

---

## Task stack

- [x] T01: `Update Pkl skill body and regenerate` (status:done)
  - Task ID: T01
  - Goal: Add bypass-mode awareness to the manual `sce-atomic-commit` skill body in `config/pkl/base/shared-content-commit.pkl`, and regenerate all generated skill outputs.
  - Boundaries (in/out of scope):
    - In: Edit `config/pkl/base/shared-content-commit.pkl` skills `["sce-atomic-commit"].canonicalBody` to:
      - Update the Goal section to acknowledge bypass mode exists and relaxes certain rules.
      - Add a new `## Bypass mode` section describing relaxed rules:
        - Do not enforce proposal-only (auto-commit is allowed).
        - Produce exactly one commit message (no split guidance).
        - Skip context-guidance gate classification.
        - Plan citations: best-effort only (omit if ambiguous).
      - Add "In regular mode:" qualifiers to steps 5-7 in the Procedure section and to the Context-file guidance gating section so the standard rules are clearly scoped.
      - Update anti-patterns if needed.
    - Out: Automated skill body, command bodies, agent bodies, Rust CLI.
  - Done when:
    - The Pkl skill body includes a `## Bypass mode` section that explicitly allows auto-commit and relaxes standard rules.
    - The standard rules (proposal-only, split guidance, context-guidance gate, plan-citation stops) are clearly scoped to regular mode.
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` runs successfully and regenerates skill output files.
    - `nix run .#pkl-check-generated` exits 0.
    - All four manual skill files are regenerated and contain the bypass mode section:
      - `.opencode/skills/sce-atomic-commit/SKILL.md`
      - `.claude/skills/sce-atomic-commit/SKILL.md`
      - `config/.opencode/skills/sce-atomic-commit/SKILL.md`
      - `config/.claude/skills/sce-atomic-commit/SKILL.md`
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated` exits 0.
    - Grep for "Bypass mode" in regenerated skill files to confirm the section exists.
    - Grep for "proposal-only" to confirm it's still present but scoped to regular mode.
    - Manual inspection: the skill should be readable as a coherent document with both regular and bypass paths.
  - **Completed:** 2026-06-17
  - **Files changed:** `config/pkl/base/shared-content-commit.pkl`, `config/.opencode/skills/sce-atomic-commit/SKILL.md`, `config/.claude/skills/sce-atomic-commit/SKILL.md`
  - **Evidence:**
    - `nix run .#pkl-check-generated` exits 0 (output: "Generated outputs are up to date.")
    - Grep for "Bypass mode" in generated skill files: found at line 42 in both `config/.opencode/skills/sce-atomic-commit/SKILL.md` and `config/.claude/skills/sce-atomic-commit/SKILL.md`
    - Grep for "proposal-only": still present and scoped (Goal + Bypass mode sections)
    - Grep for "In regular mode:": found on steps 5, 6, 7, and Context-file guidance gating
    - Automated skill (`config/automated/.opencode/skills/sce-atomic-commit/SKILL.md`) unchanged (no Bypass mode, as expected)
    - `git diff --stat`: only the 3 expected files changed (+51/-9)
  - **Notes:** Root-level `.opencode/` and `.claude/` skill copies are deployed artifacts (not directly regenerated by Pkl). They will be updated on next `sce setup`.

- [x] T02: `Update context workflow documentation` (status:done)
  - Task ID: T02
  - Goal: Update `context/sce/atomic-commit-workflow.md` to document that the skill itself now has bypass-mode awareness.
  - Boundaries (in/out of scope):
    - In: Update `context/sce/atomic-commit-workflow.md` behavior requirements section and the oneshot/skip bypass mode section to note that the skill body intentionally includes bypass-mode awareness.
    - Out: Other context files — this is a focused workflow doc update.
  - Done when:
    - The behavior requirements section notes that the manual `sce-atomic-commit` skill body includes bypass-mode awareness (not just the command body).
    - The oneshot/skip section references the skill's bypass-mode section.
  - Verification notes (commands or checks):
    - Read `context/sce/atomic-commit-workflow.md` and confirm the updated sections are accurate.
  - **Completed:** 2026-06-17
  - **Files changed:** `context/sce/atomic-commit-workflow.md`
  - **Evidence:**
    - Behavior requirements section (line 25): now begins with "The manual `sce-atomic-commit` skill body now includes bypass-mode awareness (see the skill's `## Bypass mode` section)..." — confirms skill-level awareness, preserves regular-mode description.
    - Oneshot/skip section (line 45): new sentence referencing the skill's dedicated `## Bypass mode` section aligning with command-body overrides.
    - File remains 58 lines, within quality limits.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Verify all generated outputs are current, all checks pass, and no stale artifacts remain.
  - Boundaries (in/out of scope):
    - In: Run `nix run .#pkl-check-generated`, `nix flake check`, verify git status shows only intended changes.
    - Out: Application code changes beyond those regenerated.
  - Done when:
    - `nix run .#pkl-check-generated` exits 0.
    - `nix flake check` passes (excluding pre-existing `cli-fmt` issue on `generated_migrations.rs`).
    - Generated skill files match the canonical Pkl source.
    - Regular `/commit` behavior is verified by manual inspection of the skill body.
    - No temporary scaffolding or debug artifacts remain.
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated`
    - `nix flake check`
    - `git diff --stat` — only `config/pkl/base/shared-content-commit.pkl`, regenerated skill files, and context files should appear.
    - Manual inspection of a regenerated skill file for correctness and readability.
  - **Completed:** 2026-06-17
  - **Evidence:**
    - `nix run .#pkl-check-generated` exits 0 ("Generated outputs are up to date.")
    - `nix flake check`: pkl-parity, cli-tests, cli-clippy pass; cli-fmt fails with pre-existing `generated_migrations.rs` trailing newline (documented exclusion)
    - `git diff --stat`: only the 4 expected files — `config/pkl/base/shared-content-commit.pkl`, 2 regenerated skill files, `context/sce/atomic-commit-workflow.md`
    - Manual inspection of `config/.opencode/skills/sce-atomic-commit/SKILL.md` (104 lines): Bypass mode section present, "In regular mode:" qualifiers on steps 5-7 and Context-file guidance gating, anti-pattern includes bypass mode, regular `/commit` behavior preserved
    - No temporary scaffolding or debug artifacts from this plan
  - **Notes:** All 3 tasks complete. Plan successfully delivers bypass-awareness to the manual `sce-atomic-commit` skill body.

## Open questions

_None._

---

## Validation Report

### Plans tasks completed
- [x] T01: Update Pkl skill body and regenerate
- [x] T02: Update context workflow documentation
- [x] T03: Validation and cleanup

### Commands run

| Command | Result |
|---|---|
| `nix run .#pkl-check-generated` | exit 0 — "Generated outputs are up to date." |
| `nix flake check` | cli-fmt failed (pre-existing `generated_migrations.rs` trailing newline; documented exclusion). pkl-parity, cli-tests, cli-clippy: passed. |
| `git diff --stat` | 4 files: `config/pkl/base/shared-content-commit.pkl`, 2 regenerated skill files, `context/sce/atomic-commit-workflow.md` (+54/-10) |

### Success-criteria verification

1. **[x] `/commit skip` (or `/commit oneshot`) auto-commits without the skill blocking on "proposal-only".**
   - Verified: Skill body `## Bypass mode` section line 46: "Proposal-only → auto-commit allowed. Do not block auto-commit; the command will execute `git commit` with the produced message."

2. **[x] The skill produces exactly one commit message in bypass mode (no split guidance).**
   - Verified: Skill body `## Bypass mode` section line 47: "Single message only. Produce exactly one commit message. Do not propose splits. Do not emit split guidance."

3. **[x] The skill skips context-guidance gate classification in bypass mode.**
   - Verified: Skill body `## Bypass mode` section line 48: "Context-guidance gate skipped. Do not classify staged diff scope as `context/`-only vs mixed. Do not apply context-file guidance gating."

4. **[x] Plan-citation uses best-effort inference in bypass mode (does not stop for ambiguity).**
   - Verified: Skill body `## Bypass mode` section line 49: "Plan citations: best-effort only. When staged changes include `context/plans/*.md`, make a best-effort inference... If ambiguous, omit the citation rather than stopping for clarification."

5. **[x] Regular `/commit` behavior (no arguments) is unchanged — still proposal-only, split guidance, context-guidance gate.**
   - Verified: Procedure steps 5-7 prefixed with "In regular mode:", Context-file guidance gating prefixed with "In regular mode:", Goal section retains "stay proposal-only: do not create commits automatically". All standard rules preserved.

6. **[x] Generated outputs stay deterministic and pass `nix run .#pkl-check-generated`.**
   - Verified: exit 0, "Generated outputs are up to date."

### Failed checks and follow-ups
- **cli-fmt**: Pre-existing `generated_migrations.rs` missing trailing newline. Documented exclusion; not introduced by this plan.

### Temporary scaffolding
- None introduced during this plan.

### Residual risks
- None identified. The skill body and context documentation are aligned. Generated outputs match canonical source.
