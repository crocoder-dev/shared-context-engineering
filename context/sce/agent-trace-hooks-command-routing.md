# Agent Trace Hooks Command Routing

## Scope
- Current trace-removal baseline for `cli/src/services/hooks/mod.rs`
- Focus: concrete `sce hooks` subcommand routing plus current minimal runtime behavior

## Implemented command surface
- `sce hooks pre-commit`
- `sce hooks commit-msg <message-file>`
- `sce hooks post-commit`
- `sce hooks post-rewrite <amend|rebase|other>`
- `sce hooks diff-trace`

## Parser and dispatch behavior
- `cli/src/app.rs` routes `hooks` through dedicated hook-subcommand parsing.
- `cli/src/services/hooks/mod.rs` owns deterministic runtime dispatch through `HookSubcommand` + `run_hooks_subcommand`.
- Invalid and ambiguous invocations return deterministic actionable errors pointing to `sce hooks --help`.

## Current runtime behavior
- Shared enablement gate:
  - env `SCE_ATTRIBUTION_HOOKS_ENABLED`
  - config `policies.attribution_hooks.enabled`
  - precedence: env over config file
  - default: disabled
- `commit-msg` is the only active attribution path.
  - Reads the message file as UTF-8.
  - Applies exactly one canonical trailer: `Co-authored-by: SCE <sce@crocoder.dev>`.
  - Writes back only when the attribution gate is enabled, `SCE_DISABLED` is false, and the transformed content differs.
- `pre-commit` is a deterministic no-op entrypoint.
- `post-commit` is a deterministic no-op entrypoint.
- `post-rewrite` is a deterministic no-op entrypoint.
- `diff-trace` reads STDIN JSON, validates required non-empty `sessionID`/`diff` plus required `u64` `time` (Unix epoch milliseconds), and writes one payload artifact per invocation to `context/tmp/<timestamp>-000000-diff-trace.json` with atomic create-new retry semantics so separate short-lived processes cannot overwrite same-millisecond artifacts.

## Explicit non-goals in the current baseline
- No checkpoint handoff file
- No git-notes persistence
- No local DB persistence
- No retry queue replay
- No rewrite remap ingestion
