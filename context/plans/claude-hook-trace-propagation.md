# Plan: claude-hook-trace-propagation

## Change summary

Connect SCE's existing managed OTLP trace export (PRs #306/#325, `cli/src/services/managed_telemetry/trace_export.rs`) to Claude Code hook execution so a Claude-invoked SCE hook emits a dedicated `sce.hook` span, parented to Claude Code's active tool span when Claude Code actually passes valid W3C trace context to command hooks, and otherwise exported as an independent SCE trace carrying safe Claude session/tool correlation attributes.

This reverses one recorded behavior: today every `sce hooks ...`, `sce policy bash`, and `sce auth telemetry-headers` invocation is forced to `NoopTelemetry` by `is_hook_invocation` in `cli/src/app.rs` (asserted by `hook_invocations_never_resolve_managed_telemetry`, and by AC4 of `claude-native-otel-instrumentation`). This plan admits only Claude-invoked hook processes to managed export; Codex, OpenCode, and Pi hook processes and `sce auth telemetry-headers` stay `Noop`. No trace-context propagator or `trace_context` module exists in the repository today; the W3C extraction is new, but it reuses the pinned `opentelemetry`/`opentelemetry_sdk` 0.33.1 and `tracing-opentelemetry` 0.34.0 crates, the existing exporter, credential lookup, endpoint pinning, and bounded shutdown. Whether parent-child propagation is possible at all is decided by a live probe in T01, not assumed.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: Durable context records, from real Claude Code execution of `PreToolUse` and `PostToolUse` around a real tool call, whether command hooks receive `TRACEPARENT`/`TRACESTATE` (or equivalent), whether the value is a valid W3C traceparent, whether its trace ID and parent span ID match a span Claude Code actually exported, the Claude Code version tested, and the documentation consulted; the probe captured only `session_id`, `hook_event_name`, `tool_use_id`, presence/validity flags, trace ID, and parent span ID, and no probe code remains in the repository.
  - Validate: Inspect `context/sce/claude-hook-trace-propagation.md` for the recorded evidence table and verdict; `git ls-files` shows no probe script; grep the recorded evidence for tool input, prompt text, `Bearer`, or token-shaped values (none).
- [ ] AC2: With a valid stored telemetry credential, each Claude-invoked SCE hook process exports exactly one `sce.hook` root-of-process span (no additional `sce.command` span) to the pinned `/v1/traces` endpoint, carrying `service.name=sce`, `sce.agent=claude`, `sce.hook.event`, `sce.hook.tool` when present, and `sce.outcome`; Codex, OpenCode, and Pi hook invocations and `sce auth telemetry-headers` still resolve `NoopTelemetry`, and ordinary commands keep their unchanged `sce.command` span.
  - Validate: Loopback-receiver Rust tests decode the captured OTLP payload for a Claude `PostToolUse` fixture and assert span name, count, and attributes; admission-predicate unit tests cover Claude, non-Claude, and helper invocations; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`.
- [ ] AC3: When T01 proved usable parent context: a valid upstream traceparent makes the `sce.hook` span share the upstream trace ID with its parent span ID equal to the upstream span ID, SCE span IDs are freshly generated, child spans inherit the trace across `.await` points and Tokio worker migration, and an invalid, malformed, or missing traceparent yields a new independent SCE root without panic or hook-behavior change. When T01 proved context unavailable: no extraction code ships, and the context file states the parent-child goal is blocked, why, and the smallest future bridge.
  - Validate: Loopback-receiver tests for valid, invalid, all-zero, and missing traceparent asserting trace ID / parent span ID; a multi-poll async test asserting child span trace IDs; or, for the unavailable branch, inspection of `context/sce/claude-hook-trace-propagation.md` and source grep showing no propagator wiring.
- [ ] AC4: The `sce.hook` span carries Claude `session_id` and `tool_use_id` as span attributes only (never as metric attributes, never reconstructed into a trace ID), and no exported span or log contains tool input, prompts, file contents, repository paths, raw hook payloads, the OpenBao token, or the accessor.
  - Validate: Loopback-receiver test injecting secret- and payload-shaped fixture values asserts their absence from the decoded payload and presence of only the two correlation IDs.
- [ ] AC5: Child spans `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` appear under `sce.hook` only when the corresponding operation actually executed in that hook process; a hook path that skips an operation exports no span for it.
  - Validate: Loopback-receiver tests over a mutation-scope fixture that executes all three and a fixture that executes none (e.g. a non-tracked tool) assert the exact span set and parent links.
- [ ] AC6: Telemetry cannot change Claude hook behavior: with credential lookup failure, expired credential, refused or stalled receiver, or exporter timeout, hook stdout, stderr, exit code, permission decisions, Agent Trace DB rows, and mutation-trace state match the telemetry-disabled run, and process lifetime stays within the existing `CREDENTIAL_LOOKUP_DEADLINE` + `SHUTDOWN_TIMEOUT` bounds.
  - Validate: Parity tests running the same Claude hook fixture with Noop and with failing managed telemetry, comparing outputs and persisted state, and asserting elapsed-time bounds.
- [ ] AC7: Standalone tracing is unchanged: `sce doctor` with a valid credential exports its `sce.command` span through the managed exporter with no Claude context required.
  - Validate: Existing `managed_trace_export_posts_allowlisted_span_with_bearer_only` and related trace_export tests still pass; live evidence for `sce doctor` recorded per AC8.
- [ ] AC8: Live verification is recorded: `sce doctor` and a Claude Code session triggering SCE hooks in this repository both export through `https://otel.crocoderlab.dev` with their existing managed credentials, and the record states whether each trace was observed in the correct Dash0 workspace dataset (with trace IDs and, where proven, the parent relationship) or exactly what could not be verified; a collector 2xx alone is not recorded as delivery.
  - Validate: Inspect the live-verification section of `context/sce/claude-hook-trace-propagation.md` and the T05 completion record.
- [ ] AC9: Current-state context describes hook telemetry admission, the `sce.hook` span contract and attributes, the parent-context verdict, correlation-ID privacy rules, and the opt-in live smoke procedure, without claiming distributed tracing unless AC3's parent relationship was verified live.
  - Validate: Inspect the files listed under Context sync against the implemented code.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`

### Context sync

- `context/sce/claude-hook-trace-propagation.md` (new: probe evidence, verdict, live verification, smoke procedure)
- `context/sce/cli-observability-contract.md`
- `context/sce/telemetry-credential.md` (hook admission now differs from the helper bypass)
- `context/cli/claude-mutation-scope-integration.md` (span boundaries over the adapter)
- `context/architecture.md`, `context/glossary.md`, `context/context-map.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/app.rs` (hook admission predicate, telemetry resolution), `cli/src/services/managed_telemetry.rs` and `managed_telemetry/trace_export.rs` (hook span, span filter, propagation), the Claude hook entry points in `cli/src/services/hooks/` (`claude_mutation_scope/`, `claude_model_state.rs`, Claude-classified paths of `diff_trace.rs`, `conversation_trace.rs`, and `sce policy bash`), span boundaries at existing call sites in `cli/src/services/mutation_trace/runtime/` (e.g. `git_snapshot.rs`) and Agent Trace DB persistence, focused tests, and the context files listed above.
- **Out of scope:** Codex, OpenCode, and Pi hooks/plugins; Pkl/generated Claude hook config; Claude settings written by `sce setup`; metrics; credential issuance, storage, or lifecycle; Control Plane or Collector changes; a collector, proxy, sidecar, daemon, or correlation engine; new public CLI commands; making `sce auth telemetry-headers` visible.
- **Constraints:** reuse the pinned OTEL crates, `TraceContextPropagator` from `opentelemetry_sdk`, `MANAGED_OTLP_*`, `load_credential_within_deadline`, and existing bounded shutdown; static dispatch, no new `dyn` abstractions or wrappers beyond what the span boundary needs; hooks stay fail-open; never derive a trace ID from Claude session or tool-use IDs; no code comments; Cargo only via `nix develop`; tests stay a minimal focused set using the existing loopback receiver; CI never needs live credentials, Claude Code, or Dash0.
- **Non-goal:** generic distributed-tracing support for every harness or every SCE command; span-instrumenting arbitrary internal functions.

## Assumptions

- The repository stays on Rust edition 2021 (`cli/Cargo.toml`, AGENTS.md); the request's "Rust 2024" is treated as "current Rust", since an edition migration is a separate cross-cutting change.
- `sce.hook` replaces `sce.command` for admitted Claude hook processes rather than nesting under it, so each hook process exports one root-of-process span and no redundant wrapper.
- Claude admission is decided before dispatch from argv for Claude-only subcommands (`claude-mutation-scope`, `claude-model-state`) and, for shared subcommands (`diff-trace`, `conversation-trace`, `policy bash`), from a cheap Claude-caller signal T01 confirms (presence-only check of a Claude-set hook environment marker); if no reliable signal exists, shared subcommands stay `Noop`.
- Claude `session_id` and `tool_use_id` are permitted as span attributes: they are opaque identifiers, not hook payload content, and the managed-otel-export privacy contract forbids payloads, paths, and secrets, not correlation IDs. The context sync records this explicitly.
- The "opt-in live smoke test" is a documented manual procedure in `context/sce/claude-hook-trace-propagation.md` plus recorded evidence, not a committed script or ignored Rust test, because driving Claude Code and Dash0 cannot run deterministically from the test suite.
- Probe scripts live only in the session scratchpad and user-level Claude settings changes for the probe are reverted after T01.
- The PR is opened after `/validate` passes, using the plan's recorded evidence for its description; it is not a task in this stack.

## Task stack

- [x] T01: `Probe and record Claude Code hook trace-context propagation` (status:done)
  - Task ID: T01
  - Scope: In — read Claude Code's current hooks and monitoring/OTEL documentation and any inspectable implementation; run a temporary scratchpad probe registered for `PreToolUse` and `PostToolUse` in a real Claude Code session with native telemetry enabled, triggering a real tool call; capture only `session_id`, `hook_event_name`, `tool_use_id`, `TRACEPARENT`/`TRACESTATE` presence, traceparent validity, trace ID, parent span ID, and presence-only Claude caller env markers; compare captured IDs against Claude Code's exported spans to decide whether the context corresponds to an active tool span; write `context/sce/claude-hook-trace-propagation.md` with evidence, Claude Code version, and verdict. Out — any Rust change; inferring hook behavior from Bash subprocess propagation; committing the probe.
  - Dependencies: none
  - Done when: The context file records a verdict of `parent_context_available` or `parent_context_unavailable` backed by captured evidence for both events, names the Claude-caller signal (or its absence), the probe is removed, and the user-level Claude settings are restored.
  - Verify: Inspect the context file; `git status` shows only the new context file; grep the file for prompt/tool-input/token content (none).
  - Completed: 2026-10-17
  - Files changed: `context/sce/claude-hook-trace-propagation.md` (new)
  - Result: Verdict `parent_context_available`, qualified. Headless Claude Code 2.1.301 runs with telemetry and the tracing beta on gave every `PreToolUse`/`PostToolUse` hook a valid `TRACEPARENT` with no `TRACESTATE`. Its trace ID matched the exported trace, and its parent span ID was the `claude_code.interaction` root span, never `claude_code.tool` or `claude_code.hook` (detailed beta tracing included). Hooks run outside the tool span; correlate with the tool by `tool_use_id`. Without the tracing beta, or with only `CLAUDE_CODE_PROPAGATE_TRACEPARENT=1`, no `TRACEPARENT` is present. Claude-caller signal for shared subcommands: `CLAUDECODE` and `CLAUDE_PROJECT_DIR` both present (presence only); `CLAUDE_PROJECT_DIR` is absent from the Bash tool environment. Interactive TUI hooks not probed; deferred to T05. The probe ran only in the scratchpad through `claude -p --settings`. It never edited user-level `~/.claude/settings.json` and was deleted afterward.
  - Deviation: the plan assumed parenting to Claude's active tool span; the evidence shows only the interaction span is available, so T03 parents `sce.hook` as a sibling of `claude_code.tool` under `claude_code.interaction`.
  - Verify outcomes: context file inspected (verdict, evidence for both events, caller signal, version, docs) — passed; `git status --porcelain -uall` shows only `?? context/sce/claude-hook-trace-propagation.md` — passed; grep of the file for `bearer|token=|sk-ant|hvs.|password|tool_input"|user_prompt"` and the probe file content returned no matches — passed; `git ls-files` contains no T01 probe code — passed.
  - Context impact: new current-state context file under `context/sce/` (probe evidence and verdict); needs a `context/context-map.md` entry; no code behavior change.
  - Context synchronization: synced

- [ ] T02: `Export a dedicated sce.hook span for Claude hook invocations` (status:todo)
  - Task ID: T02
  - Scope: In — narrow `is_hook_invocation` into a Claude-admission decision in `cli/src/app.rs`; add the `sce.hook` span (replacing `sce.command` for admitted hooks) in `trace_export.rs` with `sce.agent`, `sce.hook.event`, `sce.hook.tool`, `sce.outcome`, `sce.error.class`, and the two correlation IDs recorded from the parsed Claude payload; loopback tests for span shape, admission, secret/payload absence, credential failure, exporter failure/stall, bounded shutdown, and hook output/state parity. Out — parent-context extraction (T03), child spans (T04), non-Claude harnesses.
  - Dependencies: T01
  - Done when: AC2, AC4, AC6, and AC7 hold in local tests; `hook_invocations_never_resolve_managed_telemetry` is replaced by an admission test that keeps non-Claude hooks and `telemetry-headers` on `Noop`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app::tests`.
  - Context synchronization: pending

- [ ] T03: `Parent the sce.hook span on Claude-provided W3C trace context` (status:todo)
  - Task ID: T03
  - Scope: In — only when T01 recorded `parent_context_available`: extract the context Claude provides using `opentelemetry_sdk::propagation::TraceContextPropagator` before the `sce.hook` span is created, attach it with `OpenTelemetrySpanExt::set_parent`, keep it scoped to the instrumented hook future, and add loopback tests for valid, invalid, all-zero, and missing traceparent plus a multi-poll async child-span test. When T01 recorded `parent_context_unavailable`: make no code change, and record in the context file that the parent-child goal is blocked, why, and the smallest future bridge (correlating Claude OTLP export records with `sce.hook` via `session_id`/`tool_use_id`). Out — reconstructing trace IDs from Claude identifiers; global propagator registration beyond the hook boundary; non-Claude callers.
  - Dependencies: T02
  - Done when: AC3 holds for the branch T01 selected.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml trace_export`, or inspection of the context file for the unavailable branch.
  - Context synchronization: pending

- [ ] T04: `Add mutation-scope, git-snapshot, and persistence child spans under sce.hook` (status:todo)
  - Task ID: T04
  - Scope: In — spans on the managed-trace target at the existing execution sites: the Claude mutation-scope coordinate call (`sce.mutation_scope`), the mutation-trace Git snapshot/reconciliation call (`sce.git_snapshot`), and Agent Trace DB / mutation store persistence (`sce.persistence`), each with low-cardinality outcome attributes; extend the span filter so only these named spans export; loopback tests asserting the exact span set for an executing and a non-executing hook path. Out — spans inside pure protocol functions or arbitrary helpers; changes to ordering, locking, or failure semantics.
  - Dependencies: T03
  - Done when: AC5 holds, existing mutation-trace and Agent Trace tests pass unchanged, and no span is emitted for an operation that did not run.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`.
  - Context synchronization: pending

- [ ] T05: `Record live Dash0 verification and sync hook-telemetry context` (status:todo)
  - Task ID: T05
  - Scope: In — run `sce doctor` and a Claude Code session in this repository that triggers SCE hooks using a build with T02–T04; confirm both exporters use their existing managed credentials and the canonical collector, and check Dash0 for both traces in the correct workspace dataset, recording trace IDs, observed hierarchy, and anything unverified; write the opt-in live smoke procedure; update the context files listed under Context sync. Out — code changes; Collector/Control Plane changes; claiming delivery from a collector 2xx.
  - Dependencies: T04
  - Done when: AC8 and AC9 hold and the context file states the exact observed hierarchy.
  - Verify: Inspect the updated context files; `nix run .#pkl-check-generated` unaffected.
  - Context synchronization: pending

## Open questions

- Admitting Claude hooks to managed export adds an encrypted `auth.db` credential lookup (up to `CREDENTIAL_LOOKUP_DEADLINE`, 2s) and an exporter shutdown (up to `SHUTDOWN_TIMEOUT`, 3s) to every Claude hook process — several per tool call, including the blocking `PreToolUse` path. `managed-otel-export` and `claude-native-otel-instrumentation` excluded hooks deliberately. Is the existing bound acceptable for hooks, or should hooks get a tighter hook-specific deadline? A smaller version is to admit only `PostToolUse` hooks, leaving the blocking `PreToolUse` path on `Noop`.
- If T01 finds Claude Code does not pass trace context to command hooks, the remaining value is correlation attributes on independent traces. Is that still worth T02–T05, or should the plan stop after T01 and T03's documented blocker?
- The request asked for Rust 2024; the plan keeps edition 2021. Confirm, or plan the edition migration as a separate change.
