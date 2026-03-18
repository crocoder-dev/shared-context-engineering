# Agent Trace Prompt Capture Hook

T02 implements the Claude-side prompt capture entrypoint for `agent-trace-prompt-capture`.

## Current-state contract

- Canonical authored sources live in `config/pkl/lib/claude-settings.json`, `config/pkl/lib/claude-prompt-capture-hook.json`, and `config/pkl/lib/claude-capture-prompt.js`.
- Generated Claude outputs live at `config/.claude/settings.json`, `config/.claude/hooks/sce-capture-prompt.json`, and `config/.claude/hooks/sce-capture-prompt.js`.
- Claude registers a `UserPromptSubmit` hook that invokes `node "$CLAUDE_PROJECT_DIR"/.claude/hooks/sce-capture-prompt.js`.
- The hook appends one JSONL row per submitted prompt to `<resolved git dir>/sce/prompts.jsonl`.
- The append target uses `git rev-parse --git-dir`, so worktree repositories write into the worktree-specific git dir rather than assuming `.git/` is a directory.

## Captured fields

Each appended JSONL row currently contains:

- `session_id`
- `prompt`
- `cwd`
- `transcript_path`
- `timestamp`

The hook prefers Claude-provided environment variables (`SESSION_ID`, `USER_PROMPT`, `CWD`, `CLAUDE_PROJECT_DIR`) and falls back to hook JSON/stdin fields when present. `transcript_path` comes from the Claude hook stdin payload and points at the session JSONL transcript used later for tool-count derivation.

## Boundaries

- This slice captures prompt submissions and feeds the pre-commit checkpoint handoff via the Git-resolved `sce/prompts.jsonl` append target.
- Prompt-to-checkpoint ingestion is now implemented in `cli/src/services/hooks.rs` and documented in `context/sce/agent-trace-pre-commit-staged-checkpoint.md`.
- Model detection is resolved during checkpoint capture; tool counts, duration metrics, truncation, and DB persistence are now documented in `context/sce/agent-trace-prompt-persistence-metrics.md`.

## Related context

- `context/sce/agent-trace-pre-commit-staged-checkpoint.md`
- `context/sce/agent-trace-prompt-persistence-metrics.md`
- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/plans/agent-trace-prompt-capture.md`
