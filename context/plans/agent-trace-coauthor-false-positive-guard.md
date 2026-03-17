# Plan: agent-trace-coauthor-false-positive-guard

## Change summary

The Agent Trace local-hook audit has confirmed a false-positive path where `commit-msg` can add `Co-authored-by: SCE <sce@crocoder.dev>` for a human-only staged commit. Remaining work is to implement, document, and validate the narrow fix so trailer injection requires explicit staged SCE attribution.

## Success criteria

- The current false-positive risk is explicitly traced from pre-commit checkpoint generation through `commit-msg` trailer application, with code-level evidence for each decision point.
- If a false-positive path exists, `commit-msg` no longer adds the canonical SCE co-author trailer for human-only staged commits.
- Existing intended behavior remains intact: staged SCE-attributed commits still end with exactly one canonical trailer when the runtime gates pass.
- Regression coverage locks both the no-attribution case and the positive-attribution case.
- Current-state context is updated at the narrowest correct scope if implementation changes the documented contract or clarifies an edge case not already captured.

## Constraints and non-goals

- Preserve the existing no-git-wrapper Agent Trace architecture and canonical trailer string.
- Keep the fix narrowly scoped to staged-attribution detection and hook-policy behavior; do not broaden this plan into hosted reconciliation, post-commit persistence, or general attribution redesign.
- Do not rewrite human Git author/committer identity; only the commit-message trailer policy may change.
- Prefer code truth over existing context if documentation and implementation differ.
- Do not change unrelated hook UX or output wording unless required to keep tests and contracts accurate.

## Task stack

- [x] T01: `Trace staged-attribution gating end to end` (status:done)
  - Task ID: T01
  - Goal: Map the exact runtime path from staged diff collection to pre-commit checkpoint persistence to `commit-msg` gate evaluation, and determine whether human-only commits can still satisfy `has_staged_sce_attribution`.
  - Boundaries (in/out of scope): In - `cli/src/services/hooks.rs`, existing tests, and focused Agent Trace context docs tied to pre-commit and commit-msg behavior. Out - code changes beyond minimal instrumentation/tests needed to prove current behavior, hosted reconciliation, and post-commit dual-write flows except where they clarify the checkpoint contract.
  - Done when: the plan captures a clear verdict on whether a false-positive path exists, the specific cause if it does, and the exact implementation seam to change.
  - Verification notes (commands or checks): Review the pre-commit checkpoint builder/finalizer, staged-attribution presence check, and commit-msg runtime tests; add or identify a reproduction case that distinguishes SCE-attributed staged ranges from generic staged ranges.
  - Completed: 2026-03-17
  - Files changed: `cli/src/services/hooks.rs`, `cli/src/services/hooks/tests.rs`, `context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-pre-commit-staged-checkpoint.md`, `context/plans/agent-trace-coauthor-false-positive-guard.md`
  - Evidence: Code review traced `collect_pending_checkpoint` -> `finalize_pre_commit_checkpoint` -> `write_finalized_checkpoint` -> `staged_sce_attribution_present` -> `apply_commit_msg_coauthor_policy`; added `checkpoint_has_non_empty_ranges` plus `checkpoint_has_non_empty_ranges_treats_generic_staged_ranges_as_sce_attribution`; ran `nix develop -c sh -c 'cd cli && cargo test checkpoint_has_non_empty_ranges_treats_generic_staged_ranges_as_sce_attribution && cargo test pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors && cargo test commit_msg_policy_appends_canonical_trailer_once_when_allowed && cargo test commit_msg_policy_noops_without_staged_sce_attribution && cargo build'`
  - Notes: Verdict confirmed: a false-positive path exists today. `collect_pending_checkpoint` records generic staged diff ranges, `finalize_pre_commit_checkpoint` persists any non-empty staged ranges without an SCE-specific marker, and `staged_sce_attribution_present` returns true for any checkpoint file containing non-empty `files[].ranges`. The narrow fix seam for `T02` is the pre-commit checkpoint schema plus `staged_sce_attribution_present`/commit-msg gate evaluation so commit-msg keys off explicit SCE-attributed contribution rather than generic staged ranges.

- [x] T02: `Harden co-author gating against false positives` (status:done)
  - Task ID: T02
  - Goal: Implement the narrowest production fix so the canonical SCE trailer is added only when the staged checkpoint proves actual SCE-attributed contribution rather than merely any staged line range.
  - Boundaries (in/out of scope): In - hook runtime data model, checkpoint parsing/serialization, gating helpers, and targeted tests needed to preserve positive cases while blocking false positives. Out - broader attribution-schema redesign, hosted/remote flows, new persistence backends, or unrelated hook command routing changes.
  - Done when: a human-only staged commit path leaves the commit message unchanged, an SCE-attributed staged commit path still gets exactly one canonical trailer, and the implementation remains idempotent.
  - Verification notes (commands or checks): Run targeted Rust tests covering `apply_commit_msg_coauthor_policy`, `staged_sce_attribution_present`, and any checkpoint finalization/runtime helpers touched by the fix.
  - Completed: 2026-03-17
  - Files changed: `cli/src/services/hooks.rs`, `cli/src/services/hooks/tests.rs`, `context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-pre-commit-staged-checkpoint.md`, `context/plans/agent-trace-coauthor-false-positive-guard.md`
  - Evidence: Ran `nix develop -c sh -c 'cd cli && cargo test checkpoint_has_explicit_sce_attribution_requires_marker && cargo test checkpoint_has_explicit_sce_attribution_accepts_marked_staged_ranges && cargo test commit_msg_policy_noops_without_staged_sce_attribution && cargo test commit_msg_policy_appends_canonical_trailer_once_when_allowed && cargo test pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors && cargo build'`, `nix develop -c sh -c 'cd cli && cargo fmt --check && cargo clippy --all-targets --all-features'`, `nix run .#pkl-check-generated`, and `nix flake check`.
  - Notes: Added explicit `has_sce_attribution` to pre-commit checkpoint file entries, switched `commit-msg` gating to require that marker plus non-empty staged ranges, and treated this as a verify-only context sync that left root shared files unchanged while updating focused Agent Trace docs.

- [x] T03: `Sync context for corrected co-author semantics` (status:done)
  - Task ID: T03
  - Goal: Update focused current-state documentation so Agent Trace hook docs describe the corrected gating rule and any new checkpoint semantics introduced by T02.
  - Boundaries (in/out of scope): In - the narrowest relevant files under `context/sce/`, plus root shared files only if the implementation changes a repository-level contract or terminology. Out - broad documentation cleanup and historical narration.
  - Done when: no touched context file implies that generic staged ranges alone are sufficient for SCE co-author insertion, and any new required attribution marker/field is documented where future sessions will look first.
  - Verification notes (commands or checks): Review `context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-pre-commit-staged-checkpoint.md`, and root shared files in verify-only mode unless the change proves cross-cutting.
  - Completed: 2026-03-17
  - Files changed: `context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-pre-commit-staged-checkpoint.md`, `context/plans/agent-trace-coauthor-false-positive-guard.md`
  - Evidence: Verified focused Agent Trace docs now require explicit `has_sce_attribution = true` plus non-empty staged `ranges[]` for `commit-msg` trailer insertion, confirmed root shared files remained accurate in verify-only mode, then ran `nix run .#pkl-check-generated` and `nix flake check`.
  - Notes: This remained a verify-only context sync. Root shared files already matched the corrected co-author semantics, so no root-level edits were needed.

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Validate the false-positive fix end to end, confirm no temporary scaffolding remains, and leave the plan/context ready for normal implementation handoff or completion.
  - Boundaries (in/out of scope): In - final targeted test execution, lightweight repo validation required by this repo, and plan cleanup/status capture. Out - new feature work beyond the false-positive guard.
  - Done when: targeted hook tests pass, required repo-level validation passes for the touched surfaces, and the final state demonstrates both blocked false positives and preserved intended positive attribution behavior.
  - Verification notes (commands or checks): Run the narrowest relevant Rust hook test slice first, then the repo baseline validation for completed tasks (`nix run .#pkl-check-generated` and `nix flake check`) if touched files require it.
  - Completed: 2026-03-17
  - Files changed: `context/plans/agent-trace-coauthor-false-positive-guard.md`
  - Evidence: Ran `nix develop -c sh -c 'cd cli && cargo test checkpoint_has_explicit_sce_attribution_requires_marker && cargo test checkpoint_has_explicit_sce_attribution_accepts_marked_staged_ranges && cargo test commit_msg_policy_noops_without_staged_sce_attribution && cargo test commit_msg_policy_appends_canonical_trailer_once_when_allowed && cargo test pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors && cargo build'`, `nix run .#pkl-check-generated`, and `nix flake check`.
  - Notes: Validation confirmed the negative path stays blocked for generic human-only staged diffs, the positive path still appends exactly one canonical trailer when explicit staged SCE attribution is present, and no temporary task-local scaffolding was required for cleanup.

## Open questions

- None. Per user direction, the plan covers both audit and implementation of a fix if a false-positive path is confirmed.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test checkpoint_has_explicit_sce_attribution_requires_marker && cargo test checkpoint_has_explicit_sce_attribution_accepts_marked_staged_ranges && cargo test commit_msg_policy_noops_without_staged_sce_attribution && cargo test commit_msg_policy_appends_canonical_trailer_once_when_allowed && cargo test pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors && cargo build'` -> exit 0; targeted regression and build checks passed.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; repository flake checks evaluated and completed successfully.

### Temporary scaffolding

- No temporary scaffolding or debug-only artifacts were introduced for this fix, so no cleanup deletions were required.

### Context verification

- Verified final focused docs remain aligned with implementation in `context/sce/agent-trace-commit-msg-coauthor-policy.md` and `context/sce/agent-trace-pre-commit-staged-checkpoint.md`.
- Verified root shared files remained accurate in verify-only mode: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md`.

### Success-criteria verification

- [x] False-positive risk traced from pre-commit checkpoint generation through `commit-msg` trailer application -> captured in `T01` evidence and notes.
- [x] Human-only staged commits no longer receive the canonical SCE co-author trailer -> confirmed by `commit_msg_policy_noops_without_staged_sce_attribution` and explicit-marker checkpoint gating tests.
- [x] Intended positive behavior remains intact for staged SCE-attributed commits -> confirmed by `checkpoint_has_explicit_sce_attribution_accepts_marked_staged_ranges` and `commit_msg_policy_appends_canonical_trailer_once_when_allowed`.
- [x] Regression coverage locks both negative and positive cases -> confirmed by the targeted Rust hook test slice recorded above.
- [x] Current-state context reflects corrected gating semantics -> confirmed by focused Agent Trace docs and verify-only root-context pass.

### Residual risks

- None identified within the scoped false-positive guard change.
