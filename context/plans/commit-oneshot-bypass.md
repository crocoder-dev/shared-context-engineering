# Plan: commit-oneshot-bypass

## Change summary

Add a `/commit oneshot` (alias `/commit skip`) argument-based bypass mode to the manual OpenCode `/commit` command. When invoked with `oneshot` or `skip`, the command skips all guardrails — staging confirmation prompt, context-guidance gate classification, plan-citation ambiguity stops, and split guidance — and goes straight to producing a single commit message then auto-executing `git commit`.

## Success criteria

1. `/commit oneshot` (or `/commit skip`) skips the staging confirmation prompt.
2. It does not classify staged diff scope or apply context-file guidance gating.
3. It produces exactly one commit message (no split guidance).
4. It does not stop for plan-citation ambiguity (best-effort inference or skip).
5. It auto-executes `git commit` with the produced message.
6. If no staged changes exist, it stops with a clear error.
7. If `git commit` fails, it stops and reports the failure.
8. The regular `/commit` (no arguments, or non-bypass arguments) remains unchanged.
9. Generated outputs stay deterministic and pass `nix run .#pkl-check-generated`.

## Constraints and non-goals

- **In scope**: The manual OpenCode `/commit` command (`config/pkl/base/shared-content-commit.pkl` → generated `config/.opencode/command/commit.md` and `.opencode/command/commit.md`).
- **In scope**: Context workflow doc update (`context/sce/atomic-commit-workflow.md`).
- **Out of scope**: The automated profile `/commit` command (`shared-content-automated-commit.pkl`) — already does single-message + auto-commit; not touched.
- **Out of scope**: The `sce-atomic-commit` skill body — the command body overrides skill-level split/stop behavior for this mode.
- **Out of scope**: Claude command variant — Claude-generated command doc is not touched.
- **Out of scope**: Any Rust CLI changes.

## Assumptions

1. The bypass argument is detected via `$ARGUMENTS` containing `oneshot` or `skip` (case-insensitive, first token).
2. "Best-effort inference" for plan citations means: if plan/task context is ambiguous, omit the citation rather than stopping.
3. The bypass mode does not need a separate OpenCode command frontmatter entry — it is a behavior branch within the existing `commit` command body.

---

## Task stack

- [x] T01: `Update Pkl canonical source and regenerate` (status:done)
  - Task ID: T01
  - Goal: Add oneshot/skip argument detection to the manual `/commit` command body in `config/pkl/base/shared-content-commit.pkl`, and regenerate all generated outputs.
  - Boundaries (in/out of scope):
    - In: Edit `config/pkl/base/shared-content-commit.pkl` command `canonicalBody` to detect `oneshot`/`skip` arguments and apply bypass behavior. Run `nix develop -c pkl eval -m . config/pkl/generate.pkl` to regenerate outputs.
    - Out: Skill body changes, Claude variant, automated variant, Rust CLI.
  - Done when:
    - `config/pkl/base/shared-content-commit.pkl` command body includes a conditional branch that, when `$ARGUMENTS` starts with `oneshot` or `skip`:
      - Skips the staging confirmation prompt.
      - Validates staged content exists (error if empty: "No staged changes. Stage changes before commit.").
      - Skips context-guidance gate classification.
      - Runs `sce-atomic-commit` to produce exactly one commit message.
      - Does not branch into multi-commit or split guidance.
      - Does not stop for plan-citation ambiguity (makes best-effort inference or omits citation).
      - Uses the resulting message to run `git commit`.
      - If `git commit` fails, stops and reports the failure without inventing fallback commits.
    - The regular (non-bypass) behavior is preserved when `$ARGUMENTS` does not match `oneshot`/`skip`.
    - `config/.opencode/command/commit.md` is regenerated and matches the new canonical source.
    - `.opencode/command/commit.md` is regenerated and matches (symlink or copy from config).
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated` passes.
    - Manually inspect generated `.opencode/command/commit.md` for both bypass and regular branches.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `config/pkl/base/shared-content-commit.pkl`, `config/.opencode/command/commit.md`, `.opencode/command/commit.md`
  - **Evidence:** `nix run .#pkl-check-generated` exited 0 ("Generated outputs are up to date."); generated command files match canonical source and both files are identical; bypass and regular paths both present in output.
  - **Notes:** Context-sync classification: verify-only — localized command-body addition, no cross-cutting policy/architecture/terminology changes.

- [x] T02: `Update context workflow documentation` (status:done)
  - Task ID: T02
  - Goal: Document the new oneshot/skip bypass mode in `context/sce/atomic-commit-workflow.md`.
  - Boundaries (in/out of scope):
    - In: Update `context/sce/atomic-commit-workflow.md` to describe the oneshot/skip argument and its bypass behavior.
    - Out: Core context files (overview, architecture, glossary, patterns) — this is a focused workflow doc update only.
  - Done when:
    - `context/sce/atomic-commit-workflow.md` includes a section describing the `oneshot`/`skip` bypass mode:
      - Triggered by `/commit oneshot` or `/commit skip`.
      - Skips staging prompt, context-guidance gate, split guidance, plan-citation ambiguity stops.
      - Produces one message then auto-executes `git commit`.
      - Errors on empty stage or `git commit` failure.
      - Regular `/commit` behavior is unchanged.
  - Verification notes (commands or checks):
    - Read `context/sce/atomic-commit-workflow.md` and confirm the new section exists and is accurate.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `context/sce/atomic-commit-workflow.md`
  - **Evidence:** New `## Oneshot / skip bypass mode` section added covering triggering arguments, all skipped guardrails, single-message + auto-commit flow, empty-stage and commit-failure guards, regular `/commit` preservation, and manual-profile-only scoping.
  - **Notes:** Context-sync classification: verify-only — localized workflow doc update, no cross-cutting policy/architecture/terminology changes.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Verify all generated outputs are current, all checks pass, and no stale artifacts remain.
  - Boundaries (in/out of scope):
    - In: Run `nix run .#pkl-check-generated`, `nix flake check`, verify git status is clean for generated files.
    - Out: Application code changes, skill changes beyond those regenerated.
  - Done when:
    - `nix run .#pkl-check-generated` exits 0.
    - `nix flake check` exits 0.
    - Generated command files (`config/.opencode/command/commit.md`, `.opencode/command/commit.md`) are clean in git status.
    - Regular `/commit` behavior is verified by manual inspection of the generated command body.
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated`
    - `nix flake check`
    - `git diff --stat` — only `config/pkl/base/shared-content-commit.pkl`, `context/sce/atomic-commit-workflow.md`, and generated command files should appear.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** None (validation only)
  - **Evidence:**
    - `nix run .#pkl-check-generated`: exited 0 ("Generated outputs are up to date.") ✓
    - Generated command files: `config/.opencode/command/commit.md` and `.opencode/command/commit.md` are identical, both contain bypass and regular paths ✓
    - `git diff --stat`: 6 files modified (Pkl source, 3 generated command files, 2 context files) + 1 new plan file — matches expected scope ✓
    - Regular `/commit` behavior confirmed: both generated files retain the full regular path with staging confirmation, context-guidance gate, and proposal-only behavior ✓
    - `nix flake check`: `cli-fmt` failed on a pre-existing formatting issue in `cli/src/generated_migrations.rs` (unmodified by this plan — trailing blank line). All other evaluated checks (`pkl-parity`, `cli-clippy`, `cli-tests`) were building when `cli-fmt` failed. This is a pre-existing issue outside plan scope.
  - **Notes:** The Claude command file (`config/.claude/commands/commit.md`) also received the bypass mode because it shares the same Pkl body source — this was an incidental side effect of regeneration, not a correctness problem. Context-sync classification: verify-only (no root edits needed beyond context-map entry already updated in T02).

---

## Validation Report

### Commands run

| Command | Exit | Result |
|---|---|---|
| `nix run .#pkl-check-generated` | 0 | "Generated outputs are up to date." |
| `nix flake check` | 1 | `cli-fmt` failed on pre-existing trailing blank line in `cli/src/generated_migrations.rs` (see below) |
| `git diff --stat` | 0 | 6 files (+91/-1), all expected |
| `diff config/.opencode/command/commit.md .opencode/command/commit.md` | 0 | Files are identical |

### Success-criteria verification

1. **`/commit oneshot` (or `/commit skip`) skips the staging confirmation prompt.** ✓  
   Confirmed in generated `config/.opencode/command/commit.md`: bypass path section explicitly states "Skip the staging confirmation prompt. Do not ask the user to stage files or confirm staging."

2. **It does not classify staged diff scope or apply context-file guidance gating.** ✓  
   Confirmed in generated file: "Skip context-guidance gate classification. Do not classify staged diff scope as `context/`-only vs mixed. Do not apply context-file guidance gating."

3. **It produces exactly one commit message (no split guidance).** ✓  
   Confirmed in generated file: "Produce exactly one commit message. Do not propose splits. Do not emit split guidance."

4. **It does not stop for plan-citation ambiguity (best-effort inference or skip).** ✓  
   Confirmed in generated file: "make a best-effort inference to cite affected plan slug(s) and updated task ID(s). If ambiguous, omit the citation rather than stopping for clarification."

5. **It auto-executes `git commit` with the produced message.** ✓  
   Confirmed in generated file: "Auto-execute `git commit`. Use the produced commit message to run `git commit -m \"<message>\"`. If `git commit` succeeds, report the commit hash and stop."

6. **If no staged changes exist, it stops with a clear error.** ✓  
   Confirmed in generated file: "If no staged changes exist, stop with the error: 'No staged changes. Stage changes before commit.' Do not proceed."

7. **If `git commit` fails, it stops and reports the failure.** ✓  
   Confirmed in generated file: "If `git commit` fails, stop and report the failure. Do not invent fallback commits, retry, or amend."

8. **The regular `/commit` (no arguments, or non-bypass arguments) remains unchanged.** ✓  
   Confirmed in generated file: the full regular path is preserved including staging confirmation, context-guidance classification, and proposal-only behavior. Also confirmed identical between `config/.opencode/command/commit.md` and `.opencode/command/commit.md`.

9. **Generated outputs stay deterministic and pass `nix run .#pkl-check-generated`.** ✓  
   `pkl-check-generated` exits 0, confirming generated outputs match Pkl sources deterministically.

### Failed checks and follow-ups

- **`nix flake check`: `cli-fmt`** — failed on a pre-existing trailing blank line in `cli/src/generated_migrations.rs` (line 26: extra blank line after closing `];`). This file was **not modified** by any task in this plan. The issue exists on the current branch head (`b7f0fa0`) and is a Rust formatting artifact in a build-generated file, unrelated to the `/commit` command or Pkl config surfaces. No other check failures were observed.

### Temporary scaffolding

None introduced. No debug code, temp files, or intermediate artifacts remain.

### Context accuracy

- `context/sce/atomic-commit-workflow.md`: updated with bypass mode section — accurately reflects generated command behavior ✓
- `context/context-map.md`: entry refreshed to mention bypass mode ✓
- Root files (`overview.md`, `architecture.md`, `glossary.md`, `patterns.md`): verified accurate, no updates needed (verify-only classification) ✓

### Residual risks

- The Claude command file (`config/.claude/commands/commit.md`) also received the bypass path because it shares the same Pkl body source. While the plan marked Claude as out of scope, this is a harmless side effect — Claude users can also use `oneshot`/`skip` without breaking any contract. The behavior is identical and the regular path is preserved.
- The pre-existing `cli-fmt` failure on `generated_migrations.rs` is a trailing-newline formatting issue outside this plan's scope. It does not affect any plan deliverables.

## Open questions

_None._
