# Plan: mutation-cursor-close-never-seen-attribution

## Change summary

Close an unsafe positive-attribution path in the generic mutation-cursor
protocol. `commitAttempt` (`spec/mutation_cursor.qnt`) and `commit`
(`cli/src/services/mutation_trace/protocol.rs`) accept `Close(scope)` when the
scope is `NeverSeen` (kept for idempotency/recovery). Attribution comes from
`attributionForBoundary` / `attribution_for_boundary`, which only looks at the
scopes that were `Active` before the boundary. A `NeverSeen` scope is not in
that set, so the confirmation-required check never sees it. Example: Claude
scope A is `Active`, Codex scope B is `NeverSeen`, the tree changes, and
`Close(B)` emits `AiExclusive(A)`, even though B's boundary observed the
mutation and B's `Start` may never have been durably observed.

The fix adds one explicit boundary-attribution rule to both the Quint model
and the Rust kernel, with the same structure in each. When the boundary is
`Close(scope)` and that scope's pre-boundary status is `NeverSeen`, the
attribution is `IneligibleUnscoped`. Otherwise the existing logic runs
unchanged. `Close(NeverSeen)` keeps its existing acceptance, observation,
cursor/rebaseline semantics, and transition to `Closed`. Only its attribution
changes.

The model needs three changes to stay consistent with that rule. First, a
verification-only Quint history, `closeFromNeverSeenHistory`, records each
accepted close-from-`NeverSeen` boundary by `(worktreeId, revision)`. This is
needed because `MutationEvent` does not store the scope's pre-boundary status.
Second, the existing `AttributionMatchesObservedScopes` invariant gets a new
first branch that uses this history: for those events it requires
`IneligibleUnscoped` before any active-scope cardinality check. Without this
branch the invariant would reject the fix, since the regression event has
`activeScopes = {A}` and the invariant would still demand `AiExclusive(A)`.
Third, a new explicit invariant, `CloseFromNeverSeenNeverGetsPositiveAttribution`,
states the property directly. Because the history is a new top-level Quint
state variable, the Quint Connect `WireModelState` (`deny_unknown_fields`) must
list it as an `IgnoredAny` field, which keeps it out of the compared
`ModelState`. This extends existing behavior: normal `Active -> Close`
attribution is preserved.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: In the Quint model, a `Close` whose target scope was `NeverSeen` right before the transition, with an observed tree change, emits a mutation event attributed `IneligibleUnscoped`, regardless of other live scopes. The scope still ends `Closed`, and acceptance, observation and cursor/rebaseline semantics are unchanged. In the healthy regression run the cursor advances to the observed tree.
  - Validate: `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'` passes, including a new named run that reproduces the bug (Claude `Scope0` Active, Codex `Scope2` NeverSeen, `mutate(WT0, Tree1)`, `Close(Scope2)`) and expects `IneligibleUnscoped`, `scopes.get(Scope2).status == Closed`, `worktrees.get(WT0).cursorTree == Tree1`, no `AiExclusive(Scope0)` event, and `Safety`.
- [x] AC2: The model records every accepted close-from-`NeverSeen` boundary in the verification-only `closeFromNeverSeenHistory` (keyed by `(worktreeId, revision)`). It also states and checks `CloseFromNeverSeenNeverGetsPositiveAttribution`: every mutation event whose `(worktreeId, revision)` is in that history is `IneligibleUnscoped`, never `AiExclusive(_)` or `AiContended`. The invariant identifies these events through the history, not through `event.activeScopes`, and it is part of `SafetyAttribution`.
  - Validate: `nix run .#quint -- typecheck spec/mutation_cursor.qnt` succeeds. Inspect `spec/mutation_cursor.qnt`: `closeFromNeverSeenHistory` is declared, initialized and assigned in every action, and `commitAttempt` adds to it on an accepted `Close` from `NeverSeen`. The invariant is defined over that history and listed in `SafetyAttribution`.
- [x] AC3: `AttributionMatchesObservedScopes` accepts the corrected behavior. Its first branch requires `event.attribution == IneligibleUnscoped` for any event whose `(worktreeId, revision)` is in `closeFromNeverSeenHistory`. The existing failure, no-active-scope, unconfirmed-required-scope, single-scope and contended branches follow in their current order and are otherwise unchanged. The new regression run, and every existing `test*` run, satisfies `Safety` with this invariant in place.
  - Validate: inspect `AttributionMatchesObservedScopes` in `spec/mutation_cursor.qnt` for the leading close-from-`NeverSeen` branch; `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'` passes, and the new run's `.expect(Safety)` holds.
- [x] AC4: Rust `attribution_for_boundary` / `commit` produce the same result: the regression case (A Active Claude, B NeverSeen Codex, cursor tree0, observed tree1, `Close(B)`) gives `accepted = true`, `observes = true`, exactly one mutation event, attribution `IneligibleUnscoped` (explicitly asserted `!= AiExclusive(A)`), and B `Closed`. A lone `NeverSeen` scope closed over a tree change also gives `IneligibleUnscoped`. `Active -> Close` over a tree change keeps its current attribution (`AiExclusive` for the closing scope). `Close(NeverSeen)` with `before == after` still emits no mutation event.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` passes, including the four new deterministic tests.
- [x] AC5: The Rust kernel still refines the Quint model under Quint Connect, including the new named scenario (picked up by the `test.*` backstop) and generated traces. Quint traces containing `closeFromNeverSeenHistory` deserialize without error, and the history is excluded from the compared `ModelState`.
  - Validate: `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs` succeeds. Inspect `cli/src/services/mutation_trace/mbt/model.rs`: `WireModelState` has a `close_from_never_seen_history: IgnoredAny` field in its verification-only group, and `ModelState` has no corresponding field.
- [x] AC6: `spec/mutation_cursor.md` lists the close-from-`NeverSeen` rule in the attribution rules, lists the new invariant and scenario in the verification section, and no longer implies that a close-only boundary can receive positive attribution.
  - Validate: inspect the "Attribution is computed for the transition observed *at a boundary*" list and the "Verification properties and scenarios" section of `spec/mutation_cursor.md`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
- `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`
- `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`

### Context sync

- `context/cli/mutation-trace-protocol.md`: change the "a `Close` boundary's emitted event still attributes to the scope it is about to close" sentence so it covers only `Active -> Close`, add the close-from-`NeverSeen` `IneligibleUnscoped` rule, and refresh the stale `attributionFor` spec line ranges cited there and in `protocol.rs` doc comments.
- `context/cli/mutation-trace-quint-connect.md`: record `closeFromNeverSeenHistory` as a verification-only variable that is listed as `IgnoredAny` in `WireModelState` and excluded from `ModelState`. Also fix the existing drift: the doc says `serde`'s default unknown-field handling drops `mbtAction`, but `WireModelState` uses `deny_unknown_fields` and lists each verification-only variable explicitly as `IgnoredAny`.
- `context/glossary.md` / `context/context-map.md`: update only if an existing entry describes `Close` attribution.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `spec/mutation_cursor.qnt` (attribution rule, `closeFromNeverSeenHistory`, `CloseFromNeverSeenNeverGetsPositiveAttribution`, the `AttributionMatchesObservedScopes` update, named run); `spec/mutation_cursor.md`; `cli/src/services/mutation_trace/protocol.rs` (`attribution_for_boundary` and its doc comments); `cli/src/services/mutation_trace/tests.rs` (deterministic tests); `cli/src/services/mutation_trace/mbt/model.rs` (`IgnoredAny` field for the new history).
- **Out of scope:** scope lifecycle changes (acceptance, `observes`, cursor/rebaseline semantics and the `Closed` transition for `Close(NeverSeen)` stay as they are); changing normal `Active -> Close` attribution; adapter lifecycle behavior (Claude/Codex/OpenCode/Pi); `store.rs`, persistence schema and the `MutationEvent` shape; the runtime coordinator; event-persistence special-casing.
- **Constraints:** Quint, Rust and the Quint Connect wire model change together in one commit. The Quint Connect check compares `mutationEvents` attribution after every step, and `WireModelState` rejects unknown Quint state variables, so any one-sided change breaks refinement or deserialization. Quint and Rust stay structurally aligned. The rule lives in `attributionForBoundary` / `attribution_for_boundary` and reads the boundary scope's pre-boundary status from the state that function already receives. Cargo and Quint run only through Nix.
- **Non-goal:** Removing support for `Close(NeverSeen)`, adding a pre-status field to `MutationEvent`, or generalizing to other unobserved-boundary cases (`Advance` from non-live is already non-observing).

## Assumptions

- The pre-boundary status is read from the same pre-transition `scopes` / `ProtocolState` that `attributionForBoundary` / `attribution_for_boundary` already consult, so no new parameter is needed. A missing scope never reaches `commit` (see `context/cli/mutation-trace-protocol.md`, "Missing scope vs. `NeverSeen` scope").
- The verification-only history is named `closeFromNeverSeenHistory: Set[{ worktreeId: WorktreeId, revision: int }]`, modeled after `startHistory`. It is initialized empty in `init`, carried unchanged by every other action (including `mbtStutterAs` and the rejected branch of `commitAttempt`), and extended in the accepted branch of `commitAttempt` when `isClose(boundary)` and the pre-transition status is `NeverSeen`, with revision `state.revision + 1`. It is recorded whether or not a mutation event is emitted. No semantic action reads it. Under `rename_all = "camelCase"`, its Rust wire field is `close_from_never_seen_history: IgnoredAny`.
- `(worktreeId, revision)` identifies a mutation event uniquely, because `MutationEventUniquePerWorktreeRevision` already guarantees it, so matching history entries to events by that key is sound.
- No Rust test mirrors `AttributionMatchesObservedScopes` (checked by searching `cli/src`), so updating the invariant needs no Rust change beyond the attribution rule itself.
- The named Quint run reuses `Scope0` (ClaudeCode, WT0) as A and `Scope2` (Codex, WT0) as B, matching the existing scenario conventions. A second run for the lone-scope case is optional, because the Rust deterministic test covers it.
- No existing named Quint run closes a never-started scope (checked by scanning all `run test*` bodies), so no current scenario expectation changes.

## Task stack

- [x] T01: `Suppress attribution for Close from NeverSeen in Quint and Rust` (status:done)
  - Task ID: T01
  - Scope: In —
    - `spec/mutation_cursor.qnt`: conservative `IneligibleUnscoped` branch for `Close` from `NeverSeen` in `attributionForBoundary`; verification-only `closeFromNeverSeenHistory` (declared, initialized, carried by every action, recorded in the accepted branch of `commitAttempt`); new `CloseFromNeverSeenNeverGetsPositiveAttribution` invariant in `SafetyAttribution`; leading close-from-`NeverSeen` branch in `AttributionMatchesObservedScopes`; named regression run (A Active `Scope0`, B NeverSeen `Scope2`).
    - `cli/src/services/mutation_trace/protocol.rs`: matching branch in `attribution_for_boundary`, plus an updated doc comment.
    - `cli/src/services/mutation_trace/tests.rs`: four deterministic tests (regression A+B, lone NeverSeen close, `Active -> Close` preservation, `Close(NeverSeen)` with no tree change).
    - `cli/src/services/mutation_trace/mbt/model.rs`: `close_from_never_seen_history: IgnoredAny` in the verification-only group of `WireModelState`.

    Out — `spec/mutation_cursor.md` prose (T02), adapters, store/persistence schema, coordinator/runtime, `MutationEvent` shape, normal `Active -> Close` attribution.
  - Dependencies: none
  - Done when: Quint typechecks. All `test*` runs pass, including the new run, with `Safety` (and therefore the updated `AttributionMatchesObservedScopes` and the new invariant) holding. Rust mutation-trace tests pass, including the four new tests. The Quint Connect check builds green, with the new history deserialized as `IgnoredAny` and kept out of `ModelState`.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`; `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`; `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`.
  - Completed: 2026-10-04
  - Files changed: `spec/mutation_cursor.qnt`; `cli/src/services/mutation_trace/protocol.rs`; `cli/src/services/mutation_trace/tests.rs`; `cli/src/services/mutation_trace/mbt/model.rs`
  - Result: Quint adds `boundaryClosesNeverSeenScope` (checked first in `attributionForBoundary`), the `CloseFromNeverSeenCheckpoint` type and `closeFromNeverSeenHistory` var (initialized empty, carried by every action, extended in the accepted `commitAttempt` branch on `Close` from `NeverSeen` with `state.revision + 1`), `isCloseFromNeverSeenEvent`, the leading branch in `AttributionMatchesObservedScopes`, `CloseFromNeverSeenNeverGetsPositiveAttribution` in `SafetyAttribution`, and run `testCloseFromNeverSeenNeverGetsPositiveAttribution`. Rust adds `boundary_closes_never_seen_scope`, checked first in `attribution_for_boundary` with a doc comment; four deterministic tests; and `close_from_never_seen_history: IgnoredAny` in `WireModelState` (no `ModelState` field).
  - Verify outcomes: Quint typecheck passed; Quint `test*` passed (41 passing, including the new run); Rust `mutation_trace` tests passed (373 passed, 0 failed, including the four new tests); Quint Connect check built green (16 passed, including generated traces and all named scenarios).
  - Context impact: localized — mutation-trace protocol attribution rule and Quint Connect wire model; affects `context/cli/mutation-trace-protocol.md` and `context/cli/mutation-trace-quint-connect.md` per the plan's Context sync section.
  - Context synchronization: synced

- [x] T02: `Document close-from-NeverSeen attribution in the mutation-cursor spec` (status:done)
  - Task ID: T02
  - Scope: In — `spec/mutation_cursor.md`: add the rule to the boundary attribution list (ahead of the live-scope cases), explain why a close-only boundary cannot confirm or attribute (its `Start` was never durably observed), and add the invariant and the new deterministic scenario to "Verification properties and scenarios". Out — Quint/Rust code, `context/` files (handled by context sync).
  - Dependencies: T01
  - Done when: the attribution section states that `Close` from `NeverSeen` always yields `IneligibleUnscoped`, the verification section lists the new invariant and scenario, and no sentence implies positive attribution at a close-only boundary.
  - Verify: inspect `spec/mutation_cursor.md` attribution and verification sections; `nix run .#quint -- typecheck spec/mutation_cursor.qnt` (sanity, no code change expected).
  - Completed: 2026-10-04
  - Files changed: `spec/mutation_cursor.md`
  - Result: The boundary attribution list now leads with `Close(scope)` from `NeverSeen` → `IneligibleUnscoped` (regardless of other live scopes), with the remaining cases demoted to "otherwise". A new paragraph explains that `Close(NeverSeen)` keeps acceptance/observation/cursor/`Closed` semantics but cannot confirm or attribute because its `Start` was never durably observed, and that only `Active -> Close` is eligible for positive attribution. The "unconfirmed at every boundary except its own `Close`" sentence now says "`Close` from `Active`". The verification section lists `CloseFromNeverSeenNeverGetsPositiveAttribution` (via `closeFromNeverSeenHistory` and the leading `AttributionMatchesObservedScopes` branch) and the `testCloseFromNeverSeenNeverGetsPositiveAttribution` run.
  - Verify outcomes: inspected attribution and verification sections of `spec/mutation_cursor.md` (rule, rationale, invariant and scenario present; no close-only positive-attribution wording remains); `nix run .#quint -- typecheck spec/mutation_cursor.qnt` passed (exit 0).
  - Context impact: none beyond the plan's Context sync section — spec prose only; `context/cli/mutation-trace-protocol.md` and `context/cli/mutation-trace-quint-connect.md` remain covered by context sync.
  - Context synchronization: synced

## Open questions

None. The request specifies the rule, the invariants, the wire-model handling, the tests and the scope boundaries. The code confirms the bug path: `observes` accepts `NeverSeen` for `Close`, while `liveScopesOn` / `live_scopes_on` filter to `Active`. It also confirms the two consistency points: `AttributionMatchesObservedScopes` would demand `AiExclusive` for a single active scope, and `WireModelState` uses `deny_unknown_fields`.

## Validation Report

**Status:** validated  
**Date:** 2026-10-04

### Commands run

- `nix flake check` -> exit 0 (all 4 flake checks passed: cli-clippy, cli-fmt, cli-tests, mutation-trace-quint-connect)
- `nix run .#quint -- typecheck spec/mutation_cursor.qnt` -> exit 0 (typecheck clean)
- `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'` -> exit 0 (41 passing, including `testCloseFromNeverSeenNeverGetsPositiveAttribution`)
- `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs` -> exit 0 (check derivation built green)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` -> exit 0 (373 passed, 0 failed)

### Success-criteria verification

- [x] AC1: Quint `Close` from `NeverSeen` over a tree change yields `IneligibleUnscoped` with unchanged lifecycle/cursor semantics -> `testCloseFromNeverSeenNeverGetsPositiveAttribution` passes; it sets Scope0 Active, mutates WT0 to Tree1, closes NeverSeen Scope2 and expects `IneligibleUnscoped` with `activeScopes == {Scope0}`, Scope2 `Closed`, cursor `Tree1`, no `AiExclusive(Scope0)`, and `Safety`.
- [x] AC2: `closeFromNeverSeenHistory` recorded and `CloseFromNeverSeenNeverGetsPositiveAttribution` checked -> typecheck passed; inspected `spec/mutation_cursor.qnt`: var declared (l.294), initialized `Set()` in `init`, carried by every action, extended in accepted `commitAttempt` branch on `isClose(boundary) and currentScope.status == NeverSeen` with `state.revision + 1`; invariant defined via `isCloseFromNeverSeenEvent` (history-keyed, not `activeScopes`) and listed in `SafetyAttribution`.
- [x] AC3: `AttributionMatchesObservedScopes` accepts corrected behavior -> inspected: leading `isCloseFromNeverSeenEvent` branch requires `IneligibleUnscoped`, followed by the failure, no-active-scope, unconfirmed-required, single-scope and contended branches in original order; all 41 `test*` runs pass with `.expect(Safety)`.
- [x] AC4: Rust `attribution_for_boundary` / `commit` match -> `mutation_trace` tests pass, including `a_close_from_never_seen_codex_scope_beside_a_live_claude_scope_is_ineligible_not_exclusive`, `a_close_from_a_lone_never_seen_scope_over_a_tree_change_is_ineligible`, `a_close_from_an_active_scope_over_a_tree_change_keeps_exclusive_attribution`, and `a_close_from_a_never_seen_scope_without_a_tree_change_emits_no_mutation_event`.
- [x] AC5: Rust kernel still refines the Quint model -> Quint Connect check built green (standalone and inside `nix flake check`); inspected `cli/src/services/mutation_trace/mbt/model.rs`: `WireModelState` has `close_from_never_seen_history: IgnoredAny` in the verification-only group, and `ModelState` has no corresponding field.
- [x] AC6: `spec/mutation_cursor.md` documents the rule -> inspected: the boundary attribution list leads with `Close(scope)` from `NeverSeen` -> `IneligibleUnscoped`, followed by a rationale paragraph saying only `Active -> Close` is eligible for positive attribution; the "unconfirmed" sentence now reads "`Close` from `Active`"; the verification section lists `CloseFromNeverSeenNeverGetsPositiveAttribution` and `testCloseFromNeverSeenNeverGetsPositiveAttribution`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
