# Claude Raw Hook Capture

## Current implemented slice

- Hidden/internal CLI route: `sce hooks claude-capture <event-name>`.
- Supported event names are exactly `SessionStart`, `UserPromptSubmit`, `PostToolUse`, and `Stop`.
- Unsupported event names are rejected during clap-to-runtime conversion with deterministic validation guidance.
- Runtime reads one JSON payload from STDIN, parses it as `serde_json::Value`, enriches `PostToolUse` artifacts with the model identity from the Claude transcript, pretty-prints the (possibly enriched) JSON, and writes one artifact under the active repository's `context/tmp/claude/` directory.
- Invalid JSON fails before the persistence seam, so no malformed capture artifact is written.
- Pkl-generated Claude project settings register capture hooks in `config/.claude/settings.json` for `SessionStart`, `UserPromptSubmit`, `PostToolUse`, and `Stop`.
- The generated `PostToolUse` hook group matches `Write|Edit|MultiEdit|NotebookEdit`.
- Each generated handler uses Claude Code command-hook exec form with `command: "sce"` and args `hooks`, `claude-capture`, and the event name.

## Path and artifact contract

- Repo-local path ownership lives in `cli/src/services/default_paths.rs` as `RepoPaths::claude_capture_tmp_dir()`.
- The path shape is `<repo>/context/tmp/claude/`.
- Capture artifacts use the shared hook trace filename pattern:
  - `<timestamp>-<attempt>-<event-name>.json`
  - timestamp format: `YYYY-MM-DDTHH-MM-SS-mmmZ`
  - attempt is zero-padded to six digits
- Artifact writes use atomic create-new semantics and retry on filename collision up to the shared trace-attempt limit.

## Runtime boundaries

- Claude capture is raw-payload diagnostic storage only.
- Generated settings registration only invokes the raw capture CLI route.
- It does not derive Claude diff traces.
- It does not write to AgentTraceDb.
- It does not modify OpenCode agent-trace behavior or `sce hooks diff-trace` behavior.
- Doctor integration validation for Claude settings remains outside the MVP boundary.

## Generated settings ownership

- `config/pkl/renderers/claude-content.pkl` owns the rendered Claude settings document.
- `config/pkl/generate.pkl` emits that document to `config/.claude/settings.json`.
- The settings file is a generated-owned project-shareable Claude settings artifact and is included in normal generated-output parity checks.

## PostToolUse model enrichment

- `build_claude_capture_artifact` in `cli/src/services/hooks/mod.rs` enriches `PostToolUse` artifacts with the model identity before serialization.
- Enrichment is gated to `PostToolUse` only; `SessionStart`, `UserPromptSubmit`, and `Stop` remain unchanged.
- The enrichment reads `transcript_path` and `tool_use_id` from the STDIN JSON payload, calls `extract_claude_transcript_model`, and injects `"model": "<name>"` as a top-level key in the captured artifact.
- If `transcript_path` or `tool_use_id` are missing, if the transcript is inaccessible, or if no matching assistant message is found, the artifact is written without a `"model"` field and a warning is logged.
- Existing non-model fields in the payload are preserved.

## Transcript model extraction helper

- Defined in `cli/src/services/hooks/claude_transcript.rs` as `extract_claude_transcript_model`.
- Reads a Claude JSONL transcript from an absolute path, scans assistant messages for a `tool_use` content block matching a given `tool_use_id`, and returns the `model` field from that message.
- Returns `None` gracefully for missing/unreadable files, malformed JSONL lines, unmatched IDs, or missing/non-string model fields.
- Wired in T01 of the `claude-capture-enrich-model` plan; activated by the PostToolUse enrichment in T02.
- `tempfile` added as `[dev-dependencies]` for inline transcript fixtures in unit tests.

## Test contract

- Parser coverage lives in `cli/src/cli_schema.rs`.
- Runtime conversion coverage lives in `cli/src/services/parse/command_runtime.rs`.
- Capture validation, pretty JSON serialization, invalid JSON no-write behavior, and collision retry coverage live in `cli/src/services/hooks/mod.rs`.
- Unit tests use pure/injected persistence seams rather than real filesystem fixtures so they stay compatible with the Nix sandbox test policy.
- Generated settings parity is covered by `nix run .#pkl-check-generated`; embedded asset compile/test coverage is covered by the CLI test derivation.

See also: [agent-trace-hooks-command-routing.md](./agent-trace-hooks-command-routing.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../context-map.md](../context-map.md)
