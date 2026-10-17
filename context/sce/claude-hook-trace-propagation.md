# Claude Code Hook Trace-Context Propagation

Probe evidence and verdict for whether Claude Code passes W3C trace context to command hooks, recorded by T01 of `context/plans/claude-hook-trace-propagation.md`. This file is the durable source for the parent-context decision consumed by T02–T05. Shipped hook admission and the `sce.hook` span contract are described in [cli-observability-contract.md](cli-observability-contract.md); parenting `sce.hook` on a valid `TRACEPARENT` (independent root otherwise) is shipped and described there.

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

## Live verification

Recorded by T05 on 2026-10-17. The run used an `sce` debug build of commit `eae7be8f` (T02–T04), built with `nix develop -c ./scripts/run-cli-cargo.sh build`. It used the existing stored telemetry credential: `sce auth telemetry-headers` exited 0 with non-empty output, and the output was not inspected. Both exporters sent to the canonical collector `https://otel.crocoderlab.dev`: SCE through its pinned endpoint, and Claude Code through the user-level `OTEL_EXPORTER_OTLP_ENDPOINT` with `otelHeadersHelper = sce auth telemetry-headers`. Delivery was judged only from Dash0, never from a collector response. The Dash0 evidence is a user-exported root-span listing for the last 30 minutes. It held 30 root spans, all with `parentSpanId` empty, ending at 12:32:23Z. A trace detail view was not provided.

### `sce doctor`

`sce doctor --format json` ran at 2026-10-17T12:31:35Z, exited 0, and took 0.76s. Dash0 shows one root `sce.command` span: trace `1cc8e0b1eda96df1f757fab2e3798efd`, span `3f1d92756bd5860a`, `sce.command.name=doctor`, `sce.outcome=success`, resource `service.name=sce`, `service.version=0.4.0-pre-alpha-v11`. It is an independent root, so the standalone command path is unchanged.

Other branch-build invocations in the same window, all also visible in Dash0:

- `sce --version` at 12:31:21Z, as `sce.command`.
- `sce auth --help` and the rejected `sce auth status` at 12:31:40Z, as two `sce.command` spans with `sce.command.name=auth`.

The later `sce auth telemetry-headers` calls produced no span, consistent with the helper staying Noop.

### Claude Code session with SCE hooks

Headless Claude Code `2.1.301` ran `claude -p --model haiku --allowedTools Write,Read` in this repository. The run used the repository's project hooks, user-level telemetry settings (`CLAUDE_CODE_ENABLE_TELEMETRY=1`, `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1`), and the branch build first on `PATH`. The inherited parent-session `TRACEPARENT`, `CLAUDECODE`, and session variables were unset. The session made a real `Write` of a gitignored `context/tmp/` file, then a `Read` of it, and the file was deleted afterward. Session `96272753-8c39-4ec3-824d-238f5202e016`, `num_turns=3`.

A scratchpad-only probe hook, registered beside the SCE hooks through `--settings`, recorded the context every tool hook received. All four hooks resolved `sce` to the branch build.

| Hook | `tool_use_id` | `TRACEPARENT` trace ID | parent span ID |
| --- | --- | --- | --- |
| PreToolUse Write | `toolu_01ANo9Y6mnA876HcX4fnD7bn` | `dcd919ecc4f5d400df5b642a1a229783` | `3eec7153327c6839` |
| PostToolUse Write | `toolu_01ANo9Y6mnA876HcX4fnD7bn` | `dcd919ecc4f5d400df5b642a1a229783` | `3eec7153327c6839` |
| PreToolUse Read | `toolu_017JhhwrJk5d16ykAncy5mUy` | `dcd919ecc4f5d400df5b642a1a229783` | `3eec7153327c6839` |
| PostToolUse Read | `toolu_017JhhwrJk5d16ykAncy5mUy` | `dcd919ecc4f5d400df5b642a1a229783` | `3eec7153327c6839` |

Observed in Dash0, all from the same Dash0 ingestion token:

| Span | Service | Trace ID | Span ID | Parent | Attributes |
| --- | --- | --- | --- | --- | --- |
| `sce.hook` (12:32:14Z) | `sce` | `7dd41303f9bec7b68bb667b2fb39aa70` | `a9bc49c8d4a8c7ea` | none | `sce.agent=claude`, `sce.hook.event=SessionStart`, `sce.claude.session_id=96272753-…`, `sce.outcome=success` |
| `claude_code.interaction` (12:32:16Z) | `claude-code` | `dcd919ecc4f5d400df5b642a1a229783` | `3eec7153327c6839` | none | `session.id=96272753-…` |
| `sce.hook` (12:32:23Z) | `sce` | `d56f02bb11aeac1b1a2c0922d5245d05` | `7dcd68d04d9501b8` | none | `sce.agent=claude`, `sce.hook.event=SessionEnd`, `sce.claude.session_id=96272753-…`, `sce.outcome=success` |

Neither `sce.hook` span carried `sce.hook.tool`, `sce.claude.tool_use_id`, paths, or payload content. Both are correctly independent roots:

- `SessionStart` (`claude-model-state`) runs before the interaction span exists.
- `SessionEnd` (`claude-mutation-scope`) starts at 12:32:23.502Z, after the interaction ended at 12:32:23.257Z.

Neither process had a parent context to join.

### Observed hierarchy

Confirmed in Dash0:

- `sce doctor` → one root `sce.command`.
- Claude session-boundary hooks → independent root `sce.hook` spans.
- The Claude interaction root `claude_code.interaction` (`dcd919ec…/3eec7153327c6839`) is exactly the trace and span named by every tool hook's `TRACEPARENT`.

Expected but not verified in Dash0 for trace `dcd919ecc4f5d400df5b642a1a229783`:

```text
claude_code.interaction (3eec7153327c6839)
├── claude_code.tool (Write / Read)
└── sce.hook  ×N: PreToolUse/PostToolUse hooks (TRACEPARENT captured); UserPromptSubmit/Stop hooks not probed
    └── sce.mutation_scope (claude-mutation-scope hooks)
        ├── sce.git_snapshot (Write PreToolUse)
        └── sce.persistence
```

The root-span listing cannot show non-root spans: even Claude Code's own `claude_code.tool` and in-trace `claude_code.llm_request` children for this session are absent from it. So the parented `sce.hook` spans and their children could not be confirmed or ruled out. The listing does contain every root span in the 12:31–12:33 window, and it has no unparented `sce.hook` for `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, or `Stop`. So these hook spans either joined the interaction trace, as the local loopback tests prove they do, or were not delivered. Confirming the parent relationship in Dash0 requires opening trace `dcd919ecc4f5d400df5b642a1a229783` in the trace view. Distributed tracing between Claude Code and SCE is therefore implemented and locally tested, but not yet verified live. The Dash0 dataset name was not reported: SCE and Claude Code resources share one ingestion token, but which named dataset that token writes to was not checked.

## Opt-in live smoke procedure

This is manual, uses real credentials, Claude Code, and Dash0, and is never part of CI.

1. Confirm a stored credential without printing it: `out=$(sce auth telemetry-headers 2>/dev/null); [ -n "$out" ] && echo present`.
2. Build the branch: `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`.
3. Run `cli/target/debug/sce doctor`, and note the UTC time.
4. Optionally, add a scratchpad-only probe hook through `--settings` that records only `hook_event_name`, `tool_use_id`, and the trace ID and parent span ID parsed from `TRACEPARENT`. This captures the expected Claude trace ID.
5. From the repository root, with Claude tracing enabled (`CLAUDE_CODE_ENABLE_TELEMETRY=1`, `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1`), run `env -u TRACEPARENT -u TRACESTATE -u CLAUDECODE PATH="$PWD/cli/target/debug:$PATH" claude -p --allowedTools Write,Read "<write then read a file under context/tmp/>"`. Then delete the file.
6. In Dash0, find the `sce.command` span with `sce.command.name=doctor` at the noted time. Open the Claude trace by its trace ID, not a root-span list, and confirm:
   - each tool hook's `sce.hook` has `claude_code.interaction` as its parent and a `sce.claude.tool_use_id` that matches a `claude_code.tool` span;
   - the `Write` `PreToolUse` `claude-mutation-scope` hook carries `sce.mutation_scope` → `sce.git_snapshot` and `sce.persistence`;
   - `SessionStart`/`SessionEnd` `sce.hook` spans are independent roots.
7. Record the trace IDs and the observed hierarchy. Record anything that was not observed as unverified, and never record a collector 2xx as delivery.

## Not verified

- The parent-child relationship in Dash0 between `claude_code.interaction` and the tool-hook `sce.hook` spans, and the delivery of `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` spans. See [Live verification](#live-verification).
- Interactive TUI sessions were not probed for hook `TRACEPARENT`. T05 also used headless `claude -p`. Claude Code documents that only `-p`/Agent SDK sessions read an inbound `TRACEPARENT`. The outbound subprocess builder is shared, and the interactive Bash tool environment of the same version carries a valid `TRACEPARENT`. Whether interactive hooks receive the same interaction-span context is expected but unconfirmed.
- Subagent (`Agent` tool) hooks, `PostToolUseFailure`, `PermissionDenied`, and other hook events were not probed.
- Behavior behind a custom `ANTHROPIC_BASE_URL` proxy without `CLAUDE_CODE_PROPAGATE_TRACEPARENT` was not probed. Per the docs the subprocess `TRACEPARENT` follows the same switch as the API `traceparent` header.

## Consequences for the plan

- T02: admit the Claude-only subcommands from argv. Admit shared subcommands only when `CLAUDECODE` and `CLAUDE_PROJECT_DIR` are both present.
- T03: the parent-context branch applies. Extract `TRACEPARENT` (and `TRACESTATE` when present) with `TraceContextPropagator` and parent `sce.hook` on it. The resulting span is a child of `claude_code.interaction` and shares its trace ID. Missing or invalid context, which is the default when Claude tracing is off, yields an independent SCE root.
- Context describes the hierarchy as `claude_code.interaction → sce.hook` (a sibling of `claude_code.tool`), with tool correlation by `tool_use_id`, not as a tool-span child. Live status is recorded in [Live verification](#live-verification).

## Probe hygiene

The probe hook, receiver, settings, captured payloads, and exported spans lived only in the session scratchpad and were deleted after this record was written. No probe code is in the repository, and user-level Claude settings were never modified.
