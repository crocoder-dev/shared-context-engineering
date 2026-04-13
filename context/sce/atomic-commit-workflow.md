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
- Manual `sce-atomic-commit` analyzes staged changes for coherent units and proposes one or more commit messages when staged changes mix unrelated goals; it stays proposal-only and does not create commits automatically.
- Automated `sce-atomic-commit` produces exactly one commit message for the staged diff and does not branch into multi-commit or split guidance.
- When staged changes include `context/plans/*.md`, each proposed commit body must cite the affected plan slug(s) and updated task ID(s) (`T0X`); if the staged plan diff is ambiguous, the workflow must stop for clarification rather than inventing references.
- Output is proposal-only in the manual profile: commit message proposals with optional split guidance, not automatic commits.
- Output is execute-once in the automated OpenCode profile: generate exactly one commit message, then run `git commit` against the staged diff.

## Generated targets

- OpenCode command: `config/.opencode/command/commit.md`
- Automated OpenCode command: `config/automated/.opencode/command/commit.md`
- Claude command: `config/.claude/commands/commit.md`
- OpenCode skill: `config/.opencode/skills/sce-atomic-commit/SKILL.md`
- Automated OpenCode skill: `config/automated/.opencode/skills/sce-atomic-commit/SKILL.md`
- Claude skill: `config/.claude/skills/sce-atomic-commit/SKILL.md`
