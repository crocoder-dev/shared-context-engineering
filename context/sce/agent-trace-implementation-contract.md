# Agent Trace Implementation Contract (Historical Reference)

## Current status

- This document is retained as historical reference only.
- The current CLI runtime does not implement the Agent Trace contract described by the original no-git-wrapper plan.
- Local hooks are currently attribution-only: `commit-msg` may append the canonical SCE co-author trailer when the attribution gate is enabled, while `pre-commit`, `post-commit`, and `post-rewrite` are deterministic no-ops.

## Historical scope

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T01`
- Scope at the time: implementation-contract baseline only (no production code changes)

## Historical objective

Define one canonical, implementation-ready contract for Agent Trace attribution so downstream tasks in that plan could execute against a single set of invariants.

## Current guidance

- Do not treat this file as current runtime truth.
- For current local-hook behavior, use `context/sce/agent-trace-hooks-command-routing.md`.
- For the current local DB baseline, use `context/sce/agent-trace-core-schema-migrations.md`.
