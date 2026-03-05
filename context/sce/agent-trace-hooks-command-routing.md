# Agent Trace Hooks Command Routing

## Scope
- Plan: `agent-trace-local-hooks-production-mvp`
- Task: `T02`
- Focus: implemented `sce hooks` subcommand routing and hook invocation contract validation.

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
- `post-commit`: resolves runtime guards, builds commit attribution input from git + pre-commit checkpoint artifacts, executes `finalize_post_commit_trace`, writes canonical note payloads to `refs/notes/agent-trace`, persists trace records to git-path JSONL storage (`sce/trace-records.jsonl`), maintains commit-level emission ledger (`sce/trace-emission-ledger.txt`), and enqueues fallback entries (`sce/trace-retry-queue.jsonl`) when a persistence target fails.
- `post-rewrite`: reads hook pair input from STDIN, validates pair format through remap finalization parsing, and reports ingested/skipped outcomes.

## Notes for next tasks
- T02 established routing and invocation contracts.
- T03 implemented pre-commit staged-checkpoint runtime wiring.
- T04 implemented commit-msg file IO mutation wiring to canonical co-author policy.
- T05 implemented post-commit persistence adapters and runtime wiring.
- Rewrite-trace production runtime wiring remains in `T07+`.
