# Atomic Commit Command + Skill (`/commit`)

## Purpose

Define the canonical commit-proposal workflow used by generated OpenCode and Claude command surfaces.

## Canonical contract

- Command slug: `commit`
- Command behavior source: `config/pkl/base/shared-content.pkl` (`commands["commit"]`)
- Canonical skill slug: `sce-atomic-commit`
- Skill behavior source: `config/pkl/base/shared-content.pkl` (`skills["sce-atomic-commit"]`)

Naming decision:
- Canonical skill name is `sce-atomic-commit`.
- `atomic-commits` is treated as legacy wording and is not the canonical generated skill slug.

## Behavior requirements

- Empty command arguments are supported; the command infers intent from staged changes.
- Before any proposal, the command must prompt for explicit staging confirmation (`git add <files>` guidance).
- After staging confirmation, commit guidance must classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`).
- Context-file-focused commit reminders are allowed only for `context/`-only staged diffs; mixed staged diffs must not receive default context-file reminders.
- Command text stays thin and gate-focused; commit grammar and atomic split logic are skill-owned in `sce-atomic-commit`.
- Output is proposal-only: commit message(s) and split guidance.
- The workflow never creates commits automatically.

## Generated targets

- OpenCode command: `config/.opencode/command/commit.md`
- Claude command: `config/.claude/commands/commit.md`
- OpenCode skill: `config/.opencode/skills/sce-atomic-commit/SKILL.md`
- Claude skill: `config/.claude/skills/sce-atomic-commit/SKILL.md`
