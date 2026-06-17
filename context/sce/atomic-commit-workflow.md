# Atomic Commit Command + Skill (`/commit`)

## Purpose

Define the canonical commit workflow used by generated manual and automated command surfaces.

## Canonical contract

- Command slug: `commit`
- Command behavior source: `config/pkl/base/shared-content-commit.pkl` (aggregated through `config/pkl/base/shared-content.pkl`)
- Canonical skill slug: `sce-atomic-commit`
- Skill behavior source: `config/pkl/base/shared-content-commit.pkl` (aggregated through `config/pkl/base/shared-content.pkl`)

Naming decision:
- Canonical skill name is `sce-atomic-commit`.
- `atomic-commits` is treated as legacy wording and is not the canonical generated skill slug.

## Behavior requirements

- Empty command arguments are supported; the command infers intent from staged changes.
- Before any proposal, the command must prompt for explicit staging confirmation (`git add <files>` guidance).
- After staging confirmation, commit guidance must classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`).
- Context-file-focused commit reminders are allowed only for `context/`-only staged diffs; mixed staged diffs must not receive default context-file reminders.
- Command text stays thin and gate-focused; commit grammar and split-aware proposal rules are skill-owned in `sce-atomic-commit`.
- The manual `sce-atomic-commit` skill body now includes bypass-mode awareness (see the skill's `## Bypass mode` section); when invoked in bypass mode, it relaxes proposal-only, split guidance, context-guidance gate, and plan-citation ambiguity rules. In regular mode, it analyzes staged changes for coherent units and proposes one or more commit messages when staged changes mix unrelated goals; it stays proposal-only and does not create commits automatically.
- Automated `sce-atomic-commit` produces exactly one commit message for the staged diff and does not branch into multi-commit or split guidance.
- When staged changes include `context/plans/*.md`, each proposed commit body must cite the affected plan slug(s) and updated task ID(s) (`T0X`); if the staged plan diff is ambiguous, the workflow must stop for clarification rather than inventing references.
- Output is proposal-only in the manual profile: commit message proposals with optional split guidance, not automatic commits.
- Output is execute-once in the automated OpenCode profile: generate exactly one commit message, then run `git commit` against the staged diff.

## Oneshot / skip bypass mode

The manual OpenCode `/commit` command supports an argument-based bypass mode triggered by `/commit oneshot` or `/commit skip` (case-insensitive, first token). This mode is a behavior branch within the existing `commit` command body — it does not add a separate command or alter the automated profile.

When invoked with `oneshot` or `skip`:

- **Staging confirmation skipped:** The command does not prompt for or wait on explicit staging confirmation.
- **Context-guidance gate skipped:** No `context/`-only vs mixed diff classification is applied, and no context-file reminders are emitted.
- **Split guidance skipped:** The command does not branch into multi-commit or split guidance, even when staged changes mix unrelated goals.
- **Plan-citation ambiguity stops skipped:** If staged plan file context is ambiguous, the command makes a best-effort inference or omits the plan citation rather than stopping for clarification.
- **Single message + auto-commit:** The command produces exactly one commit message via `sce-atomic-commit`, then immediately executes `git commit` with that message.
- **Empty-stage guard:** If no staged changes exist, the command stops with a clear error and does not attempt a commit.
- **Commit-failure guard:** If `git commit` fails, the command stops and reports the failure without inventing fallback commits.

The `sce-atomic-commit` skill body now includes a dedicated `## Bypass mode` section that aligns with these command-body overrides, ensuring the skill does not conflict with auto-commit, single-message, or best-effort citation behavior in bypass mode.

The regular `/commit` path (no arguments, or non-bypass arguments) is unchanged: it retains the staging confirmation prompt, context-guidance gate, optional split guidance, and plan-citation ambiguity stops per the behavior requirements above.

The bypass mode is scoped to the manual OpenCode profile only. The automated OpenCode `/commit` command already produces a single message and auto-executes `git commit` without these guardrails; the bypass argument is not relevant to it.

## Generated targets

- OpenCode command: `config/.opencode/command/commit.md`
- Automated OpenCode command: `config/automated/.opencode/command/commit.md`
- Claude command: `config/.claude/commands/commit.md`
- OpenCode skill: `config/.opencode/skills/sce-atomic-commit/SKILL.md`
- Automated OpenCode skill: `config/automated/.opencode/skills/sce-atomic-commit/SKILL.md`
- Claude skill: `config/.claude/skills/sce-atomic-commit/SKILL.md`
