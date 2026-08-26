//! Pure Rust domain representation for the refinement of the verified
//! `spec/mutation_cursor.qnt` mutation-cursor protocol.
//!
//! This module defines the protocol's domain/state types, pure accessors,
//! `prepare`/`commitAttempt` transition logic for all four boundary kinds,
//! attribution derivation, mutation-event materialization, snapshot-failure
//! taint, database-failure external taint, scope abandonment, and recovery
//! with an explicit observed-tree input. This completes the action set the
//! plan scoped for `protocol.rs`.
//!
//! No Git, database, filesystem, environment, network, async, or lock I/O is
//! performed here.
//! The module is not yet wired into any hook, command, or database call
//! site: that integration, along with the `coordinator.rs` (imperative
//! shell), `git_snapshot.rs` (isolated Git snapshot capture), and `store.rs`
//! (DB-backed CAS persistence) seams the target architecture will grow into,
//! is left for later work. This layout leaves those three seams as the
//! natural home for: `coordinator.rs` loading/persisting state and supplying
//! the observed-tree inputs `prepare`/`recover` take as explicit parameters;
//! `git_snapshot.rs` capturing and diffing worktree trees; and `store.rs`
//! implementing the CAS-transactional persistence and runtime scope
//! materialization contract described in
//! `context/cli/mutation-trace-protocol.md`.
//!
//! # Quint refinement matrix
//!
//! This module refines `spec/mutation_cursor.qnt`. The table below is
//! auditable against that file: for every relevant Quint element it names
//! the element, whether it is verification-only model instrumentation or a
//! semantic property, its Rust counterpart (if any), its classification, and
//! the concrete test or mechanism backing a non-verification-only
//! classification. A property is classified `verification-only` only when it
//! has no production semantic meaning beyond restating the consistency of
//! the omitted instrumentation itself — never merely because Quint happens
//! to state it using a history variable.
//!
//! ## Verification-only model instrumentation
//!
//! These concrete Quint checkpoint types and history/counter variables exist
//! only to state or prove properties against the finite, enumerated Quint
//! model. `ProtocolState` does not materialize any of them, and no
//! production Rust equivalent exists, unless a future adapter needs one for
//! another reason: `CursorCheckpoint`, `ProtocolCheckpoint`,
//! `ScopeCheckpoint`, `AbandonCheckpoint`, `StartCheckpoint`,
//! `RecoveryCheckpoint`, `DurableProtocolCheckpoint`, and the variables
//! `cursorHistory`, `protocolHistory`, `scopeHistory`, `abandonHistory`,
//! `startHistory`, `recoveryHistory`, `taintHistory`, `evidenceAttempts`,
//! `scopeStartCount`, `everTerminal`.
//!
//! Several Quint invariants exist solely to check the internal consistency
//! of that instrumentation (for example, that two checkpoints for the same
//! worktree/revision are equal, or that a history contains an entry matching
//! current state) rather than to state a fact about production behavior;
//! these are classified `verification-only` alongside the instrumentation
//! they describe: `CursorHistoryUniquePerWorktreeRevision`,
//! `CursorHistoryHasCurrentState`, `ProtocolHistoryUniquePerWorktreeRevision`,
//! `ProtocolHistoryHasCurrentState`, `ScopeHistoryUniquePerWorktreeRevision`,
//! `AbandonHistoryUniquePerWorktreeRevisionScope`, and the witness invariants
//! `HasExclusiveEvidence`/`HasContendedEvidence`/`HasUnscopedEvidence`/
//! `HasRejectedAttempt` (reachability checks over the finite model, not
//! production properties).
//!
//! A verification-only **data structure** is not the same thing as a
//! verification-only **invariant**: `AbandonCreatesRebaselineRequirement`,
//! `MutationEventsMatchCursorHistory`, and
//! `MutationEventsCrossOnlyTrustworthyProtocolStates` are all *stated* using
//! the history variables above, but each proves a fact this module's
//! production transitions must actually uphold, so all three are classified
//! below as semantic properties, not verification-only.
//!
//! Quint's finite `SCOPES`/`WORKTREES` universes and `init`'s eager
//! population of every `ScopeState`/`WorktreeState` are **external adapter
//! responsibility**: this refinement's `ScopeId`/`WorktreeId` spaces are
//! unbounded runtime strings, so scope/worktree identity is materialized at
//! runtime by the future `coordinator.rs`/`store.rs` layer rather than at
//! protocol startup (see the "Runtime scope materialization" assumption
//! recorded in the plan and in `context/cli/mutation-trace-protocol.md`).
//!
//! ## Semantic properties
//!
//! | Quint element | Rust counterpart | Classification | Backing mechanism |
//! |---|---|---|---|
//! | `CursorRevisionConsistent` | `WorktreeState::revision: u64` | enforced by Rust type | `u64` cannot represent a negative revision |
//! | `AbandonCreatesRebaselineRequirement` | `abandon` | preserved by transition tests | `abandon_transitions_a_live_scope_without_moving_the_cursor_or_changing_identity` proves, in one assertion set, that abandoning sets `Abandoned`+`needs_rebaseline`, leaves the cursor untouched, advances revision by exactly one, and leaves `mutation_events` equal to its pre-abandon value (no evidence emitted for the abandonment revision) |
//! | `FailureKindMatchesTaint` | `WorktreeState::{tainted, failure_kind}` | preserved by transition tests | `taint`/`recover` always set both fields together; `taint_changes_exactly_tainted_failure_kind_and_revision`, `recover_from_snapshot_taint_abandons_live_scopes_and_rebaselines_cursor` |
//! | `TerminalScopesStayTerminal` | `ScopeStatus::{Closed, Abandoned}` | preserved by transition tests | `scope_started_at_most_once_and_stays_terminal_after_a_real_close`, `start_on_a_scope_abandoned_via_a_real_transition_never_reactivates_it` (terminal state reached via real transitions, then re-attempted) |
//! | `ScopeStartedAtMostOnce` | `commit`'s `Start`/`observes` gate | preserved by transition tests | `scope_started_at_most_once_and_stays_terminal_after_a_real_close` |
//! | `DatabaseFailureDoesNotMutateDurableProtocolState` | `database_failure` | preserved by transition tests | `database_failure_changes_exactly_external_taint` |
//! | `ExternalTaintNeverStrengthensAttribution` | `attribution_for` | preserved by transition tests | `attribution_for_is_ineligible_unscoped_when_worktree_is_externally_tainted_even_with_an_active_scope` |
//! | `RecoveryClearsExternalTaintOnlyAfterBaseline` | `recover` | preserved by transition tests | `recover_from_external_taint_abandons_live_scopes_and_clears_external_taint`, `database_failure_then_recover_clears_external_taint_and_rebaselines_cursor` (cursor rebaseline and `external_taint` clearing happen in the same transition) |
//! | `ScopeActorIdentityIsStable` | `ScopeState::actor_kind` | preserved by transition tests + external adapter responsibility | `abandon_transitions_a_live_scope_without_moving_the_cursor_or_changing_identity`; a future adapter must reject a conflicting `actor_kind` for an existing `ScopeId` rather than overwrite it |
//! | `NoNoopMutationEvents` | `commit`'s `changed` gate | preserved by transition tests | `commit_emits_no_mutation_event_for_a_no_op_tree_change` |
//! | `MutationEventsHavePositiveRevision` | `MutationEvent::revision` | preserved by transition tests | `commit` always sets a `MutationEvent`'s revision to the same checked `advanced_revision` (`revision.checked_add(1)`, never `0`) it writes to the worktree; `commit_emits_exactly_one_mutation_event_with_correct_attribution_boundary_and_revision_for_a_real_change` |
//! | `MutationEventUniquePerWorktreeRevision` | `ProtocolState::mutation_events` | preserved by transition tests | one `commit` call inserts at most one event, tagged with the freshly advanced revision, which strictly increases; `attribution_transitions_from_contended_to_exclusive_across_a_close_boundary` produces three events at three distinct revisions |
//! | `MutationFailureKindMatchesTaint` | `MutationEvent::{tainted, failure_kind}` | preserved by transition tests | copied verbatim from the pre-transition `WorktreeState` in `ResolvedAttempt::apply`, so `FailureKindMatchesTaint` carries over |
//! | `MutationEventsMatchCursorHistory` | `ResolvedAttempt::apply`'s `MutationEvent` construction | implemented directly + preserved by transition tests | `apply` derives `before_tree`/`after_tree`/`revision` from the same prepared attempt and the same `advanced_revision` the worktree update itself uses — they cannot diverge by construction; `commit_emits_exactly_one_mutation_event_with_correct_attribution_boundary_and_revision_for_a_real_change`, `attribution_transitions_from_contended_to_exclusive_across_a_close_boundary` (three events at three distinct revisions, each matching its own commit's before/after tree) |
//! | `MutationEventsCrossOnlyTrustworthyProtocolStates` | `commit`'s `changed` gate (`observed_change && !needs_rebaseline`) | implemented directly + preserved by transition tests | `changed` — the sole gate for `MutationEvent` construction — is `false` whenever the pre-transition worktree has `needs_rebaseline: true`, independent of whether a real tree change was observed; `needs_rebaseline_suppresses_mutation_event_even_when_commit_observes_a_real_tree_change` proves `commit` reaches `accepted && observes && observed_change` and still emits no event and leaves the cursor unmoved |
//! | `NeedsRebaselineSuppressesAttribution` | `attribution_for` | preserved by transition tests | `attribution_for_is_ineligible_unscoped_when_worktree_needs_rebaseline_even_with_an_active_scope` |
//! | `AttributionMatchesObservedScopes` | `attribution_for` | preserved by transition tests | `attribution_for_is_ai_exclusive_for_exactly_one_live_scope`, `attribution_for_is_ai_contended_for_multiple_live_scopes`, `attribution_for_is_ineligible_unscoped_when_no_scope_is_live` |
//! | `AiExclusiveRequiresExactlyOneActiveScope` | `Attribution::AiExclusive(ScopeId)` | implemented directly + preserved by transition tests | `Attribution::AiExclusive(ScopeId)` does not itself make an inconsistent scope count unrepresentable (a caller could construct it with any `ScopeId`); the guarantee comes from `attribution_for`'s own algorithm, which only reaches its `AiExclusive` branch when `live.len() == 1`, wrapping that exact scope; `attribution_for_is_ai_exclusive_for_exactly_one_live_scope` |
//! | `AiContendedRequiresMultipleActiveScopes` | `Attribution::AiContended` | preserved by transition tests | `attribution_for_is_ai_contended_for_multiple_live_scopes` |
//! | `RejectedAttemptsDoNotCommitEvidence` | `commit`'s rejection path | preserved by transition tests | rejection returns before the `changed`/`mutation_events` step is ever reached; `rejected_attempts_do_not_commit_evidence_across_a_mixed_accept_reject_sequence`, `competing_prepared_attempts_the_second_to_commit_is_rejected_by_cas`, `taint_invalidates_a_prepared_attempt_via_stale_revision` |
//! | `StartDoesNotAbandonExistingScopes` | `commit`'s `Start` scope transition | preserved by transition tests | only the boundary's own `scope_id` entry is ever written; `start_does_not_abandon_existing_scopes_multi_scope_sequence` |
//!
//! Five properties in the table above (`AbandonCreatesRebaselineRequirement`,
//! `MutationEventsMatchCursorHistory`,
//! `MutationEventsCrossOnlyTrustworthyProtocolStates`,
//! `RejectedAttemptsDoNotCommitEvidence`, `StartDoesNotAbandonExistingScopes`)
//! are stated in Quint using history variables (`abandonHistory`/
//! `protocolHistory`/`cursorHistory`/`scopeHistory`, `evidenceAttempts`,
//! `startHistory`/`scopeHistory` respectively) but are classified
//! production-semantic, not `verification-only`: the instrumentation itself
//! is omitted, but the fact each one states — no evidence is emitted for an
//! abandonment revision, an emitted event's before/after tree and revision
//! always match the transition that produced it, no evidence crosses a
//! `needs_rebaseline` boundary, no rejected attempt's boundary contributes a
//! `MutationEvent`, and starting one scope never mutates another — is a real
//! guarantee this module's `commit`/`abandon` must uphold, independently
//! verified by the named tests above without needing the history variables
//! themselves. A verification-only *mechanism* never by itself demotes the
//! *property* it was used to prove.
//!
//! ## Bounded-integer revision refinement
//!
//! Quint's `revision: int` (`spec/mutation_cursor.qnt:39`) is an unbounded
//! integer; this refinement's `WorktreeState::revision: u64` is not. Every
//! action that advances a worktree's revision — `commit`, `taint`,
//! `abandon`, `recover` — routes through the private `next_revision`
//! (`revision.checked_add(1)`) helper in `protocol.rs` rather than a raw
//! `+ 1`, so a worktree already at `revision: u64::MAX` cannot be advanced
//! and cannot wrap to `0`. `commit`'s `accepted` gate folds this check in
//! unconditionally (a would-be-overflowing attempt is rejected exactly like
//! a stale one, with no cursor movement, scope transition, processed-
//! `EventKey` insertion, or `MutationEvent`); `taint`/`abandon`/`recover`
//! treat it as an additional guarded no-op alongside their existing
//! existence/precondition guards. This has no Quint counterpart — Quint's
//! `revision` never needs such a guard — so it is a Rust-only refinement
//! precondition, verified by `commit_does_not_wrap_revision_at_u64_max`,
//! `taint_does_not_wrap_revision_at_u64_max`,
//! `abandon_does_not_wrap_revision_at_u64_max`, and
//! `recover_does_not_wrap_revision_at_u64_max`, each starting from
//! `revision: u64::MAX` and proving the action is a no-op (or, for `commit`,
//! a rejection) rather than a wrap.

pub mod protocol;
pub mod types;

#[cfg(test)]
mod tests;
