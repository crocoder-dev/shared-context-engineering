# Plan: mutation-scope-hook-ingress

## Change summary

The mutation-scope runtime (`cli/src/services/mutation_trace/runtime/`) is fully
built: `coordinate()` drives observed `Start`/`Advance`/`Close`/`Flush`
boundaries against a real worktree behind the shared `ProtectedWorktree` safety
prefix, and `abandon_scope()` retires a scope whose final boundary was never
observed. Both are `pub(crate)` re-exported from `runtime/mod.rs`, and the
harness-adapter contract is recorded in `context/cli/mutation-scope-runtime.md`.
Before this plan, nothing called either entrypoint — no hook, plugin, extension,
or command.

This plan adds one harness-neutral CLI ingress, `sce hooks mutation-scope`, that
reads a single normalized JSON object from STDIN, strictly parses and validates
it, translates it directly into a `RuntimeBoundary` (for `start`/`advance`/
`close`/`flush`) or an `abandon_scope()` call (for `abandon`), and invokes the
existing runtime with a **lazy** DB provider so DB acquisition stays inside the
runtime's protected-worktree ordering. It is the generic transport/normalization
seam every future Claude Code, Codex, OpenCode, and Pi adapter will target; it
contains no concrete harness mapping and no lifecycle-event translation.

State now (after T02): `sce hooks mutation-scope` exists and drives
`coordinate()` / `abandon_scope()`. Concrete Claude Code, Codex, OpenCode, and
Pi lifecycle adapters remain out of scope for this plan (T04 records that
boundary in durable context).

The ingress owns nothing durable. The external adapter owns `scope_id`,
`event_id`, and `actor_kind`; SCE owns `worktree_id`, Git tree identities,
mutation revisions, and attempt IDs. The payload never accepts `worktree_id` —
worktree identity is derived by the runtime from the invoking checkout.
`scope_id` and `event_id` are translated verbatim into `ScopeId(..)` /
`EventId(..)` with no prefixing, hashing, normalization, UUID generation, or
timestamping, because `EventId` equality is the existing replay/idempotency key.

Unlike `diff-trace` / `conversation-trace`, a lost mutation-scope lifecycle
boundary can change which scope stays live and therefore alter attribution, so a
valid boundary must never be silently discarded. Two failure classes are
distinguished, matching what the runtime already models:

- An ordinary runtime error *before* durable completion
  (`CoordinateError` / `AbandonScopeError` variants other than the two below) is
  a command failure with non-zero exit.
- `CoordinateError::MarkerClearAfterCommit { committed, source }` and
  `AbandonScopeError::MarkerClearAfterCompletion { completed, source }` mean the
  durable mutation operation **already succeeded** and only the trailing
  external-taint marker cleanup failed. The ingress treats these as durable
  success: it reports the cleanup failure diagnostically, emits empty stdout, and
  exits zero. It does **not** re-run or retry the mutation transition. The marker
  stays armed, so the next runtime invocation recovers conservatively per
  existing runtime semantics.

Successful operations produce empty stdout; the ingress serializes no outcome,
revision, worktree ID, or scope state.

This extends existing behavior and disturbs none of it: no change to
`spec/mutation_cursor.qnt`, `protocol.rs`, the mutation-trace SQL schema,
migrations, `diff_traces`, `post_commit_patch_intersections`, `agent_traces`, or
#259 attribution behavior. The pure mutation-domain types gain no serde derives
for this command.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `sce hooks mutation-scope` exists and routes through the normal
  CLI/hook command stack (`cli_schema::HooksSubcommand::MutationScope` →
  `convert_hooks_subcommand_request` → `services::hooks::HookSubcommand::MutationScope`
  → `run_hooks_subcommand_in_repo`), hidden with the rest of the `hooks` surface.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::parse::command_runtime` plus `services::hooks::` — a parser test asserts `sce hooks mutation-scope` converts to `HookSubcommand::MutationScope`; `sce hooks --help` does not appear in top-level `sce --help`.
- [ ] AC2: The normalized JSON contract strictly supports exactly `start`,
  `advance`, `close`, `flush`, `abandon` with exact field validation, rejecting
  unknown operation, unknown `actor_kind`, missing/empty/blank `scope_id` or
  `event_id`, unexpected fields, any `worktree_id` key, wrong JSON type, and
  malformed JSON. `flush` accepts no scope/event/actor fields; `abandon` accepts
  only `scope_id`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope` — focused parser tests cover every operation and every listed rejection case.
- [ ] AC3: `start` / `advance` / `close` map exactly to `RuntimeBoundary::Start`
  / `Advance` / `Close` and forward `ScopeId`, `EventId`, and `ActorKind`
  unchanged (`claude_code → ClaudeCode`, `codex → Codex`, `opencode → OpenCode`,
  `pi → Pi`).
  - Validate: integration test T03-Test1 asserts durable processed-event keys `(A,e1)`,`(A,e2)`,`(A,e3)` exactly as supplied; T03-Test3 asserts a mismatched `actor_kind` reaches `ScopeIdentityConflict`.
- [ ] AC4: `flush` maps to `RuntimeBoundary::Flush` with no scope/event/actor
  identity, and drives the runtime's real observed-flush behavior. Against a
  baseline tree followed by an unscoped filesystem edit, a `{"operation":"flush"}`
  advances `cursor_tree` to the edited Git tree, advances `revision` by one,
  writes exactly one `mutation_trace_events` row for the tree transition with
  `attribution = IneligibleUnscoped`, invents no `mutation_trace_scopes` row, and
  invents no `mutation_trace_processed_events` row.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope` — integration test T03-Test5 asserts each durable value against real Git trees; a `Flush => Ok("")` stub fails it.
- [ ] AC5: `abandon` calls `abandon_scope()` directly and never acquires
  `Close`/`Flush` snapshot semantics. In `start(A,e1) → record cursor_tree →
  unobserved filesystem edit → abandon(A)`, `abandon_scope()` captures no Git
  snapshot and no mutation boundary, and `cursor_tree` stays the pre-edit tree —
  it does **not** become the edited Git tree — with no `mutation_trace_events`
  row for the edit.
  - Validate: `rg -n 'RuntimeBoundary|GitSnapshotService|capture_tree|pin_tree|diff_trees|coordinate\(' cli/src/services/hooks/mutation_scope.rs` shows the abandon arm calls only `abandon_scope`; integration test T03-Test4 asserts the `cursor_tree` invariance and absent rows.
- [ ] AC6: The normalized payload contains no `worktree_id` field. The
  production ingress does not accept, derive from input, or construct a
  `WorktreeId`; worktree identity remains exclusively derived by the existing
  mutation runtime from the invoking checkout. Test-only (`#[cfg(test)]`) code
  may construct `WorktreeId` values purely to fabricate injected
  `CoordinateOutcome` / `AbandonScopeOutcome` runtime results.
  - Validate — wire contract: a parser regression rejects any payload containing
    a `worktree_id` key (the production rejection diagnostic
    `field 'worktree_id' is not accepted` is expected to contain that literal, so
    do not text-ban the string outright).
  - Validate — production code: inspecting only the module body *before*
    `#[cfg(test)]`, it never constructs `WorktreeId(...)`, never reads a
    worktree-identity field from the JSON payload, and never passes a
    `WorktreeId` into `coordinate()` / `abandon_scope()`. Confirm by reading the
    production section, or with a check scoped to it — e.g. `rg -n 'WorktreeId'
    cli/src/services/hooks/mutation_scope.rs` shows matches only within the
    `#[cfg(test)]` module.
- [ ] AC7: DB acquisition stays lazy inside the runtime's protected-worktree
  sequence — the ingress passes a `FnOnce` provider closure to `coordinate()` and
  `abandon_scope()`, never an already-open handle, reusing
  `open_agent_trace_db_for_hook_runtime`.
  - Validate: `rg -n 'open_agent_trace_db_for_hook_runtime|open_db|coordinate\(|abandon_scope\(' cli/src/services/hooks/mutation_scope.rs` shows the DB resolver is invoked only inside the provider closure passed to the runtime entrypoint.
- [ ] AC8: Runtime results are classified by durable completion, not fail-open:
  - a pre-completion `CoordinateError` / `AbandonScopeError` (any variant other
    than the two carried-outcome variants) → `CliError` / non-zero exit;
  - `CoordinateError::MarkerClearAfterCommit { committed, .. }` and
    `AbandonScopeError::MarkerClearAfterCompletion { completed, .. }` → durable
    success: the carried outcome is treated as the result, the marker-cleanup
    failure is logged diagnostically, stdout is empty, exit is zero, and the
    runtime transition is **not** executed or retried again;
  - no `"failed open" / exit 0` branch exists for a dropped or malformed boundary.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope` — tests assert (a) malformed JSON and an injected ordinary runtime error return `Err`; (b) an injected `MarkerClearAfterCommit` and an injected `MarkerClearAfterCompletion` each return `Ok` with empty stdout and the runtime entrypoint is invoked exactly once (no second transition); `rg -n 'fail.open|failed open|exit 0' cli/src/services/hooks/mutation_scope.rs` returns nothing.
- [ ] AC9: A successful mutation-scope hook execution produces empty stdout, with
  no serialized `CoordinateOutcome`, `AbandonScopeOutcome`, `MutationEvent`,
  revision, worktree ID, or scope state.
  - Validate: integration tests assert the returned success string is empty (zero stdout bytes).
- [ ] AC10: A real Git/DB `Start → edit → Advance → Close` flow through the
  ingress creates scope status `Closed`, processed events `(A,e1)`,`(A,e2)`,`(A,e3)`,
  exactly one mutation event over the edit interval with `AiExclusive(A)`, and a
  cursor tree equal to the final observed Git tree.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope` — integration test T03-Test1 asserts each durable value against real Git trees, with no manually inserted mutation event.
- [ ] AC11: A replayed `(ScopeId, EventId)` boundary through the ingress is
  fully idempotent. In `start(A,e1) → edit → advance(A,e2)`, with `revision`,
  `mutation_trace_events` count, and `mutation_trace_processed_events` count
  snapshotted immediately after the first `advance(A,e2)`, a second
  `advance(A,e2)` leaves all three counts unchanged and `mutation_trace_processed_events`
  holds exactly one key `(A,e2)`. The test fails if the ingress regenerates,
  prefixes, hashes, or otherwise transforms `EventId`.
  - Validate: integration test T03-Test2 snapshots the three values before replay and asserts equality plus the single `(A,e2)` processed key via direct DB reads.
- [ ] AC12: A conflicting `ActorKind` for an existing scope commits no second
  boundary. In `start(A,e1,claude_code)` (recording `revision`, processed-event
  rows, scope actor/status) → `advance(A,e2,codex)`, the runtime returns
  `ScopeIdentityConflict` and afterwards: `revision` is unchanged, `(A,e2)` is
  absent from `mutation_trace_processed_events`, scope `A` still has
  `actor_kind = claude_code` and an unchanged status, and no new
  `mutation_trace_events` row exists (the assertion holds even when the rejected
  boundary observed no tree change).
  - Validate: integration test T03-Test3 asserts each value.
- [ ] AC13: Abandonment through the ingress retains no-snapshot semantics and
  the correct durable transition. In `start(A,e1) → record cursor_tree →
  unobserved edit → abandon(A)`: scope `A` is `Abandoned`, `revision` advances
  exactly once for the abandonment, `needs_rebaseline` is `true`, `cursor_tree`
  remains the pre-edit tree (never the edited Git tree), and no
  `mutation_trace_events` row for the edit and no `mutation_trace_processed_events`
  row from abandon exist.
  - Validate: integration test T03-Test4 asserts each durable value after the sequence, exercising the real `abandon_scope()` path.
- [ ] AC14: The ingress reaches durable storage only through the existing
  mutation runtime, writing `mutation_trace_*` rows only. No production ingress
  path writes `diff_traces`, `post_commit_patch_intersections`, or `agent_traces`.
  The T03 integration tests are expected to read/assert those three table names in
  order to prove they stay empty; the ban is on the production write path, not on
  table names appearing in the source file.
  - Validate: the T03 regressions assert `diff_traces`, `post_commit_patch_intersections`, and `agent_traces` each hold zero rows after every ingress flow; and the production implementation (excluding the `#[cfg(test)]` section) calls none of `insert_diff_trace`, `DiffTraceInsert`, `insert_post_commit_patch_intersection`, `PostCommitPatchIntersectionInsert`, `insert_agent_trace`, or `AgentTraceInsert` — confirm by inspecting the non-test module body, or `rg -n 'insert_diff_trace|DiffTraceInsert|insert_post_commit_patch_intersection|PostCommitPatchIntersectionInsert|insert_agent_trace|AgentTraceInsert' cli/src/services/hooks/mutation_scope.rs` shows hits only within the `#[cfg(test)]` test module, if any.
- [ ] AC15: No change attributable to this plan exists in
  `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/protocol.rs`,
  `cli/migrations/agent-trace-repository/`, or the Agent Trace schema.
  - Validate: `git diff origin/mutation-trace-agent-attribution -- spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs cli/migrations/agent-trace-repository/ config/schema/agent-trace.schema.json` is empty.
- [ ] AC16: Durable context establishes this as the generic adapter seam and
  states that concrete harness lifecycle integration remains future work.
  - Validate: `context/cli/mutation-scope-hook-ingress.md` exists and documents the JSON contract, operation mapping, identity ownership, the no-`worktree_id` rule, the no-`ScopeId`/`EventId`-generation rule, the non-fail-open error semantics including the marker-clear-after-durable-completion classification, the lazy DB-provider requirement, the empty-stdout contract, abandonment ownership, and the generic-ingress vs harness-adapter boundary; `context/cli/mutation-scope-runtime.md` Status says a generic ingress now drives the runtime with no concrete harness adapter wired.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#pkl-check-generated`
- `nix flake check`
- Confirm the branch diff is against `origin/mutation-trace-agent-attribution`, not `main`.

### Context sync

T02's task synchronization already removed the now-false "the runtime is not
wired into any hook or command" / "nothing calls either entrypoint" clauses from
`context/overview.md`, `context/context-map.md`,
`context/cli/mutation-scope-runtime.md`,
`context/cli/mutation-trace-runtime-coordinator.md`,
`context/cli/mutation-trace-scope-abandonment.md`, and
`context/cli/mutation-trace-protocol.md`, and added the command to the
implemented surface list in
`context/sce/agent-trace-hooks-command-routing.md`. The remaining work below is
T04's comprehensive pass — chiefly the new domain file and the full non-fail-open
intake contract.

- New: `context/cli/mutation-scope-hook-ingress.md` — the normalized JSON
  contract, operation mapping, identity ownership, why `worktree_id` is refused,
  why `ScopeId`/`EventId` are not generated, why errors are not fail-open, the
  marker-clear-after-durable-completion classification, the lazy DB-provider
  requirement, the silent-success stdout contract, abandonment ownership, and the
  generic-ingress vs harness-adapter boundary.
- `context/cli/mutation-scope-runtime.md` — Status section: a generic
  mutation-scope hook ingress now drives the runtime; no concrete Claude Code,
  Codex, OpenCode, or Pi lifecycle adapter is wired yet.
- `context/sce/agent-trace-hooks-command-routing.md` — add `sce hooks
  mutation-scope` to the implemented command surface and describe its
  non-fail-open intake contract (including the two carried-outcome success
  variants) distinct from `diff-trace`/`conversation-trace`.
- `context/context-map.md` — register the new domain file and update the
  hooks-routing / mutation-scope-runtime annotations.
- `context/overview.md` — finalize the `mutation_trace` paragraph (T02 already
  replaced the "still not wired into any hook or command" clause with the
  generic-ingress description; T04 confirms it against the shipped contract).

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/mutation_scope.rs` (new),
  `cli/src/services/hooks/mod.rs` (module registration + dispatch +
  `hook_runtime_invocation_name`), `cli/src/cli_schema.rs`
  (`HooksSubcommand::MutationScope`), `cli/src/services/parse/command_runtime.rs`
  (`convert_hooks_subcommand_request` + tests), the five durable-context files in
  **Context sync**.
- **Out of scope:** any concrete harness mapping (Claude Code hooks, Codex hook
  mapping, OpenCode plugin, Pi extension); `SubagentStart`/`SubagentStop`/
  `PostToolUse`/tool-call translation; `session → ScopeId` or `tool-call →
  EventId` derivation; PID tracking, process supervisors, staleness detection,
  automatic scope abandonment; harness settings generation or `sce setup`
  integration for the new hook; new mutation protocol semantics, Quint actions,
  DB tables, migrations, retention, or GC; any Agent Trace attribution,
  post-commit, diff-trace, or conversation-trace behavior change; #259
  attribution behavior. Changing the underlying runtime error types.
- **Constraints:** stacked on `mutation-trace-agent-attribution`; confirm all
  diffs against `origin/mutation-trace-agent-attribution`. Reuse
  `open_agent_trace_db_for_hook_runtime` as the sole DB resolver — no second
  DB-opening implementation. Keep hook transport types local to
  `mutation_scope.rs`. Do not add serde derives to `mutation_trace` domain types.
  Preserve the runtime safety-prefix ordering: the DB provider closure must be
  the value passed to `coordinate()` / `abandon_scope()`, never invoked before
  them. Follow the repo's inline `#[cfg(test)] mod tests` + RAII
  `tempfile::TempDir` + real `git init` / `RepositoryAgentTraceDb` pattern
  (`context/patterns.md`).
- **Non-goal:** turning `abandon` into a `RuntimeBoundary` variant, or capturing
  a Git snapshot on behalf of abandonment. The ingress does not interpret valid
  runtime outcomes (`accepted = false`, `observes = false`, duplicate processed
  event, no tree change, `Abandoned` / `AlreadyTerminal` / `RecoveryRequired`) —
  those stay existing runtime semantics — beyond the marker-clear-after-durable-
  completion classification required by AC8. There is never a completed task
  state in which `sce hooks mutation-scope` accepts a valid lifecycle boundary
  but does not drive the runtime.

## Assumptions

- The user's cover note allows ordinary local shape choices ("the exact Rust
  shape may differ if repository conventions suggest something cleaner").
- serde's `deny_unknown_fields` is not honored on the variant structs of an
  internally tagged (`tag = "operation"`) enum. The parser therefore validates
  "unexpected fields" / `worktree_id` rejection explicitly — e.g. deserialize the
  tag first, then deserialize the remainder into a per-operation struct that does
  carry `#[serde(deny_unknown_fields)]`, or match on a `serde_json::Map` and
  reject unknown keys. This is a local implementation choice recorded here rather
  than asked, per the note above.
- The command stays hidden because `HOOKS_SHOW_IN_TOP_LEVEL_HELP` is already
  `false`; no new visibility flag is needed.
- The ingress reads the invoking checkout via the same `repository_root`
  (`std::env::current_dir()`) that `run_hooks_subcommand` already resolves for
  every hook; the runtime derives `git_dir` and `WorktreeId` from it.
- `CoordinateError::MarkerClearAfterCommit { source, committed: Box<CoordinateOutcome> }`
  and `AbandonScopeError::MarkerClearAfterCompletion { source, completed: Box<AbandonScopeOutcome> }`
  are the exact carried-outcome variants on the current base
  (`origin/mutation-trace-agent-attribution`), confirmed by inspection of
  `runtime/coordinator.rs` and `runtime/scope_runtime.rs`.
- Flush against an unscoped edit on a healthy, non-rebaseline worktree yields a
  single `mutation_trace_events` row with `attribution = IneligibleUnscoped` and
  no processed-event row (`is_hook(Flush) == false`), confirmed by inspection of
  `protocol.rs` (`evaluate`/`apply`) and `types.rs` (`is_hook`,
  `boundary_event_key`).

## Task stack

- [x] T01: `Add strict normalized mutation-scope payload parser` (status:done)
  - Task ID: T01
  - Scope: In — new `cli/src/services/hooks/mutation_scope.rs` with the local
    `MutationScopePayload` transport enum (`Start`/`Advance`/`Close`/`Flush`/
    `Abandon`), the ingress-local actor-kind parser mapping the four wire strings
    (`claude_code`/`codex`/`opencode`/`pi`) to `ActorKind`, a
    `parse_mutation_scope_payload(&str) -> Result<MutationScopePayload, _>`
    function with strict wire-format validation (empty/blank string rejection,
    unknown operation, unknown actor kind, missing fields, empty/blank
    `scope_id`/`event_id`, unexpected fields, any `worktree_id` key, wrong JSON
    type, malformed JSON, `flush` rejecting any scope/event/actor field,
    `abandon` accepting only `scope_id`), the `pub mod mutation_scope;`
    registration in `hooks/mod.rs` (under the existing `#[allow(dead_code)]`
    module policy if needed), and focused `#[cfg(test)] mod tests` covering every
    operation and every rejection case. Out — CLI routing, runtime invocation,
    STDIN reading, context.
  - Dependencies: none
  - Done when: the module compiles, `parse_mutation_scope_payload` accepts each
    valid operation shape and rejects each listed invalid case, and the parser
    tests pass; `cargo clippy --all-targets -- -D warnings` and `fmt --check` are
    clean for the new file.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/services/hooks/mutation_scope.rs` (new) — `MutationScopePayload`
      transport enum (`Start`/`Advance`/`Close`/`Flush`/`Abandon`), private
      `parse_actor_kind` wire-string mapper, `parse_mutation_scope_payload`
      strict parser, and `#[cfg(test)] mod tests` (20 tests).
    - `cli/src/services/hooks/mod.rs` — `#[allow(dead_code)] pub mod mutation_scope;`
      registration only.
  - Result: Added the ingress-local wire-format parser. `MutationScopePayload`
    carries `scope_id`/`event_id` as raw owned strings (no trim/prefix/hash) and
    `actor_kind` as the real `ActorKind`. `parse_mutation_scope_payload` guards
    empty/blank input, requires a JSON object with a required `operation` string,
    and dispatches to a shared scope-boundary parser (`start`/`advance`/`close`),
    a flush parser, or an abandon parser. Each enforces its exact allowed key set
    via `reject_unexpected_keys`, which emits a dedicated diagnostic for any
    `worktree_id` key. Rejected: malformed JSON, non-object JSON, wrong field
    types, unknown operation, unknown `actor_kind` (`claude_code`/`codex`/
    `opencode`/`pi` only), missing/blank `scope_id`/`event_id`, unexpected fields,
    `flush` with any scope/event/actor field, `abandon` with anything but
    `scope_id`. Error type is `anyhow::Result` with the existing
    `Invalid <label> payload from STDIN: <detail>.` message convention. No CLI
    routing, runtime invocation, or STDIN reading (all T02). Module never names
    `WorktreeId`.
  - Verify results:
    - `test ... services::hooks::mutation_scope` — pass (20 passed, 0 failed).
    - `clippy --all-targets -- -D warnings` — pass (clean).
    - `fmt --check` — pass.
  - Deviation: per a follow-up user instruction, all comments (module and item
    doc comments included) were stripped from the new file; behavior and tests
    unchanged, re-verified clean.
  - Context impact: Additive-internal. New crate-internal module and one module
    registration line. No public interface, data shape, persistence, or
    documented-behavior change; the parser has no production consumer until T02.
    Root context files (`context/overview.md`, `context/context-map.md`, etc.)
    describe the mutation-scope runtime as unwired — still accurate after T01,
    since nothing invokes the runtime yet. No context file requires an update for
    this task; the durable-context work is planned as T04.
  - Context synchronization: synced

- [x] T02: `Add CLI routing and wire the existing runtime` (status:done)
  - Task ID: T02
  - Scope: In — `cli_schema::HooksSubcommand::MutationScope` (hidden, kebab
    `mutation-scope`, "reads JSON payload from STDIN" about text);
    `convert_hooks_subcommand_request` arm →
    `services::hooks::HookSubcommand::MutationScope`; the `HookSubcommand`
    variant; the `run_hooks_subcommand_in_repo` dispatch arm calling a new
    `mutation_scope::run_mutation_scope_subcommand(repository_root, logger)`;
    `hook_runtime_invocation_name` arm ("mutation-scope runtime invocation");
    updated parser/help tests in `command_runtime.rs` per repo convention. The
    new function reads STDIN, calls `parse_mutation_scope_payload`, and drives the
    runtime: `Start`/`Advance`/`Close` → `RuntimeBoundary::Start`/`Advance`/`Close`
    with `ScopeId(scope_id)`, `EventId(event_id)`, mapped `ActorKind`, passed to
    `mutation_trace::runtime::coordinate(repository_root, &boundary, || open_agent_trace_db_for_hook_runtime(repository_root, "Failed to open Agent Trace DB for mutation-scope runtime."))`;
    `Flush` → `RuntimeBoundary::Flush` through the same call; `Abandon` →
    `mutation_trace::runtime::abandon_scope(repository_root, &ScopeId(scope_id), || open_agent_trace_db_for_hook_runtime(..))`.
    Result classification: a pre-completion `CoordinateError` / `AbandonScopeError`
    → `CliError` (non-zero); `CoordinateError::MarkerClearAfterCommit { .. }` and
    `AbandonScopeError::MarkerClearAfterCompletion { .. }` → durable success
    (carried outcome kept, cleanup failure logged, empty stdout, exit 0, no
    re-execution of the transition); any valid `CoordinateOutcome` /
    `AbandonScopeOutcome` (including no-op/rejected/replayed) → empty stdout; no
    outcome/revision/worktree/scope serialization to stdout. Malformed payload →
    `CliError`. Focused `#[cfg(test)] mod tests` for the two carried-outcome
    classifications (proving the runtime entrypoint runs once) and the malformed
    → `Err` path, using a seam that lets the test inject each runtime result.
    Out — real Git/DB regressions (T03), context (T04). Do not disturb
    `pre-commit`/`commit-msg`/`post-commit`/`post-rewrite`/`diff-trace`/
    `conversation-trace`/`codex`/`claude-model-state`. Do not move DB acquisition
    above the runtime's protected prefix. There must be no completed state in
    which a valid payload returns success without invoking the runtime.
  - Dependencies: T01
  - Done when: `sce hooks mutation-scope` parses through clap to
    `HookSubcommand::MutationScope`; every valid operation reaches its mapped
    runtime entrypoint with identities forwarded verbatim; the DB resolver is
    only ever called inside the provider closure; pre-completion runtime failures
    propagate as `CliError`; the two carried-outcome variants resolve to
    empty-stdout success without re-running the transition; malformed payloads
    return `CliError`; `sce --help` still hides `hooks`; existing hook
    parser/help tests plus the new conversion and classification tests pass;
    `clippy --all-targets -- -D warnings` and `fmt --check` are clean.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::parse::command_runtime`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`; `rg -n 'open_db|coordinate\(|abandon_scope\(|MarkerClearAfterCommit|MarkerClearAfterCompletion|open_agent_trace_db_for_hook_runtime' cli/src/services/hooks/mutation_scope.rs`; `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/cli_schema.rs` — added the hidden `HooksSubcommand::MutationScope`
      variant (kebab `mutation-scope`, "reads JSON payload from STDIN" about text).
    - `cli/src/services/parse/command_runtime.rs` —
      `convert_hooks_subcommand_request` arm mapping
      `HooksSubcommand::MutationScope` → `HookSubcommand::MutationScope`; new
      `mutation_scope_hook_parses_to_hook_subcommand` parser test.
    - `cli/src/services/hooks/mod.rs` — `HookSubcommand::MutationScope` variant;
      `run_hooks_subcommand_in_repo` dispatch arm calling
      `mutation_scope::run_mutation_scope_subcommand(repository_root, logger)`
      (unwrapped `Result`, non-fail-open like `PreCommit`);
      `hook_runtime_invocation_name` arm ("mutation-scope runtime invocation");
      dropped the now-unnecessary `#[allow(dead_code)]` on `pub mod mutation_scope`.
    - `cli/src/services/hooks/mutation_scope.rs` — `run_mutation_scope_subcommand`
      (reads STDIN via `super::read_hook_stdin`), `run_mutation_scope_from_payload`,
      and a testable `drive_mutation_scope` generic over injectable
      `coordinate`/`abandon_scope` seams; `classify_coordinate` /
      `classify_abandon` durable-completion classification;
      `log_marker_clear_after_durable_completion`; `#[cfg(test)] mod
      runtime_dispatch` (10 tests).
  - Result: `sce hooks mutation-scope` now routes through the normal
    CLI/hook stack to `HookSubcommand::MutationScope` and stays hidden (no
    `hooks` entry in `sce --help`; `sce hooks --help` lists it).
    `run_mutation_scope_subcommand` reads one STDIN payload, parses it with the
    T01 `parse_mutation_scope_payload`, and `drive_mutation_scope` builds one
    `RuntimeBoundary` (`Start`/`Advance`/`Close`/`Flush`) forwarding
    `ScopeId`/`EventId`/`ActorKind` verbatim, or returns early into
    `abandon_scope` for `Abandon`. The real path passes a `FnOnce` provider
    closure (`|| super::open_agent_trace_db_for_hook_runtime(root,
    MUTATION_SCOPE_DB_CONTEXT)`) straight into `runtime::coordinate` /
    `runtime::abandon_scope`, so DB acquisition stays inside the runtime's
    protected-worktree ordering. Classification is by durable completion:
    `Ok(_)` → empty stdout; `CoordinateError::MarkerClearAfterCommit` /
    `AbandonScopeError::MarkerClearAfterCompletion` → cleanup failure logged via
    `logger.warn`, empty stdout, `Ok` (transition not re-run — the injected
    seam is `FnOnce` and is invoked exactly once); every other error →
    `Err(anyhow!(...))`, surfaced as `CliError` by `HooksCommand::execute`.
    Successful runs serialize nothing (`Ok(String::new())`). No production
    write path touches `diff_traces`/`post_commit_patch_intersections`/
    `agent_traces`. Real Git/DB regressions and durable context are T03/T04.
  - Verify results:
    - `test services::parse::command_runtime` — pass (11 passed, 0 failed;
      includes `mutation_scope_hook_parses_to_hook_subcommand`).
    - `test services::hooks::` — pass (217 passed, 0 failed).
    - `test services::hooks::mutation_scope` — pass (29 passed, 0 failed;
      10 new `runtime_dispatch` tests).
    - `rg` seam check — `open_agent_trace_db_for_hook_runtime` appears only
      inside the provider closures passed to `coordinate(...)` /
      `abandon_scope(...)`; `rg 'fail.open|failed open|exit 0'
      cli/src/services/hooks/mutation_scope.rs` returns nothing.
    - `clippy --all-targets -- -D warnings` — pass (clean).
    - `fmt --check` — pass (after `cargo fmt`).
  - Deviation: the ingress→runtime call is split into `drive_mutation_scope`
    generic over two injectable `FnOnce` seams (defaulting to the real
    `coordinate` / `abandon_scope` wrapped with the DB-provider closure), per
    the plan's "seam that lets the test inject each runtime result" and the
    cover note allowing local shape choices. `run_mutation_scope_subcommand`
    returns `Result<String>` dispatched without an `Ok(...)` fail-open wrapper;
    empty-stdout success is `Ok(String::new())`. This task was implemented on
    branch `mutation-scope-ingress` (the plan and T01 output live only there;
    the plan is stacked on `origin/mutation-trace-agent-attribution`).
  - Context impact: Additive behavior. New hidden CLI subcommand `sce hooks
    mutation-scope` and its runtime wiring; no change to existing hooks,
    mutation protocol, DB schema, migrations, or attribution behavior. Task
    synchronization corrected every durable statement made false by T02 — the
    "not wired into any hook or command" / "nothing calls either entrypoint" /
    "no harness, hook, or command calls this" / "harness/command wiring remains
    future work" / "a seam whose consumers do not exist yet" / "only
    harness/command wiring remains" clauses in `context/overview.md`,
    `context/context-map.md`, `context/cli/mutation-scope-runtime.md` (Status and
    the seam re-export rationale), `context/cli/mutation-trace-runtime-coordinator.md`
    (intro + Status), `context/cli/mutation-trace-scope-abandonment.md` (Status),
    `context/cli/mutation-trace-protocol.md` (intro + "Target end-state
    architecture"), and this plan's own Change summary and Context sync section —
    and added the command to the implemented surface list in
    `context/sce/agent-trace-hooks-command-routing.md`. Every correction keeps
    the generic-SCE-ingress-exists vs concrete-harness-adapter-exists
    distinction, and `context/cli/mutation-trace-ref-reconciliation.md`'s
    "`reconcile_worktree` … no harness/command wiring yet" was verified still
    accurate and left unchanged. The comprehensive
    `context/cli/mutation-scope-hook-ingress.md` domain file and the full
    non-fail-open intake contract remain T04's scope per the plan.
  - Context synchronization: synced

- [ ] T03: `Add real Git/DB mutation-scope ingress regressions` (status:todo)
  - Task ID: T03
  - Scope: In — `#[cfg(test)] mod tests` coverage (in `mutation_scope.rs` or a
    sibling test module following `hooks/mod.rs`'s `mutation_attribution_e2e`
    precedent) that drives the real ingress entrypoint against an RAII
    `tempfile::TempDir` real `git init` repository and a real temp-file
    `RepositoryAgentTraceDb`, with direct durable-row assertions and real Git
    trees:
    - **Test1 — observed lifecycle:** `start(A,e1,claude_code) → edit →
      advance(A,e2,claude_code) → close(A,e3,claude_code)`; assert scope `Closed`,
      processed events `(A,e1)/(A,e2)/(A,e3)`, exactly one mutation event over
      the edit interval, `AiExclusive(A)`, cursor tree = final observed Git tree,
      no manually inserted event.
    - **Test2 — replay:** `start(A,e1) → edit → advance(A,e2)`, snapshot
      `revision` + `mutation_trace_events` count + `mutation_trace_processed_events`
      count, then `advance(A,e2)` again; assert all three counts unchanged and
      exactly one processed key `(A,e2)`. Fails if `EventId` is transformed.
    - **Test3 — actor mismatch:** `start(A,e1,claude_code)` recording `revision`
      + processed rows + scope actor/status, then `advance(A,e2,codex)`; assert
      `ScopeIdentityConflict`, `revision` unchanged, `(A,e2)` absent from
      `mutation_trace_processed_events`, scope `A` still `actor_kind = claude_code`
      with unchanged status, no new `mutation_trace_events` row.
    - **Test4 — abandonment with unobserved edit:** `start(A,e1) → record
      cursor_tree → unobserved filesystem edit → abandon(A)`; assert scope
      `Abandoned`, `revision` advanced exactly once, `needs_rebaseline = true`,
      `cursor_tree` still the pre-edit tree (not the edited Git tree), no
      `mutation_trace_events` row for the edit, no processed-event row from
      abandon.
    - **Test5 — adversarial flush:** baseline tree → unscoped filesystem edit →
      `{"operation":"flush"}`; assert `cursor_tree` moved to the edited Git tree,
      `revision` advanced, exactly one `mutation_trace_events` row for the
      transition with `attribution = IneligibleUnscoped`, no `mutation_trace_scopes`
      row invented, no `mutation_trace_processed_events` row invented. A
      `Flush => Ok("")` stub must fail this test.
    - **Test6 — marker-clear-after-commit carried outcome:** drive an
      attributable `advance` whose trailing `marker.clear()` fails (via the
      runtime's existing test seam), asserting the ingress returns empty-stdout
      success, the committed `CoordinateOutcome` is the durable state, and the
      transition is not executed a second time.
    - **Test7 — marker-clear-after-abandon carried outcome:** the equivalent for
      `AbandonScopeError::MarkerClearAfterCompletion`.
    - Every test also asserts `diff_traces` / `post_commit_patch_intersections` /
      `agent_traces` are untouched.
    Out — context updates (T04).
  - Dependencies: T02
  - Done when: all seven regressions pass, exercising the real `coordinate()` /
    `abandon_scope()` paths through the ingress.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`.
  - Context synchronization: pending

- [ ] T04: `Document the mutation-scope hook ingress` (status:todo)
  - Task ID: T04
  - Scope: In — create `context/cli/mutation-scope-hook-ingress.md` (JSON
    contract, operation mapping, identity ownership, no-`worktree_id` rationale,
    no-`ScopeId`/`EventId`-generation rationale, non-fail-open error semantics
    including the two carried-outcome success variants and why they must not be
    retried, lazy DB-provider requirement, empty-stdout contract, abandonment
    ownership, generic-ingress vs harness-adapter boundary); update
    `context/cli/mutation-scope-runtime.md` Status; add the command to
    `context/sce/agent-trace-hooks-command-routing.md` with its non-fail-open
    intake contract; register the new file and adjust annotations in
    `context/context-map.md`; finalize the `mutation_trace` wiring paragraph in
    `context/overview.md` (T02 already removed the "not wired into any hook or
    command" clause). Out — code changes.
  - Dependencies: T03
  - Done when: the new context file exists and covers every listed point, the
    four updated files reflect the shipped ingress while preserving the
    generic-ingress vs harness-adapter distinction, and `nix run .#pkl-check-generated`
    plus `nix flake check` pass.
  - Verify: `nix run .#pkl-check-generated`; `nix flake check`; `rg -n 'mutation-scope-hook-ingress' context/context-map.md context/cli/mutation-scope-runtime.md`.
  - Context synchronization: pending

## Open questions

None. The runtime seam, the identity-ownership rules, and the two-class error
contract are all fixed by `context/cli/mutation-scope-runtime.md`, the runtime
source on the current base, and the request; the one implementation wrinkle
(serde `deny_unknown_fields` on an internally tagged enum) is a local parser
choice recorded under Assumptions, not a scope question.
