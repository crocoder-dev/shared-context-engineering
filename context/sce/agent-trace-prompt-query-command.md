# Agent Trace prompt query command

T06 implements the prompt-query read surface for `agent-trace-prompt-capture`.

## Current-state contract

- CLI entrypoint: `sce trace prompts <commit-sha>`.
- Runtime entrypoint: `cli/src/services/trace.rs` via `cli/src/app.rs` and `cli/src/cli_schema.rs`.
- The command resolves the current repository root with `git rev-parse --show-toplevel`, then scopes prompt lookup to the matching persisted repository row in the local Agent Trace DB.
- Commit references accept either a full persisted SHA or a unique persisted prefix; ambiguous prefixes fail with a deterministic longer-prefix remediation message.
- Prompt rows are read from the local `prompts` table only; the command does not read git notes.

## Output contract

- Default text output prints commit metadata (`Commit`, `Harness`, `Model`, `Branch`, `Total prompts`) followed by prompt rows ordered by `turn_number` then `captured_at`.
- Prompt rows include the captured timestamp, `cwd`, derived duration, tool-call count, and a `[truncated]` marker when `is_truncated = true`.
- `--json` switches the command to machine-readable output for this command surface.
- JSON output includes stable top-level fields: `status`, `command`, `subcommand`, `commit`, `harness`, `model`, `branch`, `prompt_count`, and `prompts`.
- Each JSON prompt entry includes `turn_number`, `text`, `length`, `cwd`, `duration_ms`, `tool_call_count`, `captured_at`, and `is_truncated`; truncated prompts also include `original_length`.

## Failure behavior

- If the current repository has no persisted Agent Trace data, the command returns an actionable runtime error.
- If no persisted commit matches the supplied SHA/prefix, the command returns an actionable runtime error.
- If a short SHA matches more than one persisted commit, the command returns an explicit ambiguity error.
- If a commit exists in persistence without prompt rows, the command fails with guidance to rerun against a prompt-captured commit.

## Verification coverage

- `cargo test trace`
- `cargo test prompt_capture_flow_persists_and_queries_end_to_end`

## Related context

- `context/sce/agent-trace-prompt-persistence-metrics.md`
- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/plans/agent-trace-prompt-capture.md`
