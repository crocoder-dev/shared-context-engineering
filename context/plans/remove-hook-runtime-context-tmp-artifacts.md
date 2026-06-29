# Plan: Remove Hook Runtime Artifacts from `context/tmp`

## Change summary

Stop Agent Trace / hook runtime paths from writing debug/fallback artifacts into `context/tmp/` while preserving `context/tmp/` as SCE scratch/session space.

The current code truth shows `sce hooks diff-trace` still writes parsed payload artifacts to `context/tmp/<timestamp>-000000-diff-trace.json` before attempting AgentTraceDb persistence. Current context also documents historical or current hook artifacts such as `diff-trace` JSON files, post-commit JSON files, and Claude hook capture artifacts under `context/tmp/`. This plan removes those hook-runtime artifact writes and stale current-state references, then cleans existing hook-runtime artifacts from `context/tmp/` without touching SCE log files.

## Success criteria

- `sce hooks diff-trace` no longer writes `context/tmp/*-diff-trace.json` artifacts.
- Hook runtime success/error text no longer advertises `context/tmp` artifact fallback persistence.
- Agent Trace persistence remains DB-backed through per-checkout AgentTraceDb paths; DB write failure behavior remains deterministic and test-covered.
- No active post-commit, post-rewrite, Claude, or Agent Trace hook path writes JSON/log artifacts under `context/tmp/`.
- Existing hook-runtime artifacts are removed from `context/tmp/` using narrow patterns only.
- `context/tmp/` remains available for SCE scratch/session files and generated agent guidance/bootstrap remains intact.
- SCE log files are not deleted or modified.
- Durable context no longer describes hook-runtime `context/tmp` artifact writes as current behavior.
- Repository validation passes.

## Constraints and non-goals

- In scope: Rust hook runtime, hook-runtime tests, current-state context docs, and cleanup of existing hook-runtime artifacts under `context/tmp/`.
- Out of scope: removing `context/tmp/` itself, changing SCE scratch/session guidance, changing bootstrap creation of `context/tmp/`, changing generated agent guidance that says session-only scraps can use `context/tmp/`, and changing `sce` observability/logging behavior.
- Do not touch `sce.log` or other SCE log files.
- Do not backfill old `context/tmp` artifacts into AgentTraceDb.
- Do not introduce a new fallback artifact location for hook payloads.
- Do not change Agent Trace schema, migrations, or public payload shape unless required by removed artifact success text.

## Task stack

- [x] T01: `Remove diff-trace context/tmp artifact persistence` (status:done)
  - Task ID: T01
  - Goal: Make `sce hooks diff-trace` persist parsed payloads only to AgentTraceDb, removing the `context/tmp` artifact write path and related success/fallback wording.
  - Boundaries (in/out of scope): In — `cli/src/services/hooks/mod.rs` diff-trace persistence flow, helper removal or narrowing for hook artifact file writing, success text updates, and focused tests for DB success/failure behavior. Out — conversation-trace behavior, session-model behavior, DB schema changes, generated config changes, and cleanup of existing files.
  - Done when: Valid diff-trace input no longer creates a `context/tmp/*-diff-trace.json` file; success output mentions AgentTraceDb persistence only; DB insert failures retain deterministic logging/error or status behavior per the chosen implementation; no unused hook-artifact helpers remain unless still used by non-hook scratch paths.
  - Verification notes (commands or checks): Run a focused Rust test for hooks behavior if available, otherwise `nix develop -c sh -c 'cd cli && cargo test hooks'`; inspect test assertions for absence of `context/tmp` artifact expectations.
  - Completed: 2026-06-29
  - Files changed: `cli/src/services/hooks/mod.rs`, `context/plans/remove-hook-runtime-context-tmp-artifacts.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/cli/cli-command-surface.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix flake check` (all checks passed); `nix run .#pkl-check-generated` (generated outputs are up to date). Re-run after review feedback removing the temporary injected helper: same command chain passed. A direct focused Cargo test command was attempted first and blocked by repository bash policy in favor of `nix flake check`.
  - Notes: Removed the `diff-trace` `context/tmp` artifact write path and updated diff-trace status text to mention AgentTraceDb only; avoided adding extra injected runtime helpers after review feedback; synced durable current-state context for AgentTraceDb-only diff-trace persistence.

- [x] T02: `Audit and remove remaining hook artifact writers` (status:done)
  - Task ID: T02
  - Goal: Confirm no active post-commit, post-rewrite, Claude, or Agent Trace hook runtime writes JSON/log artifacts into `context/tmp/`.
  - Boundaries (in/out of scope): In — code/config search for `context/tmp`, `post-commit.json`, `post-rewrite`, Claude hook capture artifact paths, and hook artifact writer helpers; removal of active hook-runtime artifact writes if found. Out — generic SCE scratch/session guidance, `context/tmp` bootstrap, Pkl preview output path under `context/tmp/pkl-generated`, and SCE log files.
  - Done when: Search results show no active hook-runtime artifact writes to `context/tmp/`; any remaining `context/tmp` references are either generic scratch/session guidance, non-runtime preview output, historical plan evidence, or explicitly non-hook behavior.
  - Verification notes (commands or checks): Search code and generated config for `context/tmp`, `diff-trace.json`, `post-commit.json`, `post-rewrite`, and Claude capture artifact references; run targeted tests for any touched runtime paths.
  - Completed: 2026-06-29
  - Files changed: `cli/src/services/hooks/mod.rs`, `context/plans/remove-hook-runtime-context-tmp-artifacts.md`
  - Evidence: Targeted searches found and removed the active post-commit hook trace writer that persisted `context/tmp/*-post-commit.json`; after removal, `cli/**/*.rs` search for `persist_serialized_trace_payload`, `persist_hook_trace`, `diff-trace.json`, and `post-commit.json` found no matches, with only the generic `RepoPaths::context_tmp_dir()` accessor remaining. Generated config search for `context/tmp`, `diff-trace.json`, `post-commit.json`, `post-rewrite`, and Claude capture terms found only generic scratch/session guidance, automated session logging guidance, Pkl preview output under `context/tmp/pkl-generated`, or non-artifact hook command references. `nix develop -c sh -c 'cd cli && cargo fmt'` passed. First `nix flake check` exposed a clippy `needless_pass_by_value` issue introduced while removing trace persistence; after fixing it, rerun `nix flake check` passed. `nix run .#pkl-check-generated` passed with generated outputs up to date.
  - Notes: Removed the remaining active hook-runtime artifact writer and its now-unused collision-safe JSON artifact helper stack. Existing files under `context/tmp/` were intentionally left for T03.

- [x] T03: `Remove existing hook runtime artifacts from context/tmp` (status:done)
  - Task ID: T03
  - Goal: Delete existing Agent Trace / hook runtime artifacts from `context/tmp/` without deleting SCE logs or generic scratch/session files.
  - Boundaries (in/out of scope): In — narrow cleanup of timestamped `*-diff-trace.json`, timestamped `*-post-commit.json`, `post-rewrite/` hook artifacts, and Claude hook JSON artifact directories/files if present. Out — `context/tmp/.gitignore`, `.env`, `sce.log`, `*.log`, unrelated scratch files, and generated agent bootstrap/guidance.
  - Done when: `context/tmp/` no longer contains hook-runtime JSON artifact files/directories; SCE log files remain untouched; cleanup is reflected only as deletion of intended ignored/tracked artifacts.
  - Verification notes (commands or checks): Before deletion, inspect candidate paths and exclude logs; after deletion, list or glob `context/tmp/` for `*-diff-trace.json`, `*-post-commit.json`, `post-rewrite/`, and Claude hook JSON artifacts; verify `sce.log` still exists if it existed before.
  - Completed: 2026-06-29
  - Files changed: `context/plans/remove-hook-runtime-context-tmp-artifacts.md`; user manually removed ignored hook-runtime artifacts under `context/tmp/`
  - Evidence: Before cleanup, review found many `context/tmp/*-diff-trace.json` files, many `context/tmp/*-post-commit.json` files, `context/tmp/post-rewrite/`, and `context/tmp/claude/`, with `context/tmp/sce.log` present and explicitly preserved. After user-performed cleanup, targeted globs for `context/tmp/**/*-diff-trace.json`, `context/tmp/**/*-post-commit.json`, and `context/tmp/**/post-rewrite/**` returned no files; `context/tmp/` contained only `.env`, `.gitignore`, `claude-crof.sh`, and `sce.log`; `context/tmp/sce.log` remained present. `git status --short` was clean before the plan evidence update because cleaned artifacts were ignored/untracked.
  - Notes: Per user direction, this session did not delete artifacts directly and did not touch `sce.log` or any logic related to its creation.

- [ ] T04: `Sync durable context for DB-only hook persistence` (status:todo)
  - Task ID: T04
  - Goal: Update current-state context so future sessions understand hook-runtime persistence is DB-backed and no longer writes Agent Trace artifacts into `context/tmp/`.
  - Boundaries (in/out of scope): In — `context/sce/agent-trace-hooks-command-routing.md`, `context/overview.md`, `context/context-map.md`, and any directly relevant CLI/domain context files that currently describe `context/tmp` hook artifacts as current behavior. Out — historical completed plan evidence, generic `context/tmp` scratch guidance, and removing `context/tmp/` from the context map as a working area.
  - Done when: Current-state context describes `diff-trace` as AgentTraceDb-only persistence; no current-state context claims post-commit/post-rewrite/Claude hook JSON artifacts are written under `context/tmp/`; historical references remain only where clearly historical.
  - Verification notes (commands or checks): Search durable context outside `context/plans/` for stale current-state phrases: `context/tmp/<timestamp>-000000-diff-trace.json`, `context/tmp/*-diff-trace.json`, `post-commit.json`, `post-rewrite`, and Claude hook capture artifact wording.

- [ ] T05: `Validate and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Run full validation, generated-output parity checks, and final cleanup review for the artifact-removal change.
  - Boundaries (in/out of scope): In — repository validation, generated-output parity, final stale-reference search, verification that no task-owned temporary scaffolding remains, and plan evidence update. Out — unrelated refactors, deleting SCE logs, deleting generic `context/tmp` scratch/session files, or changing behavior beyond the plan scope.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; stale hook-runtime artifact references are absent from current-state docs/code; `context/tmp/` contains no hook-runtime artifacts targeted by this plan; SCE logs remain untouched.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted search for `context/tmp` hook artifact references; final `context/tmp/` cleanup inspection preserving `sce.log`/`*.log`.

## Open questions

- None. Scope clarified: preserve `context/tmp/` for SCE scratch/session use and do not touch SCE logs.
