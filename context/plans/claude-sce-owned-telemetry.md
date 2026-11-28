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

- **Evidence (reported by the blocked T04 execution, Claude 2.1.329):** the isolated real-process experiment observed `CLAUDE_CODE_ENABLE_TELEMETRY` from settings in hook subprocesses, but no `OTEL_*` variables from settings or the parent environment. The observed hook environment and parent PID stayed unchanged after editing settings, while native requests continued reaching the local receiver. Removing `otelHeadersHelper` from disk removes the remaining visible SCE ownership anchor, not the live exporter. The experiment used no production collector; its scratchpad is not committed evidence. T04 must reproduce and record these findings and the revised cutover results. Do not infer native destination or exporter state from a hook environment, a current settings file, or absence of the helper.
- **Supported cutover:** a stopped-process migration, not live migration. Close all Claude processes using the affected user-level settings profiles **before upgrading the CLI or changing settings/hooks**. The exporter is configured at process startup and cached headers may remain valid. Removing the helper does not revoke a token Claude already holds. There is no supported mid-process transfer of telemetry ownership.
- **Activation:** the regenerated Claude mutation-scope hook commands explicitly opt into the new observation path (an internal invocation-scoped environment marker, e.g. `SCE_CLAUDE_TELEMETRY_HOOK=1`, set on those commands by Pkl). Existing hook invocations without that marker still execute the original mutation-scope behavior and do not collect or export Claude telemetry, even with an upgraded CLI. Older binaries ignore the marker rather than failing to parse a new CLI flag. This is a generated-hook compatibility/activation signal, **not** a process-start signal, a consent mechanism, or a new user config schema. Test injection does not change this contract.
- **Legacy-settings preflight:** before the authorization gate and transcript reads, reuse the existing user and managed settings path resolution to perform a bounded, read-only check of the legacy installation surfaces. A visible SCE headers helper or a generic/per-signal OTLP endpoint targeting the SCE collector host blocks SCE-owned Claude telemetry, even with a different or absent helper. Under the injected test endpoint, use that host instead. Unavailable paths or unreadable, malformed, or oversized settings whose legacy status cannot be established also block; genuinely absent files are clean. Readable unrelated user-owned settings do not block. Do not change files, log raw values, inspect parent processes, or attempt to implement Claude's full effective-settings resolution. Use the same classifier for setup/doctor reporting, with remediation when blocked. This is a check for visible configuration leftovers, not detection of a running exporter.
- An unactivated or blocked invocation does not read transcripts, create accounting state, or export reconstructed telemetry; discard any existing session accounting state safely so reactivation starts at the next `UserPromptSubmit` baseline. Existing Agent Trace, mutation-scope decisions and CLI observability remain unchanged.
- **Supported invariant:** in the documented cutover sequence, native SCE-managed export stops before SCE-owned export begins. No pre-cutover transcript history is re-counted. This does not claim that arbitrary live edits, manually directing a native exporter at the SCE collector through other Claude configuration layers or ambient environment, or running an older CLI concurrently with migration can be detected from hooks. Those are outside the supported cutover; do not present a clean preflight as proof that no running exporter exists. If old setup recreates visible legacy settings, the preflight blocks future SCE-owned observation; perform the stopped-process procedure again before removing them. No PID registry, sticky marker, daemon, process launcher/wrapper, or runtime endpoint inference.
- Native telemetry to other destinations is user-owned and left unchanged. Under the supported cutover, it may coexist with SCE-owned export; neither a truthy `CLAUDE_CODE_ENABLE_TELEMETRY` nor a non-SCE helper alone suppresses SCE-owned telemetry. SCE consent remains governed by its existing access policy. A user-configured native exporter must not target the SCE collector through another settings layer or the launch environment when SCE-owned telemetry is active.
- **User cutover procedure** (documented in `context/cli/claude-telemetry.md`; executed by users only after the separately coordinated release):
  1. Exit all Claude processes that share the affected user settings (including IDE/headless/background sessions and each applicable `CLAUDE_CONFIG_DIR` profile). Do this before replacing the CLI on PATH: the current generated hook script resolves `sce` at each invocation. Do not resume until the following steps are complete.
  2. Upgrade SCE and stop using older CLI versions for setup. Run Claude-target `sce setup` (or the equivalent `sce doctor --fix` repair) for each affected profile and repository that should collect telemetry. Remove only exact-value SCE-owned native settings; repair reported leftovers manually at their source. Do not restart Claude until the legacy preflight passes and the new hooks, including `SubagentStart` and the activation marker, are installed. Cleanup failure must be reported as not ready, not as successful activation. Preserve unrelated user telemetry and never modify managed settings.
  3. Start or resume Claude using the cleaned profile and refreshed repository hooks. The first `UserPromptSubmit` establishes a transcript-end baseline. Existing native history is not counted. Fresh installations with no legacy settings follow the same setup/start sequence without needing a migration cleanup. Repeat the stopped-process sequence after any old-CLI rewrite or manual reintroduction of SCE native configuration.

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
- [ ] AC6: The receiver sees zero requests, and no state file exists afterwards, for each of these gate states: policy disabled, consent withdrawn, missing or expired credential, unenrolled repository, fingerprint change, and each inactive activation/preflight state (missing hook marker, visible legacy helper or endpoint, unreadable/invalid settings). After consent is re-enabled, only usage after the new baseline is exported. Exports go only to the injected test endpoint with a bearer header, redirects disabled, and ambient `OTEL_*` ignored for export. A sentinel scan finds no prompt, response, tool input or output, or path content in any export or state file. State files are `0600` in a `0700` directory.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::gating` and `... claude_telemetry::privacy`.
- [ ] AC7: Hook stdout, stderr, exit status, permission decisions, and Agent Trace persistence are byte-identical with telemetry enabled, disabled, and failing. Failure modes include a stalled collector, invalid transcripts, missing events, missing credentials, and corrupted or locked state. Each hook's telemetry work stays within fixed time and byte budgets.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_mutation_scope` and `... claude_telemetry::fail_open`.
- [ ] AC8: The parity record in `context/cli/claude-telemetry.md` lists, for each compared measurement over the T04 sessions, the agreement and every discrepancy with its cause and reconstructability. Token, cache, and model totals equal the transcript reference. Parity replay tests pass.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::parity`; inspection of the parity record.
- [ ] AC9: Setup and migration behave as follows:
  - `sce setup` installs no native telemetry or `otelHeadersHelper`, and removes SCE-owned native entries regardless of consent.
  - User-owned, modified, malformed, symlinked, and read-only settings stay byte-identical, with remediation reported when migration is blocked.
  - Setup/doctor share the legacy-settings preflight and report not ready for blocked profiles. `sce doctor --fix` removes only safely identifiable SCE-owned settings and reports manual remediation for anything it cannot remove. Non-Claude targets and user/managed settings that do not contain removable SCE entries retain their existing non-destructive behavior.
  - `sce auth telemetry-headers` exits non-zero with empty stdout.
  - Generated Claude settings register `SubagentStart` on the existing hook command, and repeated setup is idempotent.
  - Generated mutation-scope hook commands carry the internal activation marker; an upgraded CLI called by an old generated command still enforces the original mutation-scope behavior and sends no reconstructed Claude telemetry. Readiness tests cover clean/fresh profiles, unremovable or unreadable settings, SCE endpoints with a different or missing helper, and older-CLI rewrites. Visible legacy settings always prevent reconstructed export.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry_migration`, `... setup`, `... doctor`, `... auth_command`; `nix run .#pkl-check-generated`; `grep -rn 'telemetry-headers\|otelHeadersHelper' cli/src` returns only migration, preflight, and associated test matches.

- [ ] AC10: Real Claude runs demonstrate the supported stopped-process cutover. The cutover record in `context/cli/claude-telemetry.md` lists the native and SCE-owned requests received by local receivers and proves:
  - With legacy native settings and the old hook invocation, an upgraded CLI sends no reconstructed telemetry. With refreshed hooks but visible legacy settings, the preflight blocks reconstructed telemetry. Native export still reaches the local SCE-destination receiver.
  - Exit Claude before editing the settings or replacing its generated hooks. Perform legacy cleanup and hook refresh, then resume the same session in a new process. Native requests stop; SCE-owned requests begin only from the new transcript-end baseline and contain no pre-cutover usage.
  - Native user-owned telemetry to a separate local destination remains intact and does not suppress SCE-owned export with refreshed hooks and a clean legacy preflight. A truthy native enable flag alone is not a veto.
  - An older CLI recreating visible legacy settings, or a cleanup that leaves them in place, blocks reconstructed export and reports remediation. Repeat the stopped-process procedure before activation.
  - Reproduce the reported live-edit experiment as negative evidence of why live cutover is unsupported, with reconstructed export disabled during that experiment. Do not assert live migration safety or require inference of hidden native configuration.
  - No run configures or contacts the production collector.
  - Validate: `... test --manifest-path cli/Cargo.toml claude_telemetry::cutover` (scrubbed real-process captures and configuration/activation replay cases); inspection of the cutover record.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`

### Context sync

- `context/cli/claude-telemetry.md` (new): module ownership, hook-to-span mapping, accounting rules and failure semantics, cost completeness and price-table policy, consent fingerprint, generated-hook activation, legacy-settings preflight, stopped-process user cutover and its unsupported cases, the scope boundary (warehouse migration and release coordination are separate), parity record, cutover record, and Dash0 attribution.
- `context/sce/telemetry-credential.md`: remove the helper and native install sections; describe the hook export path.
- `context/sce/cli-observability-contract.md`: Claude telemetry now uses the metrics and traces endpoints; replace the native-versus-SCE section.
- `context/cli/access-policy.md`: remove helper references and the Claude-settings `telemetry.note` caveat.
- `context/sce/setup-repo-local-config-bootstrap.md` and `context/cli/cli-command-surface.md`: setup no longer installs native telemetry; migration removal.
- `context/sce/agent-trace-hook-doctor.md` and `context/sce/doctor-human-text-contract.md`: the legacy native settings readiness row, fix, and regenerated hook activation.
- `context/cli/claude-mutation-scope-integration.md` and `context/cli/config-precedence-contract.md`: generated-hook opt-in and unchanged mutation decisions; updated config-show telemetry guidance.
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
  - `cli/src/services/auth_command/`, `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/app.rs`, `cli/src/services/setup/`, `cli/src/services/doctor/`, `cli/src/services/config/render.rs`, and `cli/Cargo.toml`.
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

- Telemetry observation runs as a fail-open, post-decision step inside the existing `sce hooks claude-mutation-scope` command. That command already receives `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `StopFailure`, `SubagentStop`, and `SessionEnd`, and its parser already accepts `SubagentStart`. The only new registration is `SubagentStart`; existing mutation-scope hook commands gain the invocation-scoped activation marker. Existing `PermissionDenied` and `WorktreeRemove` registrations retain their mutation-scope behavior and do not add new telemetry accounting events.
- Spans and metrics are built directly as `opentelemetry-proto` messages and posted with the existing pinned bearer client. `opentelemetry-proto` `=0.33.1` is already in `cli/Cargo.lock` and becomes a direct dependency with only the trace and metrics features.
- Repository identity is exported as `vcs.repository.url.full` from the existing credential-stripped canonical form.
- Billing mode is reported only from Dash0's environment-presence precedence, otherwise `unknown`. No secret values are read.
- Do not assume hooks inherit native exporter configuration or that editing a settings file changes a running exporter. T04 records the negative experiment and verifies the stopped-process contract with real Claude. The explicit generated-command marker must reach the hook in direct Claude launches; T04 exercises the equivalent command without requiring a launcher or process tracking.
- The SCE Factory environment provides a `claude` executable and Claude model access for an isolated `CLAUDE_CONFIG_DIR`, as T04 requires. If it does not, T04 is marked `blocked` with that blocker.

## Accepted measurement differences

- Cost is a list-price estimate from the pinned table, not billed cost. Unsplit cache creation is priced at the 5m rate.
- Agent active time approximates the union of observed turn and subagent intervals.
- User active time and edit decisions are not emitted.
- Committed code attribution comes from Agent Trace. No session-specific line count is emitted for SCE-produced sessions.
- Measurements can be lost on a crash, lock timeout, network failure, state loss, or gate failure. They are never duplicated.
- Old hook invocations and visible legacy settings block reconstructed telemetry. Missing telemetry until users complete migration is accepted; live migration or native export to the SCE destination through unobservable configuration is not supported.

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
      - A gate implementing the Consent and authorization rules, generated-hook activation, and bounded legacy-settings preflight, reusing the access-policy resolver, `repository_id_for`, `authorized_repository_credential_with` within `CREDENTIAL_LOOKUP_DEADLINE`, and the authorization fingerprint.
      - A sender for prebuilt requests on the pinned bearer client, with redirects disabled, fixed timeouts, and a total budget.
      - Typed errors, logged at debug level through the existing file logger only.
      - A fail-open call from `claude-mutation-scope` after its decision and output are fixed. It requires the invocation-scoped marker and is active only under an injected test endpoint and state root, so production export stays off. Existing unmarked commands retain their prior output and behavior.
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
      - Stopped-process cutover runs against a local receiver standing in for the SCE destination:
        1. Start real Claude with the legacy SCE native settings and an old, unmarked hook command invoking the new test CLI. Complete a turn; verify native requests and zero reconstructed requests. Separately exercise the marked hook with visible legacy settings and verify it also sends zero reconstructed requests.
        2. Exit Claude. Remove exact-value SCE-owned settings using the existing removal path, install the equivalent marked test hooks, and resume the same session in a new process. Complete further turns, a tool call, and a subagent. Verify that native export has stopped and SCE-owned usage starts only at the new baseline.
        3. Exercise fresh setup, unrelated user-native configuration to a distinct local receiver (also used in parity scenarios), and visible leftovers or old-CLI rewrites. Assert AC10. Do not attempt or assert a live ownership transfer.
      - Reproduce and record the earlier real-process negative experiment: capture the enable flag and absence of native `OTEL_*` variables, edit settings while Claude runs, and observe native requests continuing. Keep SCE-owned observation disabled for this negative experiment. Record version and observations, not secret values.
      - Fixes to T01–T03 for reconstructable discrepancies and failures of the revised supported cutover contract.
      - Scrubbed captures as `claude_telemetry::parity` and `claude_telemetry::cutover` replay fixtures.
      - A parity record that accepts the Accepted measurement differences and requires no parity for unobservable measurements, and a cutover record (including the negative experiment), both in `context/cli/claude-telemetry.md`.

    - Out: production export, setup and doctor migration changes, and any warehouse consumer.
  - Dependencies: T03
  - Done when:
    - AC8 and AC10 pass.
    - The negative live-edit evidence and successful stopped-process scenarios are recorded. If AC8 or revised AC10 cannot be met, T04 remains incomplete and T05 is marked `blocked` with the evidence, required action, and retry condition. Absence of native `OTEL_*` variables is expected and is not itself a blocker.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry::parity`; `... claude_telemetry::cutover`; inspection of the parity and cutover records.
  - Context synchronization: pending

- [ ] T05: `Retire native telemetry and complete cutover` (status:todo)
  - Task ID: T05
  - Scope:
    - In:
      - Setup:
        - Delete the native install path.
        - Run the exact-value ownership removal (anchor `otelHeadersHelper == "sce auth telemetry-headers"` plus the recorded entries) on every Claude-target setup regardless of consent.
        - Keep the symlink, modified, malformed, read-only, and managed-settings guardrails and report lines.
      - A doctor readiness row using the same preflight as the runtime gate, and a `doctor --fix` action using the same exact-value removal. Ensure setup/doctor report restart prerequisites and not-ready outcomes; a hook refresh or CLI upgrade alone does not establish a safe migration.
      - Delete `AuthSubcommand::TelemetryHeaders` from schema, parser conversion and service dispatch, its handler, only the helper-specific `managed_span_kind` bypass (preserve existing `sce.hook` / `sce.command` spans), the `claude_helper.rs` destination-conflict code (relocating `repository_id_for`), the native install/merge code in `config_merge.rs` / `claude_telemetry.rs`, and the legacy `telemetry.note` caveat (preserve general opt-in guidance). Retain the exact-value removal fingerprints and conflict logic required for safe removal, plus the shared read-only preflight.
      - Add the `SubagentStart` registration and the internal invocation marker on all generated mutation-scope observation commands in `claude-content.pkl`, and regenerate. Keep old unmarked commands valid and the mutation-scope behavior unchanged.
      - Implement production export from the real hook. This is implementation only; release and deployment are coordinated separately after Factory publishes the branch.
      - Re-run the T04 stopped-process cutover with step 2 performed by `sce setup` and by `sce doctor --fix`, and append the results to the cutover record. Verify upgrade-only old hooks, fresh setup, failed cleanup, and old-CLI rewrite behavior using local receivers. Test actual regenerated hook commands, including the marker reaching direct Claude launches.
      - Write the decision record superseding `2026-10-21-claude-telemetry-removal-on-global-withdrawal.md`, document the user cutover procedure and the scope boundary, and update all Context sync files.
      - Migration tests named `claude_telemetry_migration`, plus cutover tests for old unmarked hooks, clean/fresh profiles, preserved user-owned settings, unremovable settings, older-CLI rewrites, and stopped-process resume without re-counting.
    - Out: new commands or executables, `SessionStart` registration, Agent Trace or mutation-scope semantic changes, warehouse changes, and production release or deployment.
  - Dependencies: T04
  - Done when:
    - AC9 passes.
    - AC7 and AC10 still pass with production wiring.
    - Generated assets are in parity.
  - Verify: `... test --manifest-path cli/Cargo.toml claude_telemetry_migration`; `... setup`; `... doctor`; `... auth_command`; `... claude_mutation_scope`; `... claude_telemetry::cutover`; `nix run .#pkl-check-generated`; clippy with `-D warnings`.
  - Context synchronization: pending

## Open questions

None. SCE ownership of the price table is decided. The supported migration requires stopping Claude before upgrading or changing its configuration; live migration is explicitly out of scope. The warehouse migration and production release coordination are outside this plan.
