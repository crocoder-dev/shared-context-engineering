# Codex native telemetry contract

This contract records the Codex OpenTelemetry probe, the shipped launcher and hook behavior, and live Dash0 verification. The minimum verified Codex CLI version is **0.172.0**.

## Provenance

- Binary: `codex-cli 0.172.0` (npm/bun distribution), Linux.
- Source reference: `openai/codex` tag `rust-v0.172.0`, commit `3d86e43b075adeba2193ec5c22ac92f55afd82e6`. The relevant files are `codex-rs/otel/src/headers_helper.rs`, `codex-rs/http-headers-helper/src/lib.rs`, `codex-rs/core/src/otel_init.rs`, `codex-rs/otel/src/provider.rs`, `codex-rs/otel/src/otlp.rs`, `codex-rs/config/src/types.rs`, `codex-rs/config/src/config_requirements.rs`, `codex-rs/config/src/loader/mod.rs`, `codex-rs/config/src/requirements_layers/layer.rs`, `codex-rs/hooks/src/engine/command_runner.rs`, `codex-rs/hooks/src/registry.rs`, `codex-rs/otel/src/trace_context.rs`, and `codex-rs/tui/src/cwd_prompt.rs`.
- Method: real `codex exec` runs, `exec resume`, `exec fork`, and a pseudo-terminal interactive start. Each run used process-local `-c` overrides that pointed all three exporters at a loopback OTLP/HTTP JSON receiver. A stub header helper printed a dummy `authorization` header and an `x-probe-repo` header that echoed its argv. A probe command hook was supplied through `-c hooks.<Event>=[...]` with `--dangerously-bypass-hook-trust`. The probe repositories, receiver, helper, and hook lived only in a session scratch directory. The user's `~/.codex/config.toml` was never modified. Model turns used the existing Codex login. No SCE credential was used and the production collector was never contacted.
- Config-precedence runs used a scratch `CODEX_HOME` without Codex authentication. Codex exported startup traces and logs before the model request failed.
- `TRACEPARENT` was unset for every run except the inherited-context probe, because the invoking shell carried a parent agent's `TRACEPARENT`.

## Header-helper invocation contract

| Question | Evidence | Verdict |
|---|---|---|
| Config keys | `otel.headers_helper` (string) and `otel.headers_helper_refresh_interval_ms` (non-zero integer). Source: `OtelConfigToml`. | Observed and source. |
| Argv form | A user-configured helper runs as `sh -c "<command>"` unless `CODEX_HOME/<command>` is an existing file, in which case that file runs directly with no arguments. Observed: `-c 'otel.headers_helper="<scratch>/helper.sh <repo-path>"'` delivered `<repo-path>` as `$1`. | Arguments work. The command is shell text, so the repository path must be shell-quoted. |
| Working directory | `cwd=$CODEX_HOME` (`~/.codex` in the observed run); `PWD=$CODEX_HOME`. Source: `HeadersHelper::start(config, &settings.codex_home, …)`. A helper from managed requirements must be an absolute path and runs from its own parent directory. | The working directory can never identify the repository. |
| Environment | The helper inherits the full Codex process environment, including an inherited `TRACEPARENT`. Codex adds no `CODEX_THREAD_ID`, `CODEX_SESSION_ID`, or repository variable. Stdin is `/dev/null`; stderr is discarded. | No repository or session context is supplied. |
| Stdout format | One JSON object of string header names and values. Header names are case-insensitive. Duplicates, non-UTF-8 output, output over 64 KiB, or a non-object fail the run. Source: `http-headers-helper::parse_headers`. | `{"Authorization":"Bearer <token>"}`, the current SCE output, is accepted. |
| Timeout | 10 s, after which the process group is killed. Source: `HELPER_TIMEOUT`. | Larger than `CREDENTIAL_LOOKUP_DEADLINE`, so the SCE deadline governs. |
| Refresh cadence | The default interval is 29 minutes (`DEFAULT_HEADERS_HELPER_REFRESH_INTERVAL_MS = 1_740_000`). Observed with `refresh_interval_ms=3000`: 17 runs at 3.01 s intervals over a 50 s session. The helper runs once at provider start. Exports wait for the first result. | Refresh is time-based only. A collector `401` does not trigger a refresh (no status handling in `otel/src/otlp.rs`). |
| Header caching | One worker per process serves every signal that has an explicit HTTP exporter. Each export uses the most recent helper result. Observed: each request's header carried the timestamp of the latest preceding helper run. | Cached between refreshes. |
| Failure recovery | Observed: while the helper exited `1` (about 12 s), no OTLP request reached the receiver and nothing was replayed afterwards. The first successful refresh restored export. Codex printed `WARNING: OTel headers helper failed; telemetry is not being exported: headers helper exited with status exit status: 1` to stderr once per process, and the session itself completed with exit 0. | Export is suspended and data is lost. Codex never falls back to stale or static headers. The session is not affected. |
| Precedence over static headers | Observed: with `OTEL_EXPORTER_OTLP_HEADERS="authorization=Bearer envtok,x-env=1"`, requests carried the helper's `authorization` and the environment's `x-env`. | Helper headers override static and environment headers. Other environment headers are merged in. |
| Redirects | `without_redirects()` whenever a helper is attached. Source: `otel/src/otlp.rs`. | Credentials do not follow collector redirects. |
| gRPC | The helper applies only to `otlp-http` exporters. | The managed path must use `otlp-http`. |

## Configuration precedence

The config layers in ascending precedence are: package, MDM admin, system `/etc/codex/config.toml`, cloud bundle, user `$CODEX_HOME/config.toml`, profile `$CODEX_HOME/<name>.config.toml`, cwd/tree/repo `.codex/config.toml`, and runtime (`-c`, UI). Requirements (`/etc/codex/requirements.toml`, `/etc/codex/managed_config.toml`, cloud requirements, MDM) are applied after the config layers and override them. Source: `config/src/loader/mod.rs`, `core/src/config/requirements.rs`.

| Case | Evidence | Verdict |
|---|---|---|
| Repository-local `otel` | `otel` is in `PROJECT_LOCAL_CONFIG_DENYLIST`. | A repository cannot set `otel.*`. |
| User `config.toml` exporters, helper only via `-c` | Observed (5a): every log and trace request, with the helper's header, went to the user-owned endpoint. | **Credential leak path.** Supplying only the helper hands the credential to any user-configured `otlp-http` exporter. |
| `-c` pins all three exporters over user config and profile | Observed (5b, 5e): every request went to the pinned endpoint and the user-owned receiver got nothing. Observed (5d): a profile-owned logs exporter that `-c` did not pin received the helper header. | `-c` beats user and profile layers per key. Every exporter key must be pinned, including `none` for a signal SCE does not export. |
| `OTEL_EXPORTER_OTLP_*_ENDPOINT` environment | Observed (5c): with `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` set to another port, requests still went to the explicit `-c` endpoint. An empty explicit endpoint falls back to the environment (source comment in `OtelHeadersHelperTargets::merge_layer`). | A non-empty explicit endpoint is authoritative. `OTEL_EXPORTER_OTLP_HEADERS` is still merged (see above). |
| Endpoint path | Observed: requests were posted to the configured endpoint string verbatim (`…/v1/traces`, `…/v1/logs`, `…/v1/metrics`). | Per-signal paths must be supplied explicitly. |
| Requirements override `-c` | `apply_otel_requirements` overwrites every set field and records a startup warning. A requirements-layer helper must be an absolute executable path from local system or MDM requirements, never cloud requirements. Source. | Enterprise OTEL settings win over SCE `-c` overrides. |
| Requirements exporter with a user helper | `headers_helper_targets()`: when requirements carry OTEL settings but no helper, a user helper applies only to signals whose requirements exporter is unset. When requirements carry a helper, that helper serves only that layer's non-empty HTTP exporters. Source. | Upstream already blocks a user-supplied helper from reaching a managed exporter. |
| Requirements forcing content logging | `log_user_prompt`, `log_agent_responses`, `log_guardian_assessments`, and `tool_result` are requirements fields. Source. | A requirements layer can force content into logs. `-c` cannot undo it. |
| Enterprise requirements observed live | Not observable. Writing `/etc/codex/requirements.toml` needs root and was out of scope. MDM and cloud requirements are not readable from the CLI. | Source-only. |

## Credential reach to non-canonical exporters

A helper header reaches every explicit `otlp-http` exporter in the effective config, whatever its endpoint (`provider.rs` gates only on the exporter kind). Three paths were identified:

1. A user or profile exporter for a signal that the launch arguments did not pin (observed, 5a and 5d).
2. An empty explicit endpoint falling back to `OTEL_EXPORTER_OTLP_*_ENDPOINT` (source).
3. A requirements-layer exporter. Upstream blocks this for a user helper (source). If requirements also set a helper, the requirements helper replaces SCE's and no SCE credential is involved.

## Attribution by invocation mode

| Mode | Evidence | Repository-safe when the helper repository is resolved from the launch directory |
|---|---|---|
| `codex` interactive, new session | Observed under a pty: `service.name=codex_cli_rs`. The helper got the launch-time argv and ran from `CODEX_HOME`. | Yes, for the first thread. |
| `codex exec` | Observed: `service.name=codex_exec`. The thread and hook `cwd` equal the launch directory. | Yes. |
| `codex exec resume <id>` without `-C` | Observed: launched from repository B, the session and every hook ran in repository A (the stored session cwd). | **No.** The launch directory does not identify the working repository. |
| Non-interactive resume with explicit `-C <dir>` | Observed: the session ran in `<dir>`. | Yes, when the repository is resolved from `-C`. Codex 0.172.0 accepts `codex exec -C <dir> resume <id>`. |
| `codex exec fork <id>` | Observed: the new session id ran in the launch directory, not in the source session's cwd. | Yes. |
| `codex resume` / `codex fork` (interactive) | Source: `tui/src/cwd_prompt.rs` can ask the user to choose between the session directory and the current one. The T01 directory probe used `exec resume`. | Unverified for telemetry. |
| `-C <dir>` | Observed with resume. The process working directory is irrelevant to the helper. | Resolve the repository from `-C` when present. |
| In-process directory change or multiple threads | Source and observed: the helper command is fixed per process, while threads (subagents, `/new`, in-TUI resume, app-server) each carry their own cwd. Subagents inherit the parent's cwd (observed). | Not safe for a thread whose cwd leaves the launch repository. This is a residual risk; Codex exposes no per-thread hook into the helper. |
| `codex app-server` / remote | Not probed. Multi-client and multi-thread by design. | Unproven. |

## Hook trace context

| Probe | Evidence | Verdict |
|---|---|---|
| `TRACEPARENT` unset in the Codex process | All 8 hook runs (`SessionStart`, `UserPromptSubmit`, `PreToolUse` ×2, `PostToolUse` ×2, `Stop`, `SessionEnd`) saw an empty `TRACEPARENT` and `TRACESTATE`. | Codex does not inject trace context into command hooks. |
| `TRACEPARENT=00-aaaa…aaaa-bbbbbbbbbbbbbbbb-01` inherited | All 12 hook runs, including subagent runs, saw exactly the inherited value. `bbbbbbbbbbbbbbbb` was not an exported Codex span. Codex parented 8 of its own root spans on it, and 2,236 of 2,522 spans carried trace ID `aaaa…`. | A hook `TRACEPARENT` is inherited process environment, never the current Codex span. |
| Source | `hooks/src/registry.rs` snapshots `std::env::vars_os()` when the session starts. `command_runner::build_command` replays that snapshot plus the handler's configured `env`. No code path writes `TRACEPARENT`. `otel/src/trace_context.rs` reads `TRACEPARENT` only as the inbound parent of Codex's own spans. Codex does export a `codex.hooks.command` span per hook run, but it is not propagated. | Confirmed. |

**Verdict:** SCE Codex hook spans must not be parented on `TRACEPARENT`. A `sce.hook` span from a Codex hook is an independent root with correlation attributes. Parenting on the inherited value would attach SCE spans to whatever trace launched Codex.

## Identifier mapping

| Hook stdin field | Native telemetry equivalent | Observed relationship |
|---|---|---|
| `session_id` | `conversation.id` on logs and spans; `thread.id`/`thread_id` (string) on some spans | Equal (for example `01a20dc9-9a1a-…`). The SCE identity is `cx_<session_id>` (`DIFF_TRACE_CODEX_SESSION_ID_PREFIX`). |
| `turn_id` | `turn.id` / `turn_id` span attributes | Equal. |
| `tool_use_id` | `call_id` on spans and the `codex.tool_result`/`codex.tool_decision` logs | Equal for `exec-…` ids. Model `call_…` ids appear on other spans. |
| `model` | `model` on metrics, logs, and the `session_task.turn` span | Equal. |
| `agent_id` (subagent hooks only), `agent_type` | Child thread `conversation.id`; `agent_name` (`/root/<name>`) on `codex.tool_result`; `session_source=subagent_thread_spawn_<parent-thread-id>_d<depth>` on metrics | `agent_id` equals the child `conversation.id`. Hook `session_id` stays the parent's. |
| `transcript_path` | none | `~/.codex/sessions/…/rollout-…-<session_id>.jsonl`. |
| — | `thread.id` as an integer | The OS thread number from `tracing`, unrelated to the Codex thread. Do not join on it. |
| — | `session.id` on `code_mode.grpc.*` spans | A code-mode runtime id. Not the conversation. |

Hooks get no `CODEX_THREAD_ID`, `CODEX_SESSION_ID`, or `CODEX_TOOL_CALL_ID` in their environment (observed empty). Identifiers come only from stdin.

## Exported data and observed semantics

Resource attributes on all signals: `service.name` (`codex_exec` | `codex_cli_rs`), `service.version=0.172.0`, `env` (from `otel.environment`, default `dev`), `telemetry.sdk.*`. Logs add `host.name`; metrics add `os`, `os_version`. No repository attribute exists.

| Data | Where | Observed semantics |
|---|---|---|
| Tokens per turn | Metric `codex.turn.token_usage` (histogram, delta temporality, one point per turn, attributes `token_type` ∈ {`input`, `cached_input`, `cache_write_input`, `output`, `reasoning_output`, `total`}, `model`, `session_source`, `originator`, `auth_mode`, `app.version`). Span `session_task.turn` attributes `codex.turn.token_usage.*` including `non_cached_input_tokens`. | `input` **includes** cached input: 69,087 = 50,816 cached + 18,271 non-cached; `total` = input + output (69,201). |
| Tokens per request | Span `handle_responses`: `gen_ai.usage.input_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens`, `gen_ai.usage.output_tokens`, `codex.usage.reasoning_output_tokens`, `codex.usage.total_tokens`. Log `codex.sse_event` (`response.completed`) token counts. | Per model request. |
| Cost | none | **Not exported.** There is no cost attribute or metric in any signal. |
| Cache | See tokens. | `cached_input` / `cache_read` = prompt-cache reads; `cache_write_input` = cache writes (0 in every observed run). |
| Tools | Metrics `codex.tool.call` (counter, `tool`, `success`, `sandbox`, `command_category`), `codex.tool.call.duration_ms`, `codex.exec_command.calls`, `codex.turn.tool.call` (per-turn count). Logs `codex.tool_decision`, `codex.tool_result`. Span `codex.hooks.command`. | Tool names are `exec`, `exec_command`, `apply_patch`, and so on. |
| Durations | `codex.turn.e2e_duration_ms`, `codex.turn.ttft.duration_ms`, `codex.turn.ttfm.duration_ms`, `codex.hooks.run.duration_ms`, many startup metrics. | Milliseconds. |
| Model | `model` attribute on metrics, logs, and turn spans. | — |
| Session or conversation on metrics | none | **Metric points carry no conversation or session id.** Only logs and spans carry `conversation.id`. |
| Repository | `cwd` on 2–3 spans per session (absolute path). | Repository attribution must come from the credential binding (collector dataset) or from `otel.span_attributes` (traces only). |
| Subagents | See Identifier mapping. | Attributable through `session_source` and the child `conversation.id`. |
| Volume | A trivial `exec` session exported about 1,800 spans in about 4 MB of OTLP JSON across 77 traces, mostly `fs.*` and `append_items`. Metrics are exported once at process shutdown for `exec`. | There is no trace-volume filter in `otel.*`. Recorded as a control-plane cost follow-up. |

### Privacy controls

| Content | Default | Control |
|---|---|---|
| User prompt (`codex.user_prompt` log `prompt`) | `[REDACTED]` (observed) | `otel.log_user_prompt` (default false). |
| Agent responses | Not logged | `otel.log_agent_responses` (default false; needs a log exporter). |
| Guardian rationales | Not logged | `otel.log_guardian_assessments` (default false). |
| Tool arguments (`codex.tool_result` `arguments`) | **Logged in full** (observed `{"cmd":"cat a.txt"}`) | **No config control** in 0.172.0. |
| Tool output (`codex.tool_result` `output`) | **Logged, capped at 2,048 bytes** (observed command output) | `otel.tool_result.max_bytes`. |
| Personal identifiers | `user.email` and `user.account_id` on every Codex log event (observed) | No config control. |
| Spans and metrics | No prompt, response, tool-argument, or output attributes observed. `cwd` paths appear on a few spans. | — |

The probe found prompt, response, and tool content on the log signal. Spans and metrics carry operational metadata. Some spans include absolute working directories.

## Implementation decision

1. **Hook spans ship without a launcher.** `sce hooks codex` and `sce hooks codex-mutation-scope` are admitted to managed telemetry as Codex-only hook subcommands. Each exports one `sce.hook` span (`sce.agent=codex`). Verified correlation attributes are `sce.codex.session_id` (the raw stdin `session_id`), `sce.codex.turn_id`, `sce.codex.tool_use_id`, `sce.codex.model`, and, for subagent hook runs, `sce.codex.agent_id`/`sce.codex.agent_type`. Every span is an independent root, with **no `TRACEPARENT` extraction** for Codex. `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` children reuse the existing contract.
2. **Native export requires a thin `sce codex` launcher.** A user-level `config.toml` install cannot identify the repository: the helper runs from `CODEX_HOME` with no repository context, and a fixed user-level command would attribute every repository to one credential. The launcher resolves the repository and effective access policy first, then `exec`s `codex` with process-local overrides only:
   - `otel.headers_helper` runs `'<absolute sce path>' auth telemetry-headers --repo '<repository root>'`. The launcher shell-quotes both paths.
   - `otel.headers_helper_refresh_interval_ms=300000` sets a five-minute timer.
   - `otel.trace_exporter` and `otel.metrics_exporter` use `otlp-http` with `protocol = "binary"` and the managed endpoint's `/v1/traces` and `/v1/metrics` paths respectively.
   - `otel.exporter = "none"` disables native log export. Codex 0.172.0 cannot redact tool arguments or user email from logs.
   - `otel.log_user_prompt`, `otel.log_agent_responses`, and `otel.log_guardian_assessments` are all `false`.

   The helper supplies the credential at runtime. The launcher passes no credential in its arguments and writes no Codex config. It adds no repository span attribute.
3. **Safe modes.** The launcher enables native telemetry for new interactive sessions, `exec`, `exec fork`, and `exec resume` with an explicit `-C` (or `--cd`) before `resume`. The repository comes from that directory when supplied. Interactive resume, interactive fork, worktree launches, and remote launches run without SCE native export. The [launcher reference](codex-native-telemetry-launcher.md#invocation-modes) lists the modes and skip notices. In-process thread switching can leave native export bound to the original launch repository.
4. **Conflict detection.** The launcher skips native telemetry and reports the reason, never overwriting anything, when any of the following holds:
   - `$CODEX_HOME/config.toml` or the selected profile file defines `otel.exporter`, `otel.trace_exporter`, `otel.metrics_exporter`, or `otel.headers_helper`, because the user owns an OTEL destination;
   - a readable `/etc/codex/requirements.toml`, `/etc/codex/managed_config.toml`, or `/etc/codex/config.toml` defines any `otel.*` key, because enterprise settings would override the pins or force content logging;
   - the caller's arguments include an `otel` or `otel.*` override, a visible destination environment variable points elsewhere, or the user or selected profile config cannot be read or parsed.
   MDM and cloud requirements cannot be read and are a residual risk. Upstream still prevents a user helper from reaching a managed exporter (`headers_helper_targets`).
5. **Helper repository context.** The hidden `--repo <path>` selects the directory for the policy gate, destination tripwire, `auth.db` presence check, and repository-binding lookup. Repository resolution and credential lookup share `CREDENTIAL_LOOKUP_DEADLINE`. The no-argument form continues to use Claude's launch directory. Codex passes its environment to the helper.
6. **Setup and doctor.** Setup leaves Codex OTEL configuration to the launcher. Doctor reports existing managed-telemetry health without a Codex-native delivery check.
7. **Decision record.** [Native Codex telemetry via the `sce codex` launcher](../decisions/2026-11-24-codex-native-telemetry-via-sce-codex-launcher.md).

## Claude/Codex parity matrix

The Claude column draws on the [credential contract](telemetry-credential.md) and [Claude hook propagation evidence](claude-hook-trace-propagation.md). The live checks below confirm Claude hook parenting and token and cost delivery. Other Claude cells describe the existing contract.

| Capability | Claude Code (SCE install) | Codex 0.172.0 with SCE |
|---|---|---|
| Activation | User-level settings with authorization from the launch directory | `sce codex` enables native export on supported, authorized launches. SCE hook spans can also export during plain `codex` launches. |
| Repository attribution | Helper uses the launch cwd; `vcs.*` on metrics and logs via `OTEL_METRICS_INCLUDE_REPOSITORY` | Repository credential binding selects the workspace dataset. Some native spans carry `cwd`. The launcher adds no repository ID to spans or metrics. |
| Helper refresh | Debounce 300 s; on demand | Fixed timer (300 s via launcher); start-up run; no refresh on 401 |
| Helper failure | Export unauthenticated or failing | Export suspended and data dropped; one stderr warning |
| Token usage | `claude_code.token.usage` counter; `input` excludes cache reads; separate `cacheRead`/`cacheCreation` | `codex.turn.token_usage` histogram per turn; `input` **includes** `cached_input`; also `cache_write_input`, `reasoning_output`, `total`. Not summable against Claude `input` without subtracting `cached_input` |
| Cost | `claude_code.cost.usage` (USD) | **None**; must be derived server-side from tokens and model |
| Session id on metrics | `session.id` attribute | **None** on metrics; `conversation.id` on spans only (logs off) |
| Model | `model` | `model` |
| Tool telemetry | Tool events and spans; `OTEL_LOG_TOOL_DETAILS=1` exports tool input | `codex.tool.call` counters and durations, tool spans; tool input not exported (logs off) |
| Prompt and response content | Prompts redacted by default | Prompts redacted; responses off; tool arguments and output excluded by disabling logs |
| Native log events | Exported | Disabled (privacy) |
| Native traces | Enhanced beta tracing | Exported; high volume |
| Subagent attribution | Agent names with tool details | Metrics use `session_source=subagent_thread_spawn_<parent>_dN`. The hook's `sce.codex.agent_id` matches the child's native `conversation.id`. |
| SCE hook spans | `sce.hook`, `sce.agent=claude`, `sce.claude.*` | `sce.hook`, `sce.agent=codex`, `sce.codex.*` |
| Hook span parenting | `TRACEPARENT` → `claude_code.interaction` span | Independent root; Codex never propagates current-span context |
| Hook-to-native join | `tool_use_id` ↔ `claude_code.tool` | `session_id` ↔ `conversation.id`, `turn_id` ↔ `turn.id`, `tool_use_id` ↔ `call_id` |

## Upstream limitations

- The helper runs from `CODEX_HOME` with no repository or thread context.
- Command hooks receive no current-span context.
- Tool arguments and the user email in logs have no redaction control.
- Metrics carry no session or conversation id.
- There is no cost signal.
- There is no trace-volume or target filter configuration.
- Refresh is timer-only, with no refresh on authentication failure.
- Requirements-layer OTEL settings override runtime `-c`.

## Live verification

On 2026-11-24, the SCE worktree build (`0.4.0-pre-alpha-v14` with the launcher) ran Codex CLI 0.172.0 against `https://otel.crocoderlab.dev` in the enrolled `codex-otel` repository. Queries used the Dash0 dataset `sce-crocoder` (SCE CroCoder). A receiver response alone is insufficient evidence of delivery.

| AC8 item | Observation |
|---|---|
| Native traces and repository attribution | At 14:33 UTC, `sce codex exec` completed session `01a20df9-d0af-7321-a5fb-2798282462f2` with exit 0 and no launcher skip notice. Native trace `ab5a6cad90b461dd3e44f9ddeb4477f5` contains the `codex_exec` `apply_patch` span. Its sampling spans report a `cwd` in `codex-otel`. The workspace dataset and those span attributes establish routing and repository context for this run. Native metrics lack a repository or session identifier. |
| Hook delivery and correlation | Dash0 returned ten `sce.hook` spans for that session. Trace `e3875fe9c047dd9f25f8d982050aebd8` contains the root `codex-mutation-scope` `PostToolUse(apply_patch)` hook with `sce.agent=codex`, `sce.outcome=success`, and `sce.codex.model=gpt-6.2-sol`. Its descendants are `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence`. The root's session, turn, and tool-use IDs match native `conversation.id`, `turn.id`, and `call_id` in the separate native trace. |
| Token, cache, model, tool, and duration metrics | A Dash0 query at 14:37 UTC found six `codex.turn.token_usage` series for `gpt-6.2-sol`, including `cached_input` and `cache_write_input`. It also found native `codex.tool.call` and `codex.tool.call.duration_ms` series with that model. These are dataset-level series in the run window. Per-session metric attribution is unavailable because the series lack a conversation ID. The token semantics come from the T01 probe. |
| Cost | A `codex.*cost*` metric query for `codex_exec` at 14:37 UTC returned no series, consistent with the T01 probe. Codex 0.172.0 supplies no native cost signal. |
| Native logs and content | The `codex_exec` log query returned zero records from 14:29 UTC through the positive runs. The launcher disables native logs, so live native log delivery is unavailable by design. The inspected native `codex.turn_input` and `apply_patch` span attribute keys had no prompt, response, patch, argument, or output content fields. These checks cover the sampled spans and the observed log window. |
| Subagent attribution | A second `sce codex exec` run at 14:54 UTC completed parent session `01a20e0c-e19e-7082-a863-4e03db041822`. The `SubagentStop` hook in trace `b14670c36c4a176ef79121dc62908b4c` carries the parent session, agent type `default`, and `sce.codex.agent_id=01a20e0d-0712-7f61-8b5b-0b81791f5a5f`. That ID equals `conversation.id` on the child's `run_turn` and `codex.sampling` in native trace `c7cf8d72a47f71dbeb6cdce08e7664f8`. The dataset also has a `codex.turn.token_usage` series with `session_source=subagent_thread_spawn_01a20e0c-e19e-7082-a863-4e03db041822_d1`. |
| Unchanged Claude telemetry | In the concurrent Claude window, trace `fc4a49e91a65212130c11d07a3ba1523` contains `sce.agent=claude` tool hooks parented to `claude_code.interaction`. Dash0 also returned `claude_code.token.usage` and `claude_code.cost.usage` series. |
| Failed authorization | An unenrolled scratch repository with consent enabled caused the launcher to report that no credential was authorized. Codex completed session `01a20e0e-a73a-7d92-b662-66eeaf2a86de`. A direct helper check exited nonzero with the fixed diagnostic and empty stdout. |
| Repository opt-out | A scratch Git repository used the enrolled repository identity. The helper initially authorized it. After setting repository `telemetry.enabled=false`, the helper denied it and Codex completed session `01a20e0e-8dc2-7202-9eb7-b3cc805d522f`. |
| Global consent withdrawal | A process-local XDG config root supplied a global SCE `telemetry.enabled=false` while the enrolled repository still granted consent. The helper denied it and Codex completed session `01a20e0e-6b9f-7222-8b2e-f50c6ccbfdf4`. The live check covered new launches. A running process keeps its cached headers until the next helper refresh. |
| Negative-run Dash0 check | The three denial runs started between 14:55:55 and 14:56:10 UTC. A query in `sce-crocoder` found zero spans matching their `conversation.id` or `sce.codex.session_id`. Native metrics have no session ID, so this query cannot establish per-session metric absence. |
| Credential replacement, expiry, and logout | Live delivery was not verifiable with the existing usable credential. Inducing these cases against the managed collector requires a disposable authenticated session with separately replaceable or expiring credentials. The isolated `telemetry_credential_storage` tests passed for replacement, expiry, and logout. The T01 loopback probe verified suspension and recovery at helper refresh. |
| Two repositories in different workspaces | Live routing was verified for one enrolled workspace. A second authorized workspace session and its enrolled repository were unavailable for this check. The isolated storage suite passed the cross-workspace rejection test. The unenrolled scratch repository only exercises failed authorization. |
| Preservation of user-owned and managed config | The real launcher detected a user-owned exporter in scratch `CODEX_HOME`, passed the original arguments to a Codex stub without telemetry overrides, and left both scratch and normal user config hashes unchanged. There was no local `/etc/codex` config or inspectable MDM/cloud fixture. The targeted launcher suite passed the injected managed-system conflict test. |

### Repeat the live smoke check

1. Use Codex 0.172.0, an enrolled repository with trusted SCE hooks, an authorized SCE login, and read access to its Dash0 workspace dataset. Use a synthetic turn and a disposable worktree for an `apply_patch` check.
2. Run `sce codex -C <repository> exec`. Retain the session ID and UTC window. Run a separate turn with a subagent for the child-identity check.
3. In Dash0, locate the native trace by `conversation.id` and the SCE hook by `sce.codex.session_id`. For a tracked tool, compare the turn and call IDs and check the independent root and operation children. For a subagent, compare `sce.codex.agent_id` to the child's `conversation.id`.
4. Query Codex token and tool metrics for the time window. Check that Codex-native logs are absent and inspect attribute keys on the turn and tool spans. Record only identifiers and metadata needed to reproduce the query.

## Control Plane follow-ups

- Map Codex token, cache, model, tool, and duration signals into ingestion and dashboards. Keep Codex's inclusive `input` separate from Claude's uncached input, or subtract `cached_input` before comparing them. Preserve each signal's units and temporality.
- Derive cost only where model pricing and cache categories are known. Label derived values separately from Claude's native USD cost metric.
- Join native and SCE traces by the observed session, turn, and tool IDs. Keep workspace routing separate from repository grouping, and avoid implying per-session or per-repository attribution for native metric series that lack those identifiers.
- Measure trace volume and decide which spans and attributes the collector needs to retain, including whether to retain absolute `cwd` values. Keep native logs disabled until Codex can redact tool content and personal identifiers.
- Verify replacement, expiry, logout, and routing across two enrolled workspaces with disposable live credentials. Recheck managed-config behavior in an enterprise environment.
