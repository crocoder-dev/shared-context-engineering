# Claude Raw Hook Capture (removed)

## Removal summary

The `sce hooks claude-capture <event-name>` CLI route, `ClaudeCaptureEvent`, `HookSubcommand::ClaudeCapture`, the `claude_transcript.rs` enrichment module, and `RepoPaths::claude_capture_tmp_dir()` were removed in T05 of the `claude-typescript-model-cache-remove-rust-capture` plan.

Rust now exposes only normalized intakes for Claude/OpenCode editor runtimes:

- `sce hooks session-model` — STDIN JSON intake for normalized model attribution upsert in `session_models`, keyed by `(tool_name, session_id)`. No raw hook artifacts are written.
- `sce hooks diff-trace` — STDIN JSON intake for normalized or Claude structured diff-trace payloads with optional/nullable attribution. When `model_id` or `tool_version` is missing, Rust resolves available values from `session_models` by `(tool_name, session_id)` and otherwise persists nullable attribution to AgentTraceDb without writing raw hook artifacts.

## Historical artifact contract

Before removal, the raw capture route:

- Was a hidden/internal CLI route: `sce hooks claude-capture <event-name>`.
- Accepted `SessionStart`, `UserPromptSubmit`, `PostToolUse`, and `Stop`.
- Wrote pretty-printed JSON artifacts to `context/tmp/claude/<timestamp>-<attempt>-<event-name>.json`.
- Enriched `PostToolUse` artifacts with the model identity from the Claude transcript.
- Did not write AgentTraceDb or derive diff traces.

The generated Claude TypeScript runtime at `config/.claude/plugins/sce-agent-trace.ts` previously forwarded the original payload to `sce hooks claude-capture` before deriving normalized `diff-trace` payloads. That raw-capture forwarding was removed in T04 when the TypeScript runtime switched to sending only normalized `session-model` and `diff-trace` payloads.

## Current state

- Claude settings call `sce hooks` directly via generated `.claude/settings.json` command hooks: `SessionStart` pipes raw hook event JSON to `sce hooks session-model`, matched `PostToolUse Write|Edit|MultiEdit|NotebookEdit` pipes raw hook event JSON to `sce hooks diff-trace`. Rust handles extraction, validation, and persistence without a TypeScript intermediary.
- The former Claude TypeScript runtime at `config/.claude/plugins/sce-agent-trace.ts` was removed in T07 of the `claude-rust-diff-trace` plan.
- Rust owns normalized persistence: `session-model` upserts into `session_models`, `diff-trace` inserts into `diff_traces` with `payload_type` classification (`"patch"` for OpenCode, `"structured"` for Claude).
- Claude `diff-trace` missing `model_id` and `tool_version` values are resolved from `session_models` at persistence time when available, otherwise stored as nullable attribution; OpenCode sends `model_id` directly and may send nullable `tool_version`.
- No raw Claude hook payload artifacts are written by TypeScript or Rust.

See also: [agent-trace-hooks-command-routing.md](./agent-trace-hooks-command-routing.md), [../context-map.md](../context-map.md)
