# Decision: Admit Claude-invoked SCE hooks to managed trace export as one `sce.hook` span

Date: 2026-10-17
Status: Accepted
Plan: `context/plans/claude-hook-trace-propagation.md`
Task: `T02`

## Context

Managed OTLP export previously forced every `sce hooks ...` invocation, `sce policy bash`, and the hidden `sce auth telemetry-headers` helper to `NoopTelemetry`. That kept hook processes free of credential lookup and exporter shutdown latency, and it kept hook data off the network. Claude Code runs SCE hooks around every tool call. Correlating those hook runs with Claude Code's own exported traces requires SCE hook processes to export something. Probe evidence for Claude Code 2.1.301 showed two things. First, the Claude-only hook subcommands can be recognized from argv. Second, Claude command hooks can be told apart from the Claude Bash tool by the presence of both `CLAUDECODE` and `CLAUDE_PROJECT_DIR`.

## Decision

Claude-invoked SCE hook processes are admitted to managed trace export. Each admitted process exports exactly one root-of-process `sce.hook` span in place of `sce.command`.

- **Admitted from argv:** `sce hooks claude-mutation-scope` and `sce hooks claude-model-state`.
- **Admitted only when `CLAUDECODE` and `CLAUDE_PROJECT_DIR` are both present** (checked for presence only): `sce hooks diff-trace`, `sce hooks conversation-trace`, and `sce policy bash`.
- **Never admitted:** every other hook (commit, Codex, OpenCode, and Pi) and `sce auth telemetry-headers`. These stay on `NoopTelemetry`.
- **Exported attributes:** `sce.agent=claude`, `sce.hook.event`, `sce.hook.tool`, `sce.outcome`, `sce.error.class`, and the Claude `session_id` and `tool_use_id`. The two IDs are correlation-only span attributes.

## Rationale

- Admitting only Claude callers delivers the correlation value without changing the latency or privacy profile of other harnesses' hooks.
- Replacing `sce.command` instead of nesting under it keeps one span per hook process.
- Session and tool-use IDs are opaque identifiers, not payload content. They are the only fields that join an SCE hook run to Claude's `claude_code.tool` span.
- Presence-only environment checks avoid reading values.

## Alternatives considered

- **Keep all hooks on Noop** — rejected because it gives no way to correlate SCE hook work with Claude Code traces.
- **Admit every hook process** — rejected because it widens latency and data exposure to Codex, OpenCode, Pi, and commit hooks, which have no proven correlation need.
- **Use `CLAUDECODE` alone as the caller signal** — rejected because the Claude Bash tool also inherits it, so commands run by the agent would be misclassified as hooks.
- **Nest `sce.hook` under `sce.command`** — rejected because it adds a redundant wrapper span per hook process.

## Compatibility and risks

- This reverses the earlier guarantee that hooks never resolve managed telemetry.
- Admitted Claude hook processes, including the blocking `PreToolUse` path, can wait up to the existing `CREDENTIAL_LOOKUP_DEADLINE` (2s) plus `SHUTDOWN_TIMEOUT` (3s), under the runtime grace bound. Whether hooks need a tighter, hook-specific deadline remains an open question in the plan.
- Claude session and tool-use IDs now leave the machine when a valid telemetry credential is stored. They go only to the pinned managed collector.

## Guardrails

- Only the four string fields `hook_event_name`, `tool_name`, `session_id`, and `tool_use_id` are read from the hook payload for telemetry. Nothing is read when the current span is disabled.
- Tool input and response, prompts, `cwd`, transcript paths, file contents, raw payloads, the OpenBao token, and the accessor are never exported.
- Session and tool-use IDs are never metric attributes and are never used to derive a trace ID.
- Telemetry stays fail-open. Credential, exporter, or collector failures cannot change hook stdout, permission decisions, persisted state, or exit codes.
- Endpoint pinning, the bearer-only header, disabled redirects, and the existing bounded shutdown are unchanged.

## Consequences

- Claude-invoked SCE hook runs become visible as `sce.hook` spans. They can be joined to Claude Code tool spans by `tool_use_id`.
- Non-Claude hook processes keep their previous Noop behavior and cost.
- The admission predicate in `cli/src/app.rs` is now part of the telemetry privacy contract and is covered by an admission test matrix.

## Follow-up

- Parenting `sce.hook` on the Claude-provided `TRACEPARENT` and adding child spans are tracked by later tasks of the plan.

## References

- Plan: [`claude-hook-trace-propagation`](../plans/claude-hook-trace-propagation.md)
- Task: `T02`
- Current-state context: [`cli-observability-contract.md`](../sce/cli-observability-contract.md)
- Current-state context: [`claude-hook-trace-propagation.md`](../sce/claude-hook-trace-propagation.md)
- Current-state context: [`telemetry-credential.md`](../sce/telemetry-credential.md)
- Evidence: [`cli/src/app.rs`](../../cli/src/app.rs) (`only_claude_hook_invocations_are_admitted_to_managed_hook_telemetry`)
- Evidence: [`cli/src/services/managed_telemetry/trace_export.rs`](../../cli/src/services/managed_telemetry/trace_export.rs) (`claude_hook_export_posts_one_hook_span_with_correlation_ids_only`)
- Evidence: [`cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`](../../cli/src/services/hooks/claude_mutation_scope/lifecycle.rs) (`failing_managed_telemetry_cannot_change_claude_hook_behavior`)
- Related decision: [`Application-owned async command runtime`](2026-10-07-application-owned-async-command-runtime.md)
