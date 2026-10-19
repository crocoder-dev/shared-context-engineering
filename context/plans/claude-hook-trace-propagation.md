# Plan: claude-hook-trace-propagation

## Change summary

Connect SCE's existing managed OTLP trace export (PRs #306/#325, `cli/src/services/managed_telemetry/trace_export.rs`) to Claude Code hook execution so a Claude-invoked SCE hook emits a dedicated `sce.hook` span, parented to Claude Code's active tool span when Claude Code actually passes valid W3C trace context to command hooks, and otherwise exported as an independent SCE trace carrying safe Claude session/tool correlation attributes.

This reverses one recorded behavior: today every `sce hooks ...`, `sce policy bash`, and `sce auth telemetry-headers` invocation is forced to `NoopTelemetry` by `is_hook_invocation` in `cli/src/app.rs` (asserted by `hook_invocations_never_resolve_managed_telemetry`, and by AC4 of `claude-native-otel-instrumentation`). This plan admits only Claude-invoked hook processes to managed export; Codex, OpenCode, and Pi hook processes and `sce auth telemetry-headers` stay `Noop`. No trace-context propagator or `trace_context` module exists in the repository today; the W3C extraction is new, but it reuses the pinned `opentelemetry`/`opentelemetry_sdk` 0.33.1 and `tracing-opentelemetry` 0.34.0 crates, the existing exporter, credential lookup, endpoint pinning, and bounded shutdown. Whether parent-child propagation is possible at all is decided by a live probe in T01, not assumed.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Durable context records, from real Claude Code execution of `PreToolUse` and `PostToolUse` around a real tool call, whether command hooks receive `TRACEPARENT`/`TRACESTATE` (or equivalent), whether the value is a valid W3C traceparent, whether its trace ID and parent span ID match a span Claude Code actually exported, the Claude Code version tested, and the documentation consulted; the probe captured only `session_id`, `hook_event_name`, `tool_use_id`, presence/validity flags, trace ID, and parent span ID, and no probe code remains in the repository.
  - Validate: Inspect `context/sce/claude-hook-trace-propagation.md` for the recorded evidence table and verdict; `git ls-files` shows no probe script; grep the recorded evidence for tool input, prompt text, `Bearer`, or token-shaped values (none).
- [x] AC2: With a valid stored telemetry credential, each Claude-invoked SCE hook process exports exactly one `sce.hook` root-of-process span (no additional `sce.command` span) to the pinned `/v1/traces` endpoint, carrying `service.name=sce`, `sce.agent=claude`, `sce.hook.event`, `sce.hook.tool` when present, and `sce.outcome`; Codex, OpenCode, and Pi hook invocations and `sce auth telemetry-headers` still resolve `NoopTelemetry`, and ordinary commands keep their unchanged `sce.command` span.
  - Validate: Loopback-receiver Rust tests decode the captured OTLP payload for a Claude `PostToolUse` fixture and assert span name, count, and attributes; admission-predicate unit tests cover Claude, non-Claude, and helper invocations; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`.
- [x] AC3: When T01 proved usable parent context: a valid upstream traceparent makes the `sce.hook` span share the upstream trace ID with its parent span ID equal to the upstream span ID, SCE span IDs are freshly generated, child spans inherit the trace across `.await` points and Tokio worker migration, and an invalid, malformed, or missing traceparent yields a new independent SCE root without panic or hook-behavior change. When T01 proved context unavailable: no extraction code ships, and the context file states the parent-child goal is blocked, why, and the smallest future bridge.
  - Validate: Loopback-receiver tests for valid, invalid, all-zero, and missing traceparent asserting trace ID / parent span ID; a multi-poll async test asserting child span trace IDs; or, for the unavailable branch, inspection of `context/sce/claude-hook-trace-propagation.md` and source grep showing no propagator wiring.
- [x] AC4: The `sce.hook` span carries Claude `session_id` and `tool_use_id` as span attributes only (never as metric attributes, never reconstructed into a trace ID), and no exported span or log contains tool input, prompts, file contents, repository paths, raw hook payloads, the OpenBao token, or the accessor.
  - Validate: Loopback-receiver test injecting secret- and payload-shaped fixture values asserts their absence from the decoded payload and presence of only the two correlation IDs.
- [x] AC5: Child spans `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` appear under `sce.hook` only when the corresponding operation actually executed in that hook process; a hook path that skips an operation exports no span for it.
  - Validate: Loopback-receiver tests over a mutation-scope fixture that executes all three and a fixture that executes none (e.g. a non-tracked tool) assert the exact span set and parent links.
- [x] AC6: Telemetry cannot change Claude hook behavior: with credential lookup failure, expired credential, refused or stalled receiver, or exporter timeout, hook stdout, stderr, exit code, permission decisions, Agent Trace DB rows, and mutation-trace state match the telemetry-disabled run, and process lifetime stays within the existing `CREDENTIAL_LOOKUP_DEADLINE` + `SHUTDOWN_TIMEOUT` bounds.
  - Validate: Parity tests running the same Claude hook fixture with Noop and with failing managed telemetry, comparing outputs and persisted state, and asserting elapsed-time bounds.
- [x] AC7: Standalone tracing is unchanged: `sce doctor` with a valid credential exports its `sce.command` span through the managed exporter with no Claude context required.
  - Validate: Existing `managed_trace_export_posts_allowlisted_span_with_bearer_only` and related trace_export tests still pass; live evidence for `sce doctor` recorded per AC8.
- [x] AC8: Live verification is recorded: `sce doctor` and a Claude Code session triggering SCE hooks in this repository both export through `https://otel.crocoderlab.dev` with their existing managed credentials, and the record states whether each trace was observed in the correct Dash0 workspace dataset (with trace IDs and, where proven, the parent relationship) or exactly what could not be verified; a collector 2xx alone is not recorded as delivery.
  - Validate: Inspect the live-verification section of `context/sce/claude-hook-trace-propagation.md` and the T05 completion record.
- [x] AC9: Current-state context describes hook telemetry admission, the `sce.hook` span contract and attributes, the parent-context verdict, correlation-ID privacy rules, and the opt-in live smoke procedure, without claiming distributed tracing unless AC3's parent relationship was verified live.
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

- [x] T02: `Export a dedicated sce.hook span for Claude hook invocations` (status:done)
  - Task ID: T02
  - Scope: In — narrow `is_hook_invocation` into a Claude-admission decision in `cli/src/app.rs`; add the `sce.hook` span (replacing `sce.command` for admitted hooks) in `trace_export.rs` with `sce.agent`, `sce.hook.event`, `sce.hook.tool`, `sce.outcome`, `sce.error.class`, and the two correlation IDs recorded from the parsed Claude payload; loopback tests for span shape, admission, secret/payload absence, credential failure, exporter failure/stall, bounded shutdown, and hook output/state parity. Out — parent-context extraction (T03), child spans (T04), non-Claude harnesses.
  - Dependencies: T01
  - Done when: AC2, AC4, AC6, and AC7 hold in local tests; `hook_invocations_never_resolve_managed_telemetry` is replaced by an admission test that keeps non-Claude hooks and `telemetry-headers` on `Noop`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app::tests`.
  - Completed: 2026-10-17
  - Files changed: `cli/src/app.rs`, `cli/src/services/managed_telemetry.rs`, `cli/src/services/managed_telemetry/trace_export.rs`, `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`, `cli/src/services/hooks/claude_model_state.rs`, `cli/src/services/hooks/diff_trace.rs`, `cli/src/services/hooks/conversation_trace.rs`, `cli/src/services/bash_policy.rs`
  - Result: `is_hook_invocation` replaced by `managed_span_kind(args, claude_hook_caller) -> Option<ManagedSpanKind>`. `hooks claude-mutation-scope` and `hooks claude-model-state` are admitted from argv. `hooks diff-trace`, `hooks conversation-trace`, and `policy bash` are admitted only when `CLAUDECODE` and `CLAUDE_PROJECT_DIR` are both present (presence only). Every other hook subcommand and `auth telemetry-headers` resolve `None` (Noop); every other command resolves `Command`. `RuntimeTelemetry::resolve`/`resolve_with` and `ManagedTraceTelemetry::new` take the span kind. For `ClaudeHook`, the root span is `sce.hook` (same `sce.managed.trace` target and filter, no `sce.command`), with `sce.agent=claude`, `sce.hook.event`, `sce.hook.tool`, `sce.claude.session_id`, `sce.claude.tool_use_id`, `sce.outcome`, and `sce.error.class`. `record_claude_hook_payload` returns early on a disabled span and copies only the four string fields `hook_event_name`, `tool_name`, `session_id`, and `tool_use_id`; it is called once after stdin is read in the five Claude entry points. Tests added: an admission matrix; a loopback test that decodes the OTLP protobuf (new test-only decoder `decode_spans`/`trace_spans`, reusable by T03/T04) and asserts exactly one root `sce.hook` span, its exact `sce.*` attribute set, `service.name=sce`, and the absence of paths, prompt, file content, tool response, `Bearer` value, token, accessor, and `sce.command`; credential failure and expiry resolve Noop for `ClaudeHook`; and a parity test that runs a four-event Claude mutation-scope fixture sequence (including a deny decision) under Noop, refused-receiver, and stalled-receiver managed telemetry, asserting identical outputs, seam (persistence) payloads, and adapter state, with shutdown under `SHUTDOWN_TIMEOUT` + 1s.
  - Assumptions: hooks reuse the existing `CREDENTIAL_LOOKUP_DEADLINE` and `SHUTDOWN_TIMEOUT` bounds per AC6, and the open question on a tighter hook deadline stays open; correlation attributes are named `sce.claude.session_id` and `sce.claude.tool_use_id`. AC6 persistence parity is proven at the mutation-scope seam (the Agent Trace DB write boundary), not by opening a real Agent Trace DB.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`: 22 passed, 0 failed (passed); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app::tests`: 12 passed, 0 failed (passed); additionally `nix flake check`: all checks passed (cli-tests, cli-clippy, cli-fmt, actionlint).
  - Context impact: important change. Hook telemetry admission reverses the recorded "hooks are always Noop" contract (`context/sce/cli-observability-contract.md`, `context/sce/telemetry-credential.md`, AC4 of `claude-native-otel-instrumentation` context), and the new `sce.hook` span contract and correlation-ID privacy rule affect `context/architecture.md` and `context/glossary.md`.
  - Context synchronization: synced

- [x] T03: `Parent the sce.hook span on Claude-provided W3C trace context` (status:done)
  - Task ID: T03
  - Scope: In — only when T01 recorded `parent_context_available`: extract the context Claude provides using `opentelemetry_sdk::propagation::TraceContextPropagator` before the `sce.hook` span is created, attach it with `OpenTelemetrySpanExt::set_parent`, keep it scoped to the instrumented hook future, and add loopback tests for valid, invalid, all-zero, and missing traceparent plus a multi-poll async child-span test. When T01 recorded `parent_context_unavailable`: make no code change, and record in the context file that the parent-child goal is blocked, why, and the smallest future bridge (correlating Claude OTLP export records with `sce.hook` via `session_id`/`tool_use_id`). Out — reconstructing trace IDs from Claude identifiers; global propagator registration beyond the hook boundary; non-Claude callers.
  - Dependencies: T02
  - Done when: AC3 holds for the branch T01 selected.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml trace_export`, or inspection of the context file for the unavailable branch.
  - Completed: 2026-10-17
  - Files changed: `cli/src/services/managed_telemetry/trace_export.rs`
  - Result: `parent_context_available` branch implemented. `ManagedTraceTelemetry` gained a `parent_context: Option<opentelemetry::Context>` field. Only for `ManagedSpanKind::ClaudeHook`, `ManagedTraceTelemetry::new` fills it from process env `TRACEPARENT`/`TRACESTATE`. The pure `extract_parent_context` helper does this by running `opentelemetry_sdk::propagation::TraceContextPropagator::extract` over a `HashMap` carrier, and it keeps the context only when the remote span context is valid. `root_span` calls `OpenTelemetrySpanExt::set_parent` on the `sce.hook` span and ignores its result (fail-open). The span is entered only through `.instrument(...)` on the hook future, so no global propagator is registered and the parent never leaks outside the hook boundary. `sce.command` and non-Claude paths never read the env. Tests added (loopback receiver plus the existing OTLP decoder):
    - A valid traceparent gives `sce.hook` the upstream trace ID, its parent span ID is the upstream span ID, and its own span ID is new.
    - A table of missing, empty, malformed, truncated, uppercase, version `ff`, all-zero trace ID, and all-zero span ID values each yields one independent root `sce.hook` with a new non-zero trace ID and no parent.
    - On a 4-worker runtime, a child span created after 8 polls and a grandchild inside a `tokio::spawn`ed task (`in_current_span` + `with_current_subscriber`) share the upstream trace ID with a correct parent chain.
  - Deviation: per T01, the upstream parent is `claude_code.interaction`, so `sce.hook` is a sibling of `claude_code.tool`, not its child.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml trace_export`: 13 passed, 0 failed (passed); additionally `nix flake check`: all checks passed (cli-tests, cli-clippy, cli-fmt).
  - Context impact: important change. `sce.hook` now joins Claude's trace when Claude Code's tracing beta supplies `TRACEPARENT`, otherwise it stays an independent root. This affects the span contract in `context/sce/cli-observability-contract.md` and the verdict/implementation notes in `context/sce/claude-hook-trace-propagation.md`; the glossary/architecture entries for `sce.hook` gain the parent-context rule.
  - Context synchronization: synced

- [x] T04: `Add mutation-scope, git-snapshot, and persistence child spans under sce.hook` (status:done)
  - Task ID: T04
  - Scope: In — spans on the managed-trace target at the existing execution sites: the Claude mutation-scope coordinate call (`sce.mutation_scope`), the mutation-trace Git snapshot/reconciliation call (`sce.git_snapshot`), and Agent Trace DB / mutation store persistence (`sce.persistence`), each with low-cardinality outcome attributes; extend the span filter so only these named spans export; loopback tests asserting the exact span set for an executing and a non-executing hook path. Out — spans inside pure protocol functions or arbitrary helpers; changes to ordering, locking, or failure semantics.
  - Dependencies: T03
  - Done when: AC5 holds, existing mutation-trace and Agent Trace tests pass unchanged, and no span is emitted for an operation that did not run.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`.
  - Completed: 2026-10-17
  - Files changed: `cli/src/services/managed_telemetry/trace_export.rs`, `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`, `cli/src/services/mutation_trace/runtime/coordinator.rs`, `cli/src/services/mutation_trace/runtime/scope_runtime.rs`
  - Result: Added `ManagedChildSpan { MutationScope, GitSnapshot, Persistence }` and `in_child_span(kind, future)`. The helper wraps the future in a span named `sce.mutation_scope`, `sce.git_snapshot`, or `sce.persistence` on the `sce.managed.trace` target, boxes the instrumented future, and records `sce.outcome=success|failure` from the `Result`. The span filter is now selected per span kind: a `Command` dispatch exports only `sce.command`, so ordinary commands that reach the coordinator still export exactly one span; a `ClaudeHook` dispatch exports only `sce.hook` and the three child names. Span sites:
    - `run_claude_mutation_scope_from_payload_with_resolver` wraps every seam (coordinate/abandon boundary) call in `sce.mutation_scope`.
    - `coordinate_boundary_inner` wraps `capture_and_pin` in `sce.git_snapshot`.
    - The post-snapshot store phase (initialize, register, provenance, CAS loop) moved unchanged into `persist_boundary`, which is wrapped in `sce.persistence`.
    - `abandon_scope_inner` wraps `abandon_protected` in `sce.persistence`.

    Spans are disabled without a managed hook dispatch. Ordering, locking, and errors are unchanged. Tests added:
    - A real temporary git repo and Agent Trace DB, with a real `coordinate` seam, run a Claude `Write` PreToolUse. It exports exactly `sce.hook` → `sce.mutation_scope` → {`sce.git_snapshot`, `sce.persistence`} in one trace, all with `success`.
    - A read-only `Read` PreToolUse exports only `sce.hook`.
    - A `Command` dispatch that runs all three child operations exports only `sce.command`.

    The T03 multi-poll test now uses the real child span names. New test helper `capture_requests_until` collects requests until shutdown, so batch timing cannot change the captured set.
  - Assumptions: `sce.persistence` covers the mutation-store phase of coordinate and abandon only. Taint persistence after a snapshot failure, and conversation/diff-trace Agent Trace DB writes, are not wrapped. `sce.git_snapshot` and `sce.persistence` are grandchildren of `sce.hook` through `sce.mutation_scope`. Child spans carry only `sce.outcome`.
  - Deviation: The first `nix flake check` failed `cli-clippy` with `large_futures` at 15 call sites, because the instrumented chain exceeded the 16 KiB future limit. Fixed in scope by `Box::pin`-ing the instrumented future inside `in_child_span`.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`: 7 passed, 0 failed (passed); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`: 26 passed, 0 failed (passed); additionally `nix flake check` after the fix: all checks passed (cli-tests incl. existing mutation-trace and Agent Trace suites, cli-clippy, cli-fmt).
  - Context impact: important change. The `sce.hook` span contract gains three operation child spans, plus the rule that spans are exported per span kind (`sce.command` alone for commands; `sce.hook` plus children for Claude hooks). This affects `context/sce/cli-observability-contract.md`, `context/sce/claude-hook-trace-propagation.md`, `context/cli/claude-mutation-scope-integration.md` (span boundaries over the adapter seam and coordinator), and the `context/glossary.md` `sce.hook` entry.
  - Context synchronization: synced

- [x] T05: `Record live Dash0 verification and sync hook-telemetry context` (status:done)
  - Task ID: T05
  - Scope: In — run `sce doctor` and a Claude Code session in this repository that triggers SCE hooks using a build with T02–T04; confirm both exporters use their existing managed credentials and the canonical collector, and check Dash0 for both traces in the correct workspace dataset, recording trace IDs, observed hierarchy, and anything unverified; write the opt-in live smoke procedure; update the context files listed under Context sync. Out — code changes; Collector/Control Plane changes; claiming delivery from a collector 2xx.
  - Dependencies: T04
  - Done when: AC8 and AC9 hold and the context file states the exact observed hierarchy.
  - Verify: Inspect the updated context files; `nix run .#pkl-check-generated` unaffected.
  - Completed: 2026-10-17
  - Files changed: `context/sce/claude-hook-trace-propagation.md`, `context/sce/cli-observability-contract.md`, `context/sce/telemetry-credential.md`, `context/context-map.md`
  - Result: Live run on 2026-10-17 used a debug build of `eae7be8f` and the existing stored credential (presence checked, value never printed). Dash0 evidence was a user-exported 30-span root-span listing; no trace detail view was provided.
    - Observed in Dash0: `sce doctor` exported one root `sce.command` (trace `1cc8e0b1eda96df1f757fab2e3798efd`). Headless Claude Code 2.1.301 session `96272753-…` (real `Write` + `Read` in this repo, branch `sce` on `PATH`, tracing beta on) exported root `claude_code.interaction` `dcd919ecc4f5d400df5b642a1a229783`/`3eec7153327c6839`, which is exactly the trace/span every tool hook's `TRACEPARENT` named (scratchpad probe). The session's `SessionStart` and `SessionEnd` `sce.hook` spans are independent roots with `sce.agent=claude`, the session ID, and `success`, because they run outside the interaction span. `sce auth telemetry-headers` produced no span.
    - Not verified: the parented tool-hook `sce.hook` spans and their `sce.mutation_scope`/`sce.git_snapshot`/`sce.persistence` children inside trace `dcd919ec…`. A root-span listing cannot show them (it also omits Claude's own `claude_code.tool` children). No unparented tool-hook `sce.hook` roots appear in the window. Also not verified: the Dash0 dataset name and interactive TUI hooks.
    - Follow-up (2026-10-17, user found the trace in Dash0): trace `dcd919ecc4f5d400df5b642a1a229783` is in the SCE CroCoder (`sce-crocoder`) dataset. Root `claude_code.interaction` (service `claude-code`, session `96272753-8c39-4ec3-824d-238f5202e016`, repo `crocoder-dev/shared-context-engineering`), started 12:32:16 UTC, about 7.0s, status UNSET, 0 descendants in error, 0 error or warning logs. It has 11 direct children (21 spans total): 3 `claude_code.llm_request`, 2 `claude_code.tool`, and 6 `sce.hook` spans from service `sce`. Most `sce.hook` spans took 1–26ms; the two with children took 111–154ms. This confirms the parented `sce.hook` spans and the dataset live. The names of the `sce.hook` children and which hook events the six spans belong to were not given in the evidence.
    - Recorded the evidence, observed vs. expected hierarchy, and an opt-in live smoke procedure in `claude-hook-trace-propagation.md`. Added session-boundary root behavior and live status to the observability contract, hook admission vs. helper bypass to `telemetry-credential.md`, and a context-map entry update.
  - Deviation: AC8's parent relationship was first recorded as unverified; the follow-up above now records it as observed in Dash0. The other context files still describe distributed tracing as locally tested, not live-verified.
  - Verify outcomes: context files inspected against T02–T04 code and Dash0 evidence — passed; secret grep of the context file for bearer values, the Dash0 token identifier, prompt/tool-input keys — no matches (passed); `nix run .#pkl-check-generated`: ephemeral generation passed, 142 files (passed); smoke file removed and `git status` shows only the four context files — passed.
  - Context impact: localized context-only change; the live-verification record and smoke procedure live in `context/sce/claude-hook-trace-propagation.md` with cross-references from the observability contract, telemetry credential doc, and context map; no code behavior change.
  - Context synchronization: synced

## Open questions

- Admitting Claude hooks to managed export adds an encrypted `auth.db` credential lookup (up to `CREDENTIAL_LOOKUP_DEADLINE`, 2s) and an exporter shutdown (up to `SHUTDOWN_TIMEOUT`, 3s) to every Claude hook process — several per tool call, including the blocking `PreToolUse` path. `managed-otel-export` and `claude-native-otel-instrumentation` excluded hooks deliberately. Is the existing bound acceptable for hooks, or should hooks get a tighter hook-specific deadline? A smaller version is to admit only `PostToolUse` hooks, leaving the blocking `PreToolUse` path on `Noop`.
- If T01 finds Claude Code does not pass trace context to command hooks, the remaining value is correlation attributes on independent traces. Is that still worth T02–T05, or should the plan stop after T01 and T03's documented blocker?
- The request asked for Rust 2024; the plan keeps edition 2021. Confirm, or plan the edition migration as a separate change.

## Validation Report

**Status:** validated  
**Date:** 2026-10-17

### Commands run

- `nix flake check` -> exit 0 (all checks passed for x86_64-linux, including cli-tests, cli-clippy, cli-fmt, cli-generated-input, pkl-generated, config-lib and npm Bun/Biome checks, workflow-actionlint)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry` -> exit 0 (26 passed, 0 failed)
- `git diff --name-only main...HEAD` plus `git status --porcelain -uall` -> exit 0 (branch changes limited to the planned code and context files; no probe files added; working tree holds only the T05 context edits)
- `grep -inE "bearer [A-Za-z0-9]|tool_input\"|user_prompt|prompt\"|sk-ant|hvs\.|token=|password" context/sce/claude-hook-trace-propagation.md` -> exit 1 (no matches)

### Success-criteria verification

- [x] AC1: Probe evidence recorded from real Claude Code hooks -> `context/sce/claude-hook-trace-propagation.md` has the verdict, Claude Code 2.1.301, per-hook traceparent tables for `PreToolUse`/`PostToolUse` matched against exported spans, gating runs, and the docs consulted; the secret/payload grep found nothing; the branch diff adds no probe files (the `probe*` fixtures in `git ls-files` predate this branch and belong to earlier adapter plans).
- [x] AC2: One `sce.hook` span per Claude hook; non-Claude hooks and the helper stay Noop -> `claude_hook_export_posts_one_hook_span_with_correlation_ids_only`, the `app::tests` admission matrix, and `managed_trace_export_posts_allowlisted_span_with_bearer_only` passed in the `managed_telemetry` and flake runs.
- [x] AC3: Parent-context branch -> `claude_hook_span_joins_valid_upstream_trace_with_fresh_span_id`, `claude_hook_span_starts_independent_root_without_usable_traceparent` (missing/malformed/all-zero table), and `claude_hook_child_spans_inherit_upstream_trace_across_polls_and_workers` passed.
- [x] AC4: Only correlation IDs, no payload or secrets -> `claude_hook_export_posts_one_hook_span_with_correlation_ids_only` asserts the exact `sce.*` attribute set and that paths, prompt, file content, tool response, Bearer value, token, and accessor are absent; passed.
- [x] AC5: Child spans only when the operation ran -> the mutation-scope loopback test (`sce.hook` → `sce.mutation_scope` → {`sce.git_snapshot`, `sce.persistence`}) and `read_only_claude_hook_exports_no_operation_children` passed in `nix flake check` cli-tests.
- [x] AC6: Telemetry cannot change hook behavior -> `failing_managed_telemetry_cannot_change_claude_hook_behavior` (Noop vs. refused vs. stalled receiver, output/seam/state parity, bounded shutdown) and `claude_hook_credential_failure_or_expiry_resolves_noop` passed.
- [x] AC7: Standalone tracing unchanged -> `managed_trace_export_posts_allowlisted_span_with_bearer_only` and the related trace_export tests passed; live `sce doctor` `sce.command` span (trace `1cc8e0b1eda96df1f757fab2e3798efd`) recorded.
- [x] AC8: Live verification recorded -> the Live verification section of the context file and the T05 record state the observed Dash0 spans and trace IDs, and say exactly what was not verified: the parented tool-hook hierarchy, the Dash0 dataset name, and interactive TUI hooks. Delivery is not inferred from a collector 2xx. Follow-up 2026-10-17: the user found trace `dcd919ecc4f5d400df5b642a1a229783` in the `sce-crocoder` dataset with 6 parented `sce.hook` spans under `claude_code.interaction` (see the T05 follow-up), so the parent relationship and dataset are now observed live; interactive TUI hooks remain unverified.
- [x] AC9: Current-state context synced -> `cli-observability-contract.md`, `telemetry-credential.md`, `claude-mutation-scope-integration.md`, `architecture.md`, `glossary.md`, and `context-map.md` describe admission, the `sce.hook` contract and children, the parent-context verdict, the correlation-ID privacy rule, and the smoke procedure. They describe distributed tracing as locally tested and not live-verified, which matches the code inspected.

### Failed checks and follow-ups

- None.

### Residual risks

- Resolved 2026-10-17: the user found trace `dcd919ecc4f5d400df5b642a1a229783` in the `sce-crocoder` Dash0 dataset with 6 `sce.hook` spans as direct children of `claude_code.interaction`, two of them with child spans. The child span names were not listed, so live delivery of `sce.mutation_scope`/`sce.git_snapshot`/`sce.persistence` by name is still proven only by local loopback tests.
- Interactive TUI sessions, subagent hooks, and other hook events were not probed for `TRACEPARENT`.
- Each Claude hook process can now take up to `CREDENTIAL_LOOKUP_DEADLINE` + `SHUTDOWN_TIMEOUT` longer, including the blocking `PreToolUse` path; the plan's open question on a tighter hook deadline is still unresolved.

## Follow-up validation: concurrent hook telemetry reliability (PR #330)

**Date:** 2026-10-19

### Change

- Root cause of missing live `sce.hook` spans (8 of 15 expected in an earlier run): the encrypted `auth.db` opened without Turso multiprocess WAL, so concurrent hook processes failed with `LockingError` and fell back silently to Noop. Fix and evidence: [Concurrent hook span loss](../sce/claude-hook-trace-propagation.md#concurrent-hook-span-loss), [auth-db.md](../sce/auth-db.md#concurrency-and-schema-readiness).
- Added `sce.hook.command`, `sce.telemetry.fallback` diagnostics, migration-free current-schema opens, serialized Busy-retried migrations, a 1000 ms auth DB busy timeout, and a telemetry lookup that never creates a missing `auth.db`.

### Commands run

- `nix flake check` -> exit 0 (all checks, including cli-tests, cli-clippy, cli-fmt, pkl-generated)
- `nix build .#checks.x86_64-linux.cli-tests --rebuild` ×2 -> all tests passed both times; the command exits 1 only on Nix's output-determinism comparison
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_db::multiprocess_tests` -> 8 passed
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential` -> 31 passed
- Multiprocess suite stress, 6 parallel copies × 8 rounds -> 48 of 48 passed
- Mutation checks: without `experimental_multiprocess_wal(true)`, 5 of 7 multiprocess tests fail; with unconditional migrations, the held-write-lock reader test fails

### Results

- 32 reader processes × 3 rounds: 32/32 each round (about 60–190 ms per round, debug build).
- 32 concurrent hook processes: every `(sce.claude.tool_use_id, sce.hook.command)` pair exported exactly once to a loopback collector. A wrong-key child resolves Noop with reason `database_error` and unchanged hook output.
- Mixed versions: unsafe for concurrent writers (silent lost update); sequential use and legacy reads are safe. See [Mixed-version rollout risk](../sce/auth-db.md#mixed-version-rollout-risk).
- Live: headless session `48a276ea-ce4a-4488-8244-649f20b87a42`, trace `132a62ceed757d0d8431011311ae76af`, 13 SCE hook invocations, zero local telemetry fallbacks. Dash0 delivery is **unverified**.

### Residual risks

- Dash0 confirmation of the 13 spans, their `sce.hook.command` values, and `tool_use_id` equality is still needed.
- Mixed old/new `sce` writers can lose updates; rollout guidance is in `auth-db.md`.
- Turso 0.8.1 has a multiprocess-WAL race when many processes concurrently create or migrate a DB under heavy load. It was not observed on read-only hook paths.
