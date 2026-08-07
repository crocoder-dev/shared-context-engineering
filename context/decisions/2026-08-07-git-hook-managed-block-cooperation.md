# Decision: SCE git hooks are a bounded in-place editor, not an exclusive owner

Date: 2026-08-07
Status: Accepted
Plan: `context/plans/git-hook-append-and-atomic-asset-swap.md`
Task: T02, T03, T04

## Context

`sce setup --hooks` installs `pre-commit`, `commit-msg`, and `post-commit`. The
predecessor plan (`non-destructive-setup-install`) replaced these files
wholesale, which is safe only as long as SCE is the sole author of every hook it
touches. That assumption breaks the moment a repository already runs husky,
lefthook, or a hand-written hook: a wholesale install silently destroys that
script. The predecessor plan named this out of scope and left the resolution
undecided (see its still-open question, carried into this plan's Open
questions). This plan had to decide how SCE coexists with a hook it does not
own, and the choice determines whether `sce setup --hooks` is safe to run in an
arbitrary repository.

## Decision

SCE treats an existing hook file as either SCE-owned or foreign, never as
something to overwrite by default. Each canonical hook payload lives inside a
delimited SCE managed block (`# >>> sce managed block (do not edit) >>>` /
`# <<< sce managed block <<<`). Ownership is decided structurally, not by
trusting the whole file:

- A hook already carrying a balanced marker pair is SCE-owned within that
  block; only the block is replaced or refreshed, and any surrounding content
  is left untouched.
- A marker-free hook containing the legacy pre-marker guidance URL (from before
  the marker pair existed) is recognized as SCE-owned wholesale and replaced
  entirely.
- Any other hook is foreign. Its bytes are preserved as an exact byte prefix,
  and the canonical SCE block is appended after them, so the foreign hook keeps
  its shebang, content, and first-run position; SCE always runs last.

This computation is pure (`cli/src/services/setup/hook_merge.rs::merge_or_create_hook`)
and is wired into install (`cli/src/services/setup/mod.rs`,
`install_single_required_hook_with_rename`), so `Installed`/`Updated`/`Skipped`
are now decided against the merged bytes plus the executable bit rather than
the canonical asset's raw bytes.

## Rationale

Appending after existing content, rather than requiring a dispatcher directory
or hook-manager detection, needs no cooperation protocol and works uniformly
whether the existing hook is husky-managed, lefthook-managed, or hand-written.
Running SCE last is not an arbitrary ordering choice: `commit-msg` trailer
insertion must see the final message, so SCE has to observe whatever the
foreign hook already did to it. Structural ownership detection (the marker
pair, then the legacy marker) mirrors the precedent already set for the two
JSON merge targets (`config_merge.rs`'s `./plugins/sce-` and
`run-sce-or-show-install-guidance.sh` ownership markers), so the same mental
model — "match markers, not full-file identity" — now applies to every merge
target SCE writes into.

## Alternatives considered

- **Wholesale replacement (status quo)** — simplest, but destroys any
  co-installed hook manager's script; ruled out as the defect this plan exists
  to fix.
- **`<name>.d/` dispatcher directory** — the predecessor plan's proposal.
  Requires every hook manager to cooperate with a dispatch convention SCE
  invents, and a manager that reinstalls its own hook still drops SCE from the
  chain just as easily as today's approach. Rejected as added complexity with
  no corresponding robustness gain.
- **Hook-manager detection and cooperation protocol** — explicitly out of
  scope for this plan; open-ended and highly manager-specific.

## Compatibility and risks

- **Cooperative, not authoritative**: a hook manager (husky, lefthook) that
  rewrites its own hooks on `npm install` / `lefthook install` will drop the
  SCE block silently, and SCE stops running until the next `sce setup --hooks`.
  Nothing currently alerts the user in-band; `sce doctor` catching this drift
  is a stated goal of a later task (T05) in the same plan, not this decision.
- **Unreachable appended block**: appending after a foreign hook whose last
  effective line is a zero-indent `exec`/`exit` produces a block that never
  runs. Mitigated by a narrow last-effective-line heuristic that surfaces a
  named advisory rather than silently installing a dead block; the heuristic
  deliberately does not parse shell, so it can miss an early conditional
  `exit`.
- **Argument propagation**: the appended block relies on the invoking hook's
  top-level `"$@"`; a foreign hook that `shift`s its arguments before the block
  changes what SCE receives. Accepted rather than defended against.

## Guardrails

- Ownership detection is structural only (exact marker-line match, or the
  fixed legacy guidance-URL substring) — never a heuristic guess at whether a
  file "looks like" an SCE hook.
- The merge computation stays pure and filesystem-free
  (`cli/src/services/setup/hook_merge.rs`); all I/O and swap choreography stay
  in the install seam.
- No new hook-manager-specific protocol, detection, or dispatcher directory is
  introduced by this decision.

## Consequences

- `sce setup --hooks` is now safe to run in a repository that already has
  husky, lefthook, or a hand-written hook installed: that hook survives, and
  SCE's logic runs alongside it.
- The existing outcome vocabulary (`Installed`/`Updated`/`Skipped`) and the
  no-backup atomic-swap policy are preserved unchanged; only what counts as
  "current" changed, from whole-file byte identity to managed-block currency.
- `sce doctor`'s hook inspection has not yet moved to the same block-currency
  model as of this decision (T04); until the follow-up task lands, doctor may
  report drift on a hook this decision considers current.

## Follow-up

- T05 in the same plan (`Inspect hook content by SCE managed block currency`)
  moves `sce doctor` from whole-file byte comparison to the same block-currency
  predicate this decision establishes for install, so a legitimately extended
  hook reports `[PASS]` instead of drift.

## References

- Plan: [`git-hook-append-and-atomic-asset-swap.md`](../plans/git-hook-append-and-atomic-asset-swap.md)
- Task: T02, T03, T04
- Current-state context: [`setup-githooks-install-contract.md`](../sce/setup-githooks-install-contract.md), [`setup-githooks-install-flow.md`](../sce/setup-githooks-install-flow.md), [`setup-no-backup-policy-seam.md`](../sce/setup-no-backup-policy-seam.md), [`setup-githooks-hook-asset-packaging.md`](../sce/setup-githooks-hook-asset-packaging.md)
- Evidence: `cli/src/services/setup/hook_merge.rs` unit tests; `cli/src/services/setup/mod.rs` integration tests `foreign_pre_commit_hook_keeps_its_content_and_gains_the_sce_block`, `rerunning_hook_install_is_idempotent_for_block_only_and_foreign_plus_block_shapes`, `legacy_pre_marker_hook_upgrades_to_the_managed_block_form`, `foreign_hook_ending_in_exec_installs_the_block_and_reports_the_advisory` (`./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 55 passed, 0 failed)
