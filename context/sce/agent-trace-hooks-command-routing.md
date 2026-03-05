# Agent Trace Hooks Command Routing

## Scope
- Plan: `agent-trace-local-hooks-production-mvp`
- Tasks: `T02`, `T07`
- Focus: implemented `sce hooks` subcommand routing plus production post-rewrite runtime wiring.

## Implemented command surface
- `sce hooks pre-commit`
- `sce hooks commit-msg <message-file>`
- `sce hooks post-commit`
- `sce hooks post-rewrite <amend|rebase|other>` (reads rewrite pairs from STDIN)

## Parser and dispatch behavior
- `cli/src/app.rs` routes `hooks` through dedicated hook-subcommand parsing instead of generic no-arg subcommand parsing.
- `cli/src/services/hooks.rs` now owns hook CLI usage text, deterministic parse errors, and runtime dispatch through `HookSubcommand` + `run_hooks_subcommand`.
- Invalid and ambiguous invocations return deterministic actionable errors pointing to `sce hooks --help`.

## Current runtime entrypoint behavior
- `pre-commit`: executes the pre-commit runtime entrypoint and reports staged-checkpoint finalization outcome.
- `commit-msg`: validates `<message-file>`, resolves runtime gates (`SCE_DISABLED`, `SCE_COAUTHOR_ENABLED`, staged checkpoint presence), applies canonical co-author policy, and writes back only when trailer mutation is required.
- `post-commit`: resolves runtime guards, builds commit attribution input from git + pre-commit checkpoint artifacts, executes `finalize_post_commit_trace`, writes canonical note payloads to `refs/notes/agent-trace`, ensures persistent local DB readiness (`.../sce/agent-trace/local.db`) with migrations before write attempts, persists trace records to local Turso-backed tables, maintains commit-level emission ledger (`sce/trace-emission-ledger.txt`), and enqueues fallback entries (`sce/trace-retry-queue.jsonl`) when a persistence target fails.
- `post-rewrite`: resolves runtime guards, ensures persistence paths are available, ingests parsed rewrite remap pairs into local DB-backed `rewrite_mappings` with deterministic idempotency, and runs rewritten-trace finalization (`finalize_rewrite_trace`) per accepted remap using canonical notes + DB writers, shared emission ledger, and retry queue adapters.
- `post-rewrite` output now reports both remap ingestion counters and rewrite trace finalization counters (`persisted`, `queued`, `no_op`, `failed`) for deterministic operator diagnostics.

## Notes for next tasks
- T02 established routing and invocation contracts.
- T03 implemented pre-commit staged-checkpoint runtime wiring.
- T04 implemented commit-msg file IO mutation wiring to canonical co-author policy.
- T05 implemented post-commit persistence adapters and runtime wiring.
- T07 implemented production post-rewrite runtime orchestration (remap ingestion + rewritten trace emission).
