# Plan: claude-sce-owned-telemetry

## Change summary

This plan replaces the SCE-managed Claude Code native OpenTelemetry integration with Claude telemetry that the Rust SCE CLI collects, reconstructs, and exports itself. Today `sce setup` writes Claude's native OTel `env` block and `otelHeadersHelper = "sce auth telemetry-headers"` into user-level Claude settings (`cli/src/services/setup/claude_telemetry.rs`, `cli/src/services/setup/config_merge.rs`). Claude Code then exports `claude_code.*` metrics to `https://otel.crocoderlab.dev` with a token from the helper, which Claude re-invokes on a 300 000 ms debounce. After this change, the existing Claude hooks feed hook payloads and transcripts into a new `cli/src/services/hooks/claude_telemetry/` module. That module rebuilds GenAI spans and the minimal `claude_code.*` metrics the warehouse consumes, and exports them through the existing pinned, bearer-authenticated managed OTLP path. Claude Code no longer receives SCE telemetry configuration or credentials.

The work reuses what already exists:

- The Claude transcript bounds (`hooks/claude_transcript.rs`) and subagent transcript derivation (`hooks/claude_model_resolver.rs`).
- Repository identity (`managed_telemetry/claude_helper.rs::repository_id_for`) and the access-policy resolver.
- Repository-bound credential lookup (`auth_command::authorized_repository_credential_with`, `telemetry_credential_storage::load_repository_telemetry_credential`).
- The pinned endpoint constants and pinned bearer `reqwest` client (`managed_telemetry.rs`, `managed_telemetry/trace_export.rs`).
- The exact-value ownership removal path (`setup/claude_telemetry.rs`).

Parsing and accounting knowledge, fixtures, and edge cases come from the Dash0 reference implementation `https://github.com/dash0hq/dash0-agent-plugin`, pinned at commit `093fb794a98a04f935eb91d7865aac959cc35eb0` (release `v0.1.34`, 2026-11-25, Apache-2.0). Dash0 is a source of empirical knowledge and regression fixtures, not an architecture to reproduce.

## Scope boundary: warehouse migration and release

- This plan is self-contained within the `shared-context-engineering` repository. Every task is implemented, verified, committed, and context-synchronized in a single checkout of this repository.
- The warehouse migration is outside the scope of this plan. No task modifies, tests, merges, or deploys the `control-plane` repository, and no acceptance criterion depends on warehouse consumer changes. Warehouse compatibility work is planned and delivered separately.
- The producer contract this plan emits (metric names, dimensions, units, temporality, session identity, and the cost completeness attributes) is fixed by this plan so the separate warehouse work can rely on it.
- Implementing production export support (T05) does not mean releasing or deploying it. Production release coordination, including ordering against the separate warehouse migration, is handled separately after SCE Factory publishes the resulting branch.
- Plan execution requires no GitHub write access, external PR creation, production deployment, or human approval. Publishing the resulting branch is SCE Factory's responsibility, not a task in this plan.

Planning inputs (verified 2026-11-27):

- SCE `main` at `47e72246`. Planning branch and PR: `claude-sce-owned-telemetry`, PR #362.
- The warehouse contract below is informational. It was read from `control-plane` `main` at `d016aeba` (`@repo/warehouse`) and explains why the producer emits what it emits. Tasks do not read, change, or test `control-plane`.
- Warehouse metric contract (`packages/warehouse/src/session-metrics.ts` `METRIC_FACT_MAPPINGS`, provider `claude_code`, session prefix `cc_`):
  - `claude_code.token.usage` with `type` ∈ `input|output|cacheRead|cacheCreation`, plus `model` (read by `ai-usage-metrics.ts`).
  - `claude_code.cost.usage`, read by `session-metrics.ts`, `ai-usage-metrics.ts`, and `ai-economics.ts`.
  - `claude_code.active_time.total` with `type` ∈ `user|cli`.
  - `claude_code.lines_of_code.count`, `claude_code.commit.count`, and `claude_code.code_edit_tool.decision`.
  - Only `metric_type = 'sum'` with `delta` or `cumulative` temporality is aggregated.
- Metric point identity (`apps/event-dwh-transformer/src/otel-metrics-transform.ts`):
  - `series_id` hashes the workspace, resource attributes, scope name, version, and attributes, metric name, unit, type, temporality, and point attributes.
  - `point_id` hashes `series_id` with `start_time_unix_nano` and `time_unix_nano`.
  - `record_hash` additionally covers the value and the point JSON.
  - Readers keep a `point_id` only when `uniqExact(record_hash) = 1`. An identical resend therefore dedupes. The same `point_id` with a different value is dropped entirely. The same measurement with different timestamps or attributes is a different point and is summed twice.
  - Deterministic span IDs therefore give no metric idempotency.
- Agent Trace attribution (`cli/src/services/agent_trace.rs`; consumer side in `apps/event-dwh-transformer/src/trace-facts.ts` and `packages/warehouse/src/sql-fragments.ts`):
  - `committed_trace_observations` carries only commit-level `ai | mixed | non_exclusive | unknown` added and removed counts.
  - `committed_trace_session_links` carries `(observation, session)` links derived from conversation `related` session URLs.
  - Added-line ranges may lack a session (`session_related(None)`), and removed lines have no session ranges.
  - Existing facts therefore support session-to-commit links, but no session-exclusive line counts.
- Transformer and ingest behavior:
  - `session.id` comes from point or resource attributes; conflicting values make it empty.
  - The workspace comes from the collector-authenticated `sce.s3.workspace_id`.
  - `user.*` and `organization.id` are dropped at ingest.
  - The transformer ingests no OTel spans; spans reach Dash0 only.

## Accounting, delivery, and attribution rules

These rules hold across all tasks. Tasks reference them instead of restating them.

### Code attribution

- Agent Trace is authoritative for committed code attribution. Its behavior is unchanged by this plan.
- SCE emits no `claude_code.commit.count`, `claude_code.lines_of_code.count`, or `claude_code.code_edit_tool.decision` metric. Claude's native line count is an intermediate-edit measurement, not committed attribution, and existing Agent Trace facts cannot establish session-exclusive line counts truthfully.
- No task assigns commit-level AI lines to individual sessions, whether as a whole, proportionally, or equally.
- Informational: deriving session commit counts from Agent Trace session links, and keeping historical native line counts separate from committed attribution, belong to the separate warehouse migration.

### Cost completeness

- SCE keeps `claude_code.cost.usage` (unit `USD`, delta sum). Every SCE point carries `sce.cost.basis = list_price_estimate` and `sce.cost.coverage` ∈ `complete | incomplete`.
- A measurement is `incomplete` when either of these holds:
  - Its model is absent from the price table.
  - A token measurement needed for pricing is absent.
- An incomplete measurement emits a point with value `0` and `sce.cost.coverage = incomplete`. The `0` is a marker, never a measured cost. No partial priced value is emitted for that measurement.
- **Cache-creation policy:** when the 5m/1h breakdown is absent, all cache-creation tokens are priced at the 5-minute write rate, and coverage stays `complete`. This is a documented under-estimate when 1-hour caching was used.
- Within one completion, priced and incomplete points travel in the same metrics request, so they are delivered or lost together.
- One version-controlled Anthropic list-price table with an effective date lives in the SCE CLI. It is maintained through SCE releases. Until a new model is added, its measurements are `incomplete`.
- Informational, intended consumer interpretation (implemented by the separate warehouse migration, not this plan): a session with any cost point whose `sce.cost.coverage` is non-empty and not `complete` has `null` total cost and contributes nothing to cost sums; points without `sce.cost.coverage` (historical native) keep their existing interpretation.

### Accounting checkpoint and failure semantics

- **Session state:**
  - Per-session state lives at `<state_root>/sce/claude-telemetry/<session_id>`, with the directory `0700` and files `0600`.
  - It is guarded by a file lock with a bounded wait and written by atomic replace.
  - It holds the authorization fingerprint (see Consent), the main-transcript checkpoint, a checkpoint per open subagent transcript, the bounded set of call keys counted since baseline, the open turn, tool, and subagent span IDs, and covered active intervals.
- **Checkpoint:** a transcript offset that only advances to the end of the last complete, newline-terminated record. It is set together with the last counted record timestamp.
- **Baseline:** established only on `UserPromptSubmit` when no valid state exists. Its offset is the transcript's current end, and nothing before it is ever counted. `SubagentStart` establishes a subagent's baseline only when session state exists.
- **Completion:** on `Stop`, `StopFailure`, `SubagentStop`, `SessionEnd`, or the next `UserPromptSubmit` for an interrupted turn, the hook does the following under the lock:
  1. Reads records from the checkpoint to the current complete end, skipping counted call keys.
  2. Builds the measurement.
  3. Advances and persists the checkpoint.
  4. Releases the lock.
  5. Sends best effort, once.
- **Point identity:** each measurement's points use deterministic attributes. `start_time` is the previous checkpoint's record timestamp, or the baseline time. `time` is the last counted record's timestamp. An identical rebuild therefore yields identical `point_id` and `record_hash`. An empty window emits no metric points.

| Case | Behavior |
|---|---|
| Resume after state deletion, TTL expiry, or corruption | The next `UserPromptSubmit` baselines at the transcript end. Earlier history, including history copied into a forked transcript, is never counted. Usage between the lost state and that baseline is lost. |
| Duplicate `Stop` / `SubagentStop` | The window is empty and the turn is already closed, so no metric points and no second turn span are emitted. |
| Repeated `SessionEnd` | The first one accounts the final window and deletes state. Later ones find no state and do nothing; `SessionEnd` never baselines. |
| Interrupted turn | The next `UserPromptSubmit` closes the open turn up to the current end before opening the new one. |
| Concurrent hooks | Serialized by the lock. A lock timeout skips that event's telemetry entirely. |
| Subagent completing after its parent turn | It is accounted from its own transcript checkpoint keyed by `agent_id`. Its span keeps the recorded parent IDs. A subagent without an observed `SubagentStart` is not accounted. |
| Export succeeds and then the process crashes | The checkpoint was persisted first, so nothing is resent. |
| Export fails after the checkpoint is persisted | The measurement is lost. There is no retry. |
| Partially written transcript | A trailing partial record is not consumed. A call key counted in an earlier window is never recounted, even if later streamed entries for it arrive, which is an accepted undercount. |
| Corrupted, oversized, or unreadable state | It is deleted, nothing is exported for that event, and the session re-baselines at the next `UserPromptSubmit`. |

- **Accepted failure semantics:** delivery is at most once and best effort. A crash, lock timeout, or network failure can lose measurements. Duplicated or additive measurements are not acceptable. Missing telemetry is preferred to duplicated or incorrect telemetry. There is no persistent queue, outbox, retry worker, distributed lock, extra database, or exactly-once protocol.

### Consent and authorization

- **Gate:** every observed hook resolves the gate before reading transcripts or touching accounting state. The gate checks the effective access policy for the `CLAUDE_PROJECT_DIR` Git root, a stat of `auth.db`, repository identity via `repository_id_for`, and the repository-bound credential.
- **Gate failure:** on any failure, existing session state is deleted, nothing is recorded or retained, and nothing is sent.
- **Authorization fingerprint:** stored in state at baseline. It combines the canonical repository identity, the credential's non-secret binding identity, and the path, length, and mtime of each loaded access-policy config file. Any mismatch deletes state, and the session re-baselines at the next `UserPromptSubmit`. This catches consent toggled and restored between two hooks without adding a consent timestamp or schema.
- **Recheck:** the credential is re-resolved immediately before send. A failure drops the measurement, and the checkpoint stays advanced.
- Telemetry that becomes enabled starts from a fresh baseline. Previously unauthorized history is never recovered.
- Agent Trace behavior is independent of telemetry consent and unchanged.

### Native telemetry cutover

- Claude caches native exporter configuration for the life of its process, and caches helper headers between refreshes. Deleting `sce auth telemetry-headers` stops future token retrieval only. It does not revoke tokens Claude already holds.
- **Native-export guard:**
  - Claude hooks inherit the Claude process environment. On every hook, before the gate, SCE-owned telemetry stays inactive (no state, no export) when the hook environment shows a native Claude exporter targeting the SCE collector. That means `CLAUDE_CODE_ENABLE_TELEMETRY` is truthy and any recorded `OTEL_EXPORTER_OTLP_*ENDPOINT` key resolves to the SCE collector host, or `otelHeadersHelper` resolves to the SCE helper.
  - The SCE collector host is the pinned production host. Under the injected test endpoint used by tests and T04, the guard compares against that injected host instead, so native and SCE-owned export to the same destination can be exercised locally without contacting production.
  - This covers running pre-migration sessions, sessions spanning the migration, settings that setup could not remove (modified, malformed, symlinked, read-only, managed), and settings re-written by an older SCE CLI.
  - It is a detection, not a configuration mode. It ships and is retired with the legacy migration cleanup.
  - **Invariant:** native and SCE-owned telemetry are never simultaneously exported to the SCE destination for the same Claude process.
- Native telemetry pointed at any other destination is user-owned. It is preserved and does not suppress SCE-owned telemetry, because it cannot reach the SCE warehouse.
- **User cutover procedure** (documented in `context/cli/claude-telemetry.md`; executed by users only after the separately coordinated release):
  1. Users run `sce setup` or `sce doctor --fix`, which removes SCE-owned native settings and reports manual remediation where it cannot.
  2. Users restart Claude sessions. Any session still carrying SCE native exporter configuration stays native-only until its process exits. A resumed session in a new process baselines at the transcript end, so usage already exported natively is not re-counted.

## Acceptance criteria

`/validate` runs these checks; no task performs final validation. Every check runs inside this repository.

- [ ] AC1: Every exported span and metric point carries the Claude `session_id` as `session.id` / `gen_ai.conversation.id` and the canonical repository identity from the `CLAUDE_PROJECT_DIR` Git root. This holds for nested cwd, linked worktrees, resumed sessions, and subagents. No exported attribute derives from Git identity or OS account data.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry::attribution`.
- [ ] AC2: Token usage from main and subagent transcripts equals the Dash0-reference totals for every ported fixture, per model and per token type, including the cache-creation breakdown when present. Streamed duplicates, fallback iterations, and concurrent subagents are counted once. An absent measurement stays absent.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::usage`.
- [ ] AC3: Replaying each failure-semantics table case yields the stated outcome. Replayed or duplicated completion hooks yield either no new metric points or points with identical `point_id` and `record_hash`. Pre-baseline history is never counted.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::accounting` (receiver-asserted; computes `point_id` / `record_hash` with the transformer's formula ported as a test helper in this crate).
- [ ] AC4: Cost points follow the Cost completeness rules. Test cases cover multiple models, multiple turns, a resumed session, an unknown model only, mixed known and unknown models, missing token measurements, and a missing cache breakdown. Every complete price matches a hand-computed list-price value to the microdollar.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::pricing` and `... claude_telemetry::metrics`.
- [ ] AC5: Spans form the expected GenAI trees: a turn span; `chat` spans that alone carry tokens and have no fabricated duration; `execute_tool` spans with `Error` status on failure, MCP server name, and skill name; and `invoke_agent` subagent spans with correct parents. Metrics are `claude_code.token.usage`, `claude_code.cost.usage`, and `claude_code.active_time.total` (`type=cli` only) with the warehouse contract names, units, dimensions, and delta temporality. No line, commit, edit-decision, or `type=user` metric is emitted.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::spans` and `... claude_telemetry::metrics`.
- [ ] AC6: The receiver sees zero requests, and no state file exists afterwards, for each of these gate states: policy disabled, consent withdrawn, missing or expired credential, unenrolled repository, fingerprint change, and active native-export guard. After consent is re-enabled, only usage after the new baseline is exported. Exports go only to the injected test endpoint with a bearer header, redirects disabled, and ambient `OTEL_*` ignored for export. A sentinel scan finds no prompt, response, tool input or output, or path content in any export or state file. State files are `0600` in a `0700` directory.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::gating` and `... claude_telemetry::privacy`.
- [ ] AC7: Hook stdout, stderr, exit status, permission decisions, and Agent Trace persistence are byte-identical with telemetry enabled, disabled, and failing. Failure modes include a stalled collector, invalid transcripts, missing events, missing credentials, and corrupted or locked state. Each hook's telemetry work stays within fixed time and byte budgets.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_mutation_scope` and `... claude_telemetry::fail_open`.
- [ ] AC8: The parity record in `context/cli/claude-telemetry.md` lists, for each compared measurement over the T04 sessions, the agreement and every discrepancy with its cause and reconstructability. Token, cache, and model totals equal the transcript reference. Parity replay tests pass.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::parity`; inspection of the parity record.
- [ ] AC9: Setup and migration behave as follows:
  - `sce setup` installs no native telemetry or `otelHeadersHelper`, and removes SCE-owned native entries regardless of consent.
  - User-owned, modified, malformed, symlinked, and read-only settings stay byte-identical, with remediation reported.
  - `sce doctor` reports leftovers, and `sce doctor --fix` removes them.
  - `sce auth telemetry-headers` exits non-zero with empty stdout.
  - Generated Claude settings register `SubagentStart` on the existing hook command, and repeated setup is idempotent.
  - Guard tests cover running native sessions, sessions spanning the migration, unremovable settings, and older-CLI rewrites; each yields zero SCE-owned requests while native exporter configuration is present.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry_migration`, `... setup`, `... doctor`, `... auth_command`; `nix run .#pkl-check-generated`; `grep -rn "telemetry-headers\|otelHeadersHelper" cli/src` returns only migration-removal and guard matches.
- [ ] AC10: The native-export guard holds against a real running Claude process whose settings are migrated while it is still running. The guard record in `context/cli/claude-telemetry.md` shows, per scenario, the native and SCE-owned requests received by the local SCE-destination receiver, and proves both of the following:
  - For every Claude process, the SCE-destination receiver never receives both native and SCE-owned telemetry. While a process started with SCE native exporter configuration lives, including after its settings are migrated, the receiver sees zero SCE-owned requests for its session.
  - After that process exits and the session resumes in a new process, the receiver sees zero native requests, and SCE-owned usage starts at the transcript-end baseline, so usage already exported natively is not re-counted.
  - No run configures or contacts the production collector.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::native_guard` (replays the scrubbed real-process captures); inspection of the guard record.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`

### Context sync

- `context/cli/claude-telemetry.md` (new): module ownership, hook-to-span mapping, accounting rules and failure semantics, cost completeness and price-table policy, consent fingerprint, native-export guard, user cutover procedure, the scope boundary (warehouse migration and release coordination are separate), parity record, guard record, and Dash0 attribution.
- `context/sce/telemetry-credential.md`: remove the helper and native install sections; describe the hook export path.
- `context/sce/cli-observability-contract.md`: Claude telemetry now uses the metrics and traces endpoints; replace the native-versus-SCE section.
- `context/cli/access-policy.md`: remove helper references and the Claude-settings `telemetry.note` caveat.
- `context/sce/setup-repo-local-config-bootstrap.md` and `context/cli/cli-command-surface.md`: setup no longer installs native telemetry; migration removal.
- `context/sce/agent-trace-hook-doctor.md` and `context/sce/doctor-human-text-contract.md`: the legacy native settings row and fix.
- `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/context-map.md`.
- A new decision record superseding `context/decisions/2026-10-21-claude-telemetry-removal-on-global-withdrawal.md`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - `cli/src/services/hooks/claude_telemetry/` (new) and its fixtures.
  - `cli/src/services/hooks/{claude_transcript,claude_model_resolver}.rs` (reuse; extract shared bounded readers only where needed).
  - `cli/src/services/hooks/claude_mutation_scope/` (the observation seam).
  - `cli/src/services/managed_telemetry*`: a sender for prebuilt OTLP requests on the pinned client, and removal of the destination-conflict code.
  - `cli/src/services/auth_command/`, `cli/src/app.rs`, `cli/src/services/setup/`, `cli/src/services/doctor/`, `cli/src/services/config/render.rs`, and `cli/Cargo.toml`.
  - `config/pkl/renderers/claude-content.pkl` and regenerated outputs.
  - The `context/` files listed under Context sync.
- **Out of scope:**
  - Any change to, test of, merge into, or deployment of the `control-plane` repository, including warehouse, transformer, collector, schema-migration, and dashboard changes. If a producer requirement proves unsatisfiable without a consumer change, record it as a blocker instead of expanding scope.
  - Production release, deployment, and release coordination.
  - Codex, Copilot, Pi, and OpenCode telemetry.
  - Agent Trace generation, persistence, sync, and attribution.
  - Credential issuance, rotation, and enrollment.
  - The existing `sce.hook` / `sce.command` CLI spans.
  - Dash0-specific attributes, session titles, rate limits, and team names.
- **Constraints:**
  - One Rust executable. No Go dependency, separate service, daemon, background worker, event bus, persistent queue, outbox, telemetry database, new credential store, new consent system, or new config schema.
  - Static dispatch, concrete types, and explicit typed errors. No new `anyhow`, `dyn`, or unnecessary `Box`. No code comments except rustdoc.
  - Builds and tests go through Nix-wrapped Cargo only.
  - No automated test or validation run, including the real Claude runs in T04, contacts `otel.crocoderlab.dev` or uses production credentials. Senders take an injected endpoint in tests. The production endpoint stays pinned and cannot be redirected by environment or config.
  - Upstream fixtures keep Apache-2.0 attribution: SPDX headers, a copy of the upstream `LICENSE` beside the fixtures, and a rustdoc note naming the upstream path and pinned commit. Fixtures stay inside the crate `include` set (`/src/**`).
- **Non-goals:**
  - A cross-harness event model, a generic instrumentation framework, or a native-versus-SCE configuration mode.
  - A remote pricing API or pricing service.
  - Proportional or heuristic session line attribution.
  - Exactly-once delivery.
  - Exporting conversation content.
  - Speculative extension points.

## Assumptions

- Telemetry observation runs as a fail-open, post-decision step inside the existing `sce hooks claude-mutation-scope` command. That command already receives `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `StopFailure`, `SubagentStop`, and `SessionEnd`, and its parser already accepts `SubagentStart`. The only new registration is `SubagentStart`.
- Spans and metrics are built directly as `opentelemetry-proto` messages and posted with the existing pinned bearer client. `opentelemetry-proto` `=0.33.1` is already in `cli/Cargo.lock` and becomes a direct dependency with only the trace and metrics features.
- Repository identity is exported as `vcs.repository.url.full` from the existing credential-stripped canonical form.
- Billing mode is reported only from Dash0's environment-presence precedence, otherwise `unknown`. No secret values are read.
- Claude hook subprocesses inherit Claude's process environment, including the settings `env` block, and that environment does not change for the life of the process when settings are edited. T04 verifies both with real Claude. If either does not hold, AC10 fails, and T05 is marked `blocked` instead of weakening the guard invariant.
- The SCE Factory environment provides a `claude` executable and Claude model access for an isolated `CLAUDE_CONFIG_DIR`, as T04 requires. If it does not, T04 is marked `blocked` with that blocker.

## Accepted measurement differences

- Cost is a list-price estimate from the pinned table, not billed cost. Unsplit cache creation is priced at the 5m rate.
- Agent active time approximates the union of observed turn and subagent intervals.
- User active time and edit decisions are not emitted.
- Committed code attribution comes from Agent Trace. No session-specific line count is emitted for SCE-produced sessions.
- Measurements can be lost on a crash, lock timeout, network failure, state loss, or gate failure. They are never duplicated.
- Sessions running with SCE native exporter configuration produce no SCE-owned telemetry.

## Task stack

- [ ] T01: `Recover Claude usage and estimate cost` (status:todo)
  - Task ID: T01
  - Scope:
    - In:
      - `claude_telemetry::usage`: bounded reading of a main or subagent transcript from a byte offset, reusing the 1 MiB record cap and subagent path derivation. It ports Dash0's turn-usage rules:
        - Call-key precedence and last-streamed-entry-wins.
        - Fallback `usage.iterations` summing.
        - The 5m/1h breakdown and turn boundaries.
        - Skipping supplied counted keys.
        - Stopping at a syntax error and skipping on a type mismatch.
      - It returns per-model `Option` measurements, the slash-command skill name, the new complete-record offset and last record timestamp, and a truncation flag.
      - `claude_telemetry::pricing`: the pinned list-price table with effective date, model-id normalization, and per-model cost with `complete | incomplete` coverage per the Cost completeness rules. It also ports billing-mode detection.
      - Ported Dash0 fixtures with attribution. Upstream compatibility fixtures cover multi-call, streamed duplicates, resumed and forked transcripts, subagents, cache usage, a missing file, a partial last line, model changes, and unknown models.
    - Out: state, hooks, OTLP, export, and setup.
  - Dependencies: none
  - Done when:
    - Fixtures reproduce the Dash0 totals.
    - Every listed edge case has a passing test.
    - Missing or partial transcripts return typed unavailable or partial results without unbounded reads.
    - The AC4 pricing cases pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry::usage`; `... claude_telemetry::pricing`; `... claude_transcript`.
  - Context synchronization: pending

- [ ] T02: `Correlate Claude events and construct telemetry` (status:todo)
  - Task ID: T02
  - Scope:
    - In:
      - `claude_telemetry::state`, implementing exactly the Accounting checkpoint rules: lock, atomic replace, baseline, checkpoint, counted-key set, open turn, tool, and subagent entries, TTL sweep on `UserPromptSubmit`, and deletion on `SessionEnd`.
      - A pure `claude_telemetry::otlp` builder producing trace and metrics requests:
        - Spans and attributes per AC5, with deterministic IDs derived from session, turn, `tool_use_id`, and `agent_id`.
        - Metric timestamps from checkpoint record times.
        - One attribute allowlist.
      - A pure `observe` core that applies a parsed hook event plus transcript reads to state and returns the requests to send. No gate and no network.
      - Replay tests for every failure-semantics table case, using the transformer `point_id` / `record_hash` formula ported as a test helper.
    - Out: gating, transport, and hook wiring.
  - Dependencies: T01
  - Done when:
    - AC3 and AC5 pass.
    - Rebuilding the same window yields byte-identical requests.
    - Path-traversal session IDs are rejected.
    - File modes are asserted.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry::accounting`; `... claude_telemetry::spans`; `... claude_telemetry::metrics`; `... claude_telemetry::privacy`.
  - Context synchronization: pending

- [ ] T03: `Integrate authenticated export and hook observation` (status:todo)
  - Task ID: T03
  - Scope:
    - In:
      - A gate implementing the Consent and authorization rules and the native-export guard, reusing the access-policy resolver, `repository_id_for`, `authorized_repository_credential_with` within `CREDENTIAL_LOOKUP_DEADLINE`, and the authorization fingerprint.
      - A sender for prebuilt requests on the pinned bearer client, with redirects disabled, fixed timeouts, and a total budget.
      - Typed errors, logged at debug level through the existing file logger only.
      - A fail-open call from `claude-mutation-scope` after its decision and output are fixed. It is active only under an injected test endpoint and state root, so production export stays off.
      - Golden hook-output tests across telemetry enabled, disabled, and failing.
    - Out: Pkl registration, setup, doctor, helper removal, and production activation.
  - Dependencies: T02
  - Done when: AC1, AC6, and AC7 pass, and no test touches the production endpoint.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry::gating`; `... claude_telemetry::attribution`; `... claude_telemetry::fail_open`; `... claude_mutation_scope`.
  - Context synchronization: pending

- [ ] T04: `Validate real Claude telemetry parity` (status:todo)
  - Task ID: T04
  - Scope:
    - In:
      - Real Claude sessions in an isolated `CLAUDE_CONFIG_DIR`, with SCE telemetry under the injected test endpoint and state root. The production collector is never configured.
      - Parity runs: native telemetry goes to one local OTLP receiver and SCE telemetry to a separate local receiver. Scenarios: a simple prompt, multiple model calls, successful and failed tools, an MCP tool, a skill (tool and slash routes), multiple turns, a resumed session, parallel subagents, a model change, an interrupted turn, cached and uncached usage, and a linked worktree.
      - Comparison of session identity, attribution, token, cache, and model totals, cost semantics, active time, and tool and subagent counts, plus conformance of SCE points to the informational warehouse metric contract (names, units, dimensions, temporality) and identity checks with the ported `point_id` / `record_hash` helper. Transcripts are the independent token reference.
      - Native-export guard runs against a single local receiver standing in for the SCE destination, with Claude's SCE native exporter configuration (the `env` block and `otelHeadersHelper` as setup writes them) pointed at that receiver:
        1. Start a real Claude process with that configuration and complete a turn.
        2. While the process is still running, remove the SCE-owned native settings with the existing exact-value ownership removal path, then complete further turns, a tool call, and a subagent in the same process.
        3. Exit the process, resume the session in a new process, and complete further turns.
        - Record, per step, the hook environment observed by the guard and every request at the receiver classified as native or SCE-owned by instrumentation scope.
        - Assert AC10: no Claude process ever has both native and SCE-owned telemetry received at the SCE destination; steps 1 and 2 yield zero SCE-owned requests; step 3 yields zero native requests and no re-counted pre-resume usage.
      - Verification that hook subprocesses inherit Claude's settings `env` and that it stays unchanged after the in-process settings edit, which the guard assumes.
      - Fixes to T01–T03 for reconstructable discrepancies and guard failures that the existing guard design can fix.
      - Scrubbed captures as `claude_telemetry::parity` and `claude_telemetry::native_guard` replay fixtures.
      - A parity record that accepts the Accepted measurement differences and requires no parity for unobservable measurements, and a guard record, both in `context/cli/claude-telemetry.md`.
    - Out: production export, setup and doctor migration changes, and any warehouse consumer.
  - Dependencies: T03
  - Done when:
    - AC8 and AC10 pass.
    - The env-inheritance and env-stability results are recorded. If either fails, or AC10 cannot be met by the existing guard design, T05 is marked `blocked` with the failure as the blocker.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry::parity`; `... claude_telemetry::native_guard`; inspection of the parity and guard records.
  - Context synchronization: pending

- [ ] T05: `Retire native telemetry and complete cutover` (status:todo)
  - Task ID: T05
  - Scope:
    - In:
      - Setup:
        - Delete the native install path.
        - Run the exact-value ownership removal (anchor `otelHeadersHelper == "sce auth telemetry-headers"` plus the recorded entries) on every Claude-target setup regardless of consent.
        - Keep the symlink, modified, malformed, read-only, and managed-settings guardrails and report lines.
      - A doctor row for leftover SCE-owned native settings, and a `doctor --fix` action using the same removal.
      - Delete `AuthSubcommand::TelemetryHeaders` and its handler, the `managed_span_kind` branch, the `claude_helper.rs` destination-conflict code (relocating `repository_id_for`), the native merge and conflict code in `config_merge.rs` / `claude_telemetry.rs`, and the `telemetry.note` caveat. Keep only the removal fingerprints that migration and the guard need.
      - Add the `SubagentStart` registration in `claude-content.pkl` and regenerate.
      - Implement production export from the real hook. This is implementation only; release and deployment are coordinated separately after Factory publishes the branch.
      - Re-run the T04 native-export guard scenario with step 2 performed by `sce setup` and by `sce doctor --fix`, and append the results to the guard record.
      - Write the decision record superseding `2026-10-21-claude-telemetry-removal-on-global-withdrawal.md`, document the user cutover procedure and the scope boundary, and update all Context sync files.
      - Migration tests named `claude_telemetry_migration`, plus guard tests for running native sessions, spanning sessions, unremovable settings, and older-CLI rewrites.
    - Out: new commands or executables, `SessionStart` registration, Agent Trace or mutation-scope semantic changes, warehouse changes, and production release or deployment.
  - Dependencies: T04
  - Done when:
    - AC9 passes.
    - AC7 and AC10 still pass with production wiring.
    - Generated assets are in parity.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry_migration`; `... setup`; `... doctor`; `... auth_command`; `... claude_mutation_scope`; `... claude_telemetry::native_guard`; `nix run .#pkl-check-generated`; clippy with `-D warnings`.
  - Context synchronization: pending

## Open questions

None. SCE ownership of the price table is decided. The warehouse migration and production release coordination are outside this plan.
