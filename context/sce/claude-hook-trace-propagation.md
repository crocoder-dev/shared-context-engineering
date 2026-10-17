# Claude Code Hook Trace-Context Propagation

Probe evidence and verdict for whether Claude Code passes W3C trace context to command hooks, recorded by T01 of `context/plans/claude-hook-trace-propagation.md`. This file is the durable source for the parent-context decision consumed by T02–T05; it does not describe any shipped SCE behavior yet.

## Verdict

**`parent_context_available`**, with two binding qualifications:

1. **The parent is the `claude_code.interaction` span, not the tool span.** Every `PreToolUse` and `PostToolUse` command hook received a valid `TRACEPARENT` whose trace ID matched the exported Claude Code trace and whose parent span ID was the span ID of the turn's `claude_code.interaction` root span. It never matched `claude_code.tool`, `claude_code.tool.execution`, or (with detailed beta tracing) the `claude_code.hook` span that wraps the hook run. An SCE `sce.hook` span parented on this context is therefore a **sibling** of `claude_code.tool` and `claude_code.hook` under the interaction, not their child. Hook-to-tool correlation must use `tool_use_id`, which appears both in the hook stdin payload and as the `tool_use_id` attribute on Claude's `claude_code.tool` and `claude_code.tool.execution` spans.
2. **The context exists only when Claude Code tracing is active.** `TRACEPARENT` is present only when both `CLAUDE_CODE_ENABLE_TELEMETRY=1` and `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1` are set (so Claude Code has a live span). With telemetry off, with telemetry on but the tracing beta off, or with only `CLAUDE_CODE_PROPAGATE_TRACEPARENT=1`, hooks receive no `TRACEPARENT`. SCE must treat a missing traceparent as the normal case and export an independent root.

`TRACESTATE` was never present. The hook stdin payload carries no trace fields.

Deviation from plan wording: the plan's change summary assumed parenting "to Claude Code's active tool span". T03 should parent `sce.hook` on the provided interaction context as recorded here; parenting under the tool span is not possible from the hook process with Claude Code 2.1.301.

## Claude-caller signal

Presence-only environment markers observed in every probed hook process, compared with the Claude Code Bash tool environment of the same Claude Code version:

| Marker | Hook process | Bash tool subprocess |
| --- | --- | --- |
| `CLAUDECODE` | present | present |
| `CLAUDE_CODE_SESSION_ID` | present | present |
| `CLAUDE_CODE_CHILD_SESSION` | present | present |
| `CLAUDE_PID` | present | present |
| `CLAUDE_CODE_ENTRYPOINT` | present | present |
| `AI_AGENT` | present | present |
| `CLAUDE_PROJECT_DIR` | present | absent |
| `CLAUDE_ENV_FILE` | absent | absent |

Selected signal for shared subcommands (`diff-trace`, `conversation-trace`, `policy bash`): **`CLAUDECODE` and `CLAUDE_PROJECT_DIR` both present**, checked for presence only, never read for value. `CLAUDECODE` alone is not hook-specific because every Claude Code subprocess inherits it, including commands the Bash tool runs. Claude Code documents `CLAUDE_PROJECT_DIR` as set for command hooks, stdio MCP servers, and plugin LSP servers, and the Bash tool does not receive it. The Claude-only subcommands (`claude-mutation-scope`, `claude-model-state`) are admitted from argv and do not need the signal. The generated `.claude/hooks/run-sce-or-show-install-guidance.sh` helper `exec`s `sce` with the inherited environment, so these markers and `TRACEPARENT` reach the SCE process unchanged.

Claude Code removes `OTEL_*` exporter variables from every subprocess it spawns (documented). SCE hook export must not depend on Claude's `OTEL_*` configuration; it keeps using SCE's pinned managed endpoint and stored credential.

## Evidence

Environment: Claude Code `2.1.301` (`service.version` resource attribute on exported spans), Linux, headless `claude -p` runs on 2026-10-17 from a scratch working directory outside this repository. The runs used `env -i` (no inherited `CLAUDECODE`/`TRACEPARENT`), `--setting-sources local`, and `--settings` pointing at a scratchpad file. That file registered a probe command hook with matcher `*` on `PreToolUse` and `PostToolUse` and pointed Claude Code's OTLP/HTTP-JSON trace exporter at a loopback receiver. User-level `~/.claude/settings.json` was not edited. Each run triggered real `Bash` and/or `Read` tool calls.

The probe captured only `session_id`, `hook_event_name`, `tool_use_id`, `TRACEPARENT`/`TRACESTATE` presence, traceparent validity, trace ID, parent span ID, trace flags, and presence-only caller markers. One extra run captured only the top-level key names of the hook stdin payload, not their values. Exported spans were inspected for name, trace ID, span ID, parent span ID, timestamps, `span.type`, `tool_name`, `tool_use_id`, and `hook_event` only.

### Run A — telemetry + tracing beta, Bash then Read

Session `7d0750a6-560e-4e42-9120-ed8f14304c30`, trace `aaddb6f35b4fcaf4260a454b6a278f74`.

| Hook | `tool_use_id` | traceparent | trace ID | parent span ID |
| --- | --- | --- | --- | --- |
| PreToolUse | `toolu_01KN1scistt4Kcj3WCWcLbEp` (Bash) | valid, flags `01` | `aaddb6f35b4fcaf4260a454b6a278f74` | `00809c8f510c99f9` |
| PostToolUse | `toolu_01KN1scistt4Kcj3WCWcLbEp` (Bash) | valid, flags `01` | `aaddb6f35b4fcaf4260a454b6a278f74` | `00809c8f510c99f9` |
| PreToolUse | `toolu_01FAZCUXyrFmYNd2R4zr28Tr` (Read) | valid, flags `01` | `aaddb6f35b4fcaf4260a454b6a278f74` | `00809c8f510c99f9` |
| PostToolUse | `toolu_01FAZCUXyrFmYNd2R4zr28Tr` (Read) | valid, flags `01` | `aaddb6f35b4fcaf4260a454b6a278f74` | `00809c8f510c99f9` |

Exported spans (same trace):

| Span | span ID | parent span ID | `tool_use_id` |
| --- | --- | --- | --- |
| `claude_code.interaction` | `00809c8f510c99f9` | none (`parent.source=none`) | — |
| `claude_code.llm_request` | `77020189eb8449ab` | `00809c8f510c99f9` | — |
| `claude_code.tool` (Bash) | `e385b69cbfed31e4` | `00809c8f510c99f9` | `toolu_01KN1scistt4Kcj3WCWcLbEp` |
| `claude_code.tool.blocked_on_user` | `0e4c901b3fb2b096` | `e385b69cbfed31e4` | — |
| `claude_code.tool.execution` | `242141c79233929f` | `e385b69cbfed31e4` | `toolu_01KN1scistt4Kcj3WCWcLbEp` |
| `claude_code.tool` (Read) | `9408f8abebb89bce` | `00809c8f510c99f9` | `toolu_01FAZCUXyrFmYNd2R4zr28Tr` |
| `claude_code.tool.blocked_on_user` | `abbeb106b84cd5d3` | `9408f8abebb89bce` | — |
| `claude_code.tool.execution` | `83fbb33ed6cdf6d9` | `9408f8abebb89bce` | `toolu_01FAZCUXyrFmYNd2R4zr28Tr` |
| `claude_code.llm_request` | `6661c65cfc1fc0d8` | `00809c8f510c99f9` | — |

All four hooks shared one parent across two different tools, which equals the interaction span ID. Timing confirms that hooks run outside the tool span. The Bash `PreToolUse` probe started at `…734978746818` ns, before `claude_code.tool` started at `…734988000000`. The `PostToolUse` probe started at `…735048034958` ns, after `claude_code.tool` ended at `…735039758337`.

### Run E — telemetry + tracing beta, Read, payload key names

Trace `7ce9b9cb88f26f47f24496b1cf1b828e`. Both hooks for `toolu_01Qn5H38gufu6cdCHsowFzNu` received a valid traceparent with parent `a5d5939e4fd7c534`, which equals `claude_code.interaction`. The `claude_code.tool` span was `d39aebf463d29ac8`.

Hook stdin top-level keys: `PreToolUse` had `cwd`, `effort`, `hook_event_name`, `permission_mode`, `prompt_id`, `session_id`, `tool_input`, `tool_name`, `tool_use_id`, `transcript_path`. `PostToolUse` had the same keys plus `duration_ms` and `tool_response`. Neither carried a trace field.

### Run F2 — detailed beta tracing (`ENABLE_BETA_TRACING_DETAILED=1`, `BETA_TRACING_ENDPOINT` loopback)

Trace `1c6afedd2f2dc795c4aaa2c94b35184b`. Both hooks for `toolu_019vSjT7iXZedtTDFXE7TucH` received parent `56d0e76fdb58fe2e`, which equals `claude_code.interaction`.

| Span | span ID | parent span ID | start → end (ns suffix) |
| --- | --- | --- | --- |
| `claude_code.hook` (`hook_event=PreToolUse`) | `70b2f799a056a4d0` | `56d0e76fdb58fe2e` | `942778000000 → 942793833861` |
| `claude_code.tool` | `0ccef232e0abf4dc` | `56d0e76fdb58fe2e` | `942795000000 → 942803155221` |
| `claude_code.hook` (`hook_event=PostToolUse`) | `308ab39a6ca018d8` | `56d0e76fdb58fe2e` | `942804000000 → 942818720789` |

Even when Claude Code exports its own `claude_code.hook` span, the hook process's `TRACEPARENT` names the interaction span, not the `claude_code.hook` span that wraps the hook run.

### Gating runs (Read tool, real tool call confirmed by `num_turns=2`)

| Run | Settings env | `TRACEPARENT` in hooks | Spans exported |
| --- | --- | --- | --- |
| B | telemetry on, tracing beta off | absent (Pre + Post) | none |
| C | telemetry off | absent (Pre + Post) | none |
| D | only `CLAUDE_CODE_PROPAGATE_TRACEPARENT=1` | absent (Pre + Post) | none |

### Implementation cross-check

The Claude Code 2.1.301 bundle contains a shared subprocess-environment builder. It sets `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ATTENDED`, `CLAUDE_PID`, `AI_AGENT` (agent source), and `CLAUDE_EFFORT`. It sets `TRACEPARENT` only when tracing is active (or `CLAUDE_CODE_PROPAGATE_TRACEPARENT` is set) and there is an active span in the current context. This is consistent with runs B–D: run D has the propagate flag but no active span, so no traceparent.

## Documentation consulted

- Claude Code monitoring docs, `https://code.claude.com/docs/en/monitoring-usage` (Traces (beta), TRACEPARENT propagation, span hierarchy, hook spans). The docs describe Bash/PowerShell subprocesses as inheriting the active tool execution span's context. They describe hook executions as children of `claude_code.interaction`. They say nothing about hook-process `TRACEPARENT`; the probe shows hook processes receive the interaction context.
- Claude Code hooks docs, `https://code.claude.com/docs/en/hooks` (hook environment: `CLAUDE_PROJECT_DIR`; `OTEL_*` removed from subprocesses; `prompt_id` matches the `prompt.id` event attribute). These docs do not mention `TRACEPARENT`.

## Not verified

- Interactive TUI sessions were not probed for hook `TRACEPARENT`. Claude Code documents that only `-p`/Agent SDK sessions read an inbound `TRACEPARENT`. The outbound subprocess builder is shared, and the interactive Bash tool environment of the same version carries a valid `TRACEPARENT` (observed during this task). Whether interactive hooks receive the same interaction-span context is expected but unconfirmed. T05 live verification must check it.
- Subagent (`Agent` tool) hooks, `PostToolUseFailure`, `PermissionDenied`, and other hook events were not probed.
- Behavior behind a custom `ANTHROPIC_BASE_URL` proxy without `CLAUDE_CODE_PROPAGATE_TRACEPARENT` was not probed. Per the docs the subprocess `TRACEPARENT` follows the same switch as the API `traceparent` header.

## Consequences for the plan

- T02: admit the Claude-only subcommands from argv. Admit shared subcommands only when `CLAUDECODE` and `CLAUDE_PROJECT_DIR` are both present.
- T03: the parent-context branch applies. Extract `TRACEPARENT` (and `TRACESTATE` when present) with `TraceContextPropagator` and parent `sce.hook` on it. The resulting span is a child of `claude_code.interaction` and shares its trace ID. Missing or invalid context, which is the default when Claude tracing is off, yields an independent SCE root.
- T05 and context sync must describe the hierarchy as `claude_code.interaction → sce.hook` (sibling of `claude_code.tool`), with tool correlation by `tool_use_id`, not as a tool-span child.

## Probe hygiene

The probe hook, receiver, settings, captured payloads, and exported spans lived only in the session scratchpad and were deleted after this record was written. No probe code is in the repository, and user-level Claude settings were never modified.
