# Plan: git-hook-append-and-atomic-asset-swap

## Change summary

Two defects in the setup install write path, both surfaced by review of the
`non-destructive-setup-install` change. First, every setup write — config assets
and required git hooks alike — unlinks the destination file before renaming the
staged replacement over it (`cli/src/services/setup/mod.rs:1408`, `:979`). The
staging file always lives in the destination's own directory, so `fs::rename`
already replaces atomically and the unlink buys nothing; it only opens a window
in which the file does not exist, and on a rename failure the error path deletes
the staging file too, leaving neither copy. That was tolerable when every
installed file was SCE-owned and regenerable, but `.claude/settings.json` and
`.opencode/opencode.json` are now merge targets holding the user's
`permissions`, `env`, `model`, and `mcp` keys, which setup cannot reconstruct.
This plan drops both pre-deletes and relies on atomic rename.

Second, `sce setup --hooks` still replaces `.git/hooks/pre-commit`,
`commit-msg`, and `post-commit` wholesale, so a repository already running husky,
lefthook, or a hand-written hook loses it. The predecessor plan named this as
out of scope and left the design undecided. This plan implements the append
variant the user chose: each canonical hook payload is delimited by SCE managed
block markers, and install replaces only that block — creating the full script
when no hook exists, replacing the block in place when one carries it, and
appending the block after the existing content when a foreign hook is found. The
foreign hook keeps its shebang, its content, and its position first in the run
order; SCE runs last, which is what `commit-msg` trailer insertion needs anyway.
`sce doctor` moves from byte-exact hook comparison to the same SCE-fragment
comparison the two JSON merge targets already use, so a legitimately extended
hook is not reported as drift.

This extends the existing non-destructive install policy to git hooks and
corrects the swap choreography that policy is built on. It preserves every
existing outcome vocabulary (`Installed`/`Updated`/`Skipped`,
`[PASS]`/`[FAIL]`/`[MISS]`, `Missing`/`Current`/`Stale`/`Unknown`) and adds no
new one.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: A rename failure while installing a setup config asset leaves the previous file's content intact on disk, and leaves no staging artifact.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — the injected-`rename_fn`-failure test asserts the pre-existing destination bytes are unchanged after the error, in addition to the existing path-naming, recovery-guidance, and staging-cleanup assertions.
- [x] AC2: A rename failure while installing a required git hook leaves the previous hook file's content and executable bit intact.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — an injected hook `rename_fn` failure asserts the seeded prior hook bytes and mode survive.
- [x] AC3: Running `sce setup --hooks` in a repository whose `pre-commit` is a foreign script preserves that script byte-for-byte at the head of the file and appends the SCE managed block after it, with the file executable.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — an integration test seeds a foreign `pre-commit`, installs, and asserts the original content is a prefix of the result, the managed block follows it, and the file is executable.
- [x] AC4: Re-running setup against a hook that already carries the current SCE managed block reports `skipped` and leaves the file byte-identical, whether or not foreign content sits above the block.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — a two-run test asserts identical bytes and `RequiredHookInstallStatus::Skipped` on the second run for both the foreign-plus-block and block-only shapes.
- [x] AC5: A hook carrying a pre-marker SCE payload is recognized as SCE-owned and replaced wholesale with the marker form, without preserving any of the old payload as if it were foreign content.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::hook_merge` — a unit test feeds the legacy payload and asserts the output equals the canonical marker-form document.
- [x] AC6: `sce doctor` reports a hook carrying foreign content plus a current SCE block as current, reports it stale when the block is missing or outdated, and `sce doctor --fix` repairs it by rewriting only the block while preserving the foreign content.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` — filesystem-backed tests covering current-with-foreign-content, drifted block, and post-`--fix` re-inspection with the foreign content asserted intact.
- [x] AC7: When a foreign hook's last effective line is a zero-indent `exec` or `exit`, setup still installs the block and reports a deterministic advisory naming that hook, because the appended block would not run.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — a test seeds a foreign hook ending in `exec some-tool "$@"`, asserts the advisory is present in the install result, and asserts no advisory for a foreign hook ending in an ordinary command.

### Full validation

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets`
- `nix run .#pkl-check-generated`
- `nix flake check`
- `sh -n cli/assets/hooks/pre-commit && sh -n cli/assets/hooks/commit-msg && sh -n cli/assets/hooks/post-commit`

### Context sync

- `context/sce/setup-no-backup-policy-seam.md` — the per-file choreography is no longer "stage, remove destination, swap" but "stage, atomically swap"; required-hook install is no longer remove-and-replace but a managed-block merge, making it a second merge-target family alongside the two JSON configs.
- `context/sce/setup-githooks-install-contract.md` — "Preservation and replacement policy" and "Rollback guarantees" both state remove-before-swap and wholesale replacement.
- `context/sce/setup-githooks-install-flow.md` — "Staged write and remove-and-replace behavior" and the per-hook outcome definitions.
- `context/sce/setup-githooks-hook-asset-packaging.md` — the canonical templates now carry managed-block markers and no longer `exec`.
- `context/sce/doctor-human-text-contract.md` — hook rows are decided by SCE-fragment currency rather than byte-exact `sha256`.
- `context/patterns.md` — the setup-install execution pattern (currently "remove only that destination file if one already exists, then swap") and the required-hook execution pattern (currently "remove-and-replace behavior that removes existing hooks before swapping staged content").
- `context/architecture.md` — the `cli/src/services/setup/mod.rs` paragraph describing install-engine and required-hook choreography.
- `context/overview.md` — the setup-service paragraph stating remove-and-replace for hooks.
- `context/glossary.md` — the SCE managed block term, if the glossary carries the merge-target/ownership-marker vocabulary.
- `context/context-map.md` — annotation updates for any of the above whose subject line changes.

This change makes SCE a bounded in-place editor of user-authored git hooks,
which the synchronization decision gate may judge a qualifying system-wide
decision.

## Constraints and non-goals

- **In scope:** `cli/src/services/setup/mod.rs` (config-asset and required-hook install choreography), a new pure hook-merge module under `cli/src/services/setup/`, `cli/assets/hooks/{pre-commit,commit-msg,post-commit}`, `cli/src/services/doctor/inspect.rs` hook content inspection, and the durable-context files named under Context sync.
- **Out of scope:** `core.hooksPath` resolution, which already works — `git rev-parse --git-path hooks` returns the configured path (verified against git 2.54.0), contradicting the predecessor plan's open question.
- **Out of scope:** the two JSON merge targets and `config_merge.rs`. Their merge semantics are unchanged; only the swap step beneath them changes.
- **Out of scope:** a `<name>.d/` dispatcher directory, hook-manager detection, or cooperation protocol with husky/lefthook.
- **Out of scope:** repairing the stale `## Validation Report` in `context/plans/non-destructive-setup-install.md`.
- **Constraints:** No backup artifacts and no backup-based rollback (`context/sce/setup-no-backup-policy-seam.md`). No new crate dependency; the hook merge is byte/string work over the existing asset bytes. Unit tests stay filesystem-free (`context/patterns.md`, "Unit testing in Nix sandbox"): the merge is pure and unit-tested, install and doctor behavior belongs in integration tests. Hook payloads stay POSIX `sh`. Existing outcome vocabularies are not extended.
- **Non-goal:** a general shell-script merge engine, or parsing shell to determine reachability. The unreachable-block advisory is a last-effective-line heuristic, not an analysis.
- **Non-goal:** guaranteeing the SCE block survives a third-party hook manager reinstalling its own hooks. This is cooperative.

## Assumptions

- SCE ownership of an existing hook is matched structurally, mirroring the `./plugins/sce-` precedent in `config_merge.rs`: a file carrying the managed-block markers is owned within that block, and a pre-marker file containing the canonical guidance URL (`https://sce.crocoder.dev/docs/getting-started#install-cli`) is owned wholesale. Any other file is foreign and preserved.
- The appended block runs last on purpose. `commit-msg` must be last so the SCE trailer lands on the final message, and `pre-commit`/`post-commit` are order-indifferent.
- The block invokes `sce hooks <name> "$@"` and propagates its status by ordinary exit rather than `exec`, so a block appended after SCE's by some later tool would still run. The canonical fresh-install script uses the identical block.
- The block relies on the invoking hook's top-level `"$@"`. A foreign hook that `shift`s its arguments before the block changes what SCE receives; this is accepted rather than defended against.
- Removing the pre-delete is safe on Windows: `std::fs::rename` replaces an existing destination file there as it does on Unix.

## Task stack

- [x] T01: `Replace setup destinations by atomic rename` (status:done)
  - Task ID: T01
  - Goal: Setup never unlinks a destination before renaming staged content over it, so a failed swap leaves the previous file intact.
  - Boundaries (in/out of scope): In — delete the `if destination.exists() { fs::remove_file(...) }` block in `install_single_asset_with_rename`, delete the `remove_existing_install_target(&hook_path)` call and the now-unused `remove_existing_install_target` helper in `install_single_required_hook_with_rename`, collapse that function's two rename branches into one, and update the existing rename-failure tests to assert the prior file survives. Out — hook content merging, doctor, and any change to staging-path allocation or recovery-guidance text.
  - Dependencies: none
  - Done when: Neither install path calls `remove_file`/`remove_dir_all` on a destination before renaming; `remove_existing_install_target` is gone; the config-asset rename-failure test asserts the seeded prior content is unchanged after the error alongside its existing assertions; an equivalent hook rename-failure test asserts prior hook bytes and mode survive; recovery guidance and staging cleanup behavior are unchanged.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets`.
  - Implementation evidence: `cli/src/services/setup/mod.rs` — removed the pre-rename `fs::remove_file` in `install_single_asset_with_rename`; collapsed `install_single_required_hook_with_rename`'s install/update branches into one rename call (status and recovery-guidance context still chosen from whether a prior hook existed); deleted `remove_existing_install_target`; added `pub(super) install_required_git_hooks_with_rename` (mirroring the existing `install_embedded_setup_assets_with_rename` seam) so hook rename failures are test-injectable; updated `install_cleans_up_staging_and_reports_asset_path_on_rename_failure` to seed prior destination content and assert it survives; added `hook_install_leaves_prior_hook_intact_on_rename_failure` asserting prior hook bytes, executable mode, and staging cleanup survive a rename failure.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 38 passed, 0 failed. `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` — clean, no warnings.
  - Deviations/assumptions: Exposed `install_required_git_hooks_with_rename` (crate-internal `pub(super)`) as a minimal, in-convention test seam; no other deviation from the reviewed scope.

- [x] T02: `Delimit canonical hook payloads with SCE managed block markers` (status:done)
  - Task ID: T02
  - Goal: Each canonical hook template carries its SCE logic inside stable start/end markers and exits by status propagation instead of `exec`, so the same block can be embedded in a foreign hook.
  - Boundaries (in/out of scope): In — `cli/assets/hooks/{pre-commit,commit-msg,post-commit}`: wrap the missing-CLI guidance plus the `sce hooks <name>` invocation in `# >>> sce managed block (do not edit) >>>` / `# <<< sce managed block <<<`, replace `exec sce hooks ...` with a status-propagating invocation, and keep the shebang, `set -eu`, the branded guidance text, the terminal-only ANSI policy, and `post-commit`'s `origin`/`--remote-url` behavior unchanged. Out — the merge module, install wiring, and doctor; byte-exact comparison still governs install/doctor at the end of this task, so a pre-existing hook simply reports `Updated` once.
  - Dependencies: T01
  - Done when: All three templates pass `sh -n`; each contains exactly one marker pair; no template invokes `exec`; a hook installed fresh into an empty hooks directory still blocks nothing when `sce` is absent and still forwards arguments and exit status when it is present.
  - Verification notes (commands or checks): `sh -n cli/assets/hooks/pre-commit && sh -n cli/assets/hooks/commit-msg && sh -n cli/assets/hooks/post-commit`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; manual run of an installed `post-commit` with and without `sce` on `PATH`.
  - Implementation evidence: `cli/assets/hooks/pre-commit`, `cli/assets/hooks/commit-msg`, `cli/assets/hooks/post-commit` — wrapped the missing-CLI guidance plus the `sce hooks <name>` invocation in `# >>> sce managed block (do not edit) >>>` / `# <<< sce managed block <<<`; replaced each `exec sce hooks ...` with an explicit invocation followed by `status=$?; exit "$status"`; in `post-commit`, moved the `remote_url="$(git remote get-url origin ...)"` computation inside the managed block so it travels with the block when a future foreign-hook append embeds only the block; shebang, `set -eu`, branded guidance text, terminal-only ANSI policy, and `post-commit`'s `--remote-url` behavior are unchanged outside these edits.
  - Verification outcome: `sh -n` on all three templates — clean. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 38 passed, 0 failed. Manual check: each file contains exactly 2 marker-comment lines (one pair) and zero `exec` invocations. Manually ran an installed `post-commit` and `pre-commit` in a scratch git repo with `sce` absent from `PATH` (exit 0, guidance printed to stderr) and with a fake `sce` present (arguments forwarded correctly, non-zero exit status propagated).
  - Deviations/assumptions: Placed `remote_url` computation inside the managed block rather than before it, as an ordinary local implementation choice needed so the `--remote-url` behavior is preserved when T04 later appends only the block (not the whole file) to a foreign `post-commit`; no other deviation from the reviewed scope.

- [x] T03: `Add pure hook managed-block merge module` (status:done)
  - Task ID: T03
  - Goal: A filesystem-free module computes the bytes to install for a hook from the existing file's bytes and the canonical template, mirroring `config_merge.rs`.
  - Boundaries (in/out of scope): In — new `cli/src/services/setup/hook_merge.rs` exposing the marker constants, the legacy-ownership marker, a `merge_or_create_hook(existing: Option<&[u8]>, canonical: &[u8]) -> Result<HookMerge>` returning the merged bytes plus a classification (created / managed-block replaced / appended to foreign hook / already current) and an unreachable-block advisory flag, plus the last-effective-line heuristic (last non-blank, non-comment line at zero indentation starting with `exec ` or `exit`), and its unit tests. Out — install and doctor wiring; no filesystem access anywhere in the module.
  - Dependencies: T02
  - Done when: The module is pure and its tests use no temp directories; unit tests cover create-from-absent, replace-in-place preserving surrounding foreign content, append-to-foreign preserving the original bytes as an exact prefix, legacy pre-marker payload replaced wholesale, idempotence across two merges producing identical bytes, a foreign hook with unbalanced or partial markers failing with a deterministic error naming the hook, and the advisory firing on a trailing zero-indent `exec`/`exit` but not on an indented one or on an ordinary final command.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::hook_merge`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets`.
  - Implementation evidence: `cli/src/services/setup/hook_merge.rs` (new) — `MANAGED_BLOCK_START`/`MANAGED_BLOCK_END` marker constants; `HookMergeKind` (`Created`/`ManagedBlockReplaced`/`AppendedToForeign`/`AlreadyCurrent`) and `HookMerge { bytes, kind, unreachable_block_advisory }`; `merge_or_create_hook(existing, canonical, hook_name)` locating the marker pair by exact-line match, splicing the canonical block in place when the existing block differs, replacing wholesale when the legacy guidance-URL marker is found with no block, appending the canonical block after the existing bytes (kept as an exact prefix) otherwise, and failing with a `hook_name`-naming error on an unbalanced/partial marker pair; `ends_with_unreachable_control_flow` implementing the last-effective-line heuristic. `cli/src/services/setup/mod.rs` — added `pub(crate) mod hook_merge;` alongside `config_merge`.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::hook_merge` — 13 passed, 0 failed. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 51 passed, 0 failed (no regression from the new module or its registration). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` — clean, no warnings.
  - Deviations/assumptions: Added a `hook_name: &str` third parameter to `merge_or_create_hook`, beyond the two-argument signature sketched in Boundaries, because the done check requires the unbalanced-marker error to name the offending hook and the module has no other source for that name; mirrors `config_merge.rs`'s own `source_path` parameter on `merge_or_create_claude_settings`/`merge_or_create_opencode_config`, so it stays in convention. `merge_or_create_hook`, `HookMerge`, and `HookMergeKind` are marked `#[allow(dead_code)]` since T04 is the first caller, matching the existing forward-declared-item precedent in this file (`RequiredHookAsset`, `get_required_hook_asset`). No other deviation from the reviewed scope.

- [x] T04: `Install required hooks through the managed-block merge` (status:done)
  - Task ID: T04
  - Goal: `sce setup --hooks` preserves a foreign hook and appends the SCE block, keeps `Installed`/`Updated`/`Skipped` accurate against the block rather than the whole file, and surfaces the unreachable-block advisory.
  - Boundaries (in/out of scope): In — `install_single_required_hook_with_rename` reads the existing hook, computes bytes via `hook_merge`, and stages those instead of `hook_asset.bytes`; the skip decision compares merged bytes plus executable bit against the file on disk; the advisory is carried on `RequiredHookInstallResult` and rendered in the hook section of setup output; integration tests for foreign-hook append, idempotent rerun, legacy upgrade, and the advisory. Out — doctor inspection, which still compares bytes and will report drift until T05.
  - Dependencies: T03
  - Done when: A foreign `pre-commit` survives as an exact prefix with the block appended and the file executable; a second run reports `Skipped` with identical bytes for both foreign-plus-block and block-only shapes; a legacy pre-marker SCE hook upgrades to the marker form; a foreign hook ending in a zero-indent `exec` installs and reports the advisory; existing hooks-path resolution, write-permission probes, recovery guidance, and no-backup behavior are unchanged.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; manual `sce setup --hooks` in a temp checkout seeded with a husky-style `pre-commit`, then `git commit` to confirm both the foreign hook and the SCE block run.
  - Implementation evidence: `cli/src/services/setup/mod.rs` — the install/skip wiring (`install_single_required_hook_with_rename` reading the existing hook, calling `hook_merge::merge_or_create_hook`, staging `merge.bytes`, comparing merged bytes plus the executable bit for the skip decision, and carrying `unreachable_block_advisory` on `RequiredHookInstallResult` with its advisory line in `format_required_hook_install_success_message`) was already present on this branch from prior work on this task stack; this task's remaining scope was the missing integration coverage, added as four new tests in `cli/src/services/setup/mod.rs`'s `tests` module: `foreign_pre_commit_hook_keeps_its_content_and_gains_the_sce_block` (foreign hook survives as an exact prefix, block appended, file executable, `Updated`, no advisory), `rerunning_hook_install_is_idempotent_for_block_only_and_foreign_plus_block_shapes` (second run `Skipped` with identical bytes for both the block-only `pre-commit` and a foreign-plus-block `commit-msg`), `legacy_pre_marker_hook_upgrades_to_the_managed_block_form` (a pre-marker guidance-URL hook upgrades to the exact canonical bytes, stays executable), and `foreign_hook_ending_in_exec_installs_the_block_and_reports_the_advisory` (a foreign `pre-commit` ending in `exec` installs the block and sets the advisory, while a sibling foreign `commit-msg` ending in an ordinary command does not).
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 55 passed, 0 failed (51 pre-existing plus the 4 new tests). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` — clean, no warnings.
  - Deviations/assumptions: The install/skip wiring itself was not written by this task — it was found already implemented and uncommitted on the branch when this task's review began, matching the Boundaries description exactly (merge-based staging, executable-bit-aware skip comparison, advisory field and rendering). This task's actual work was limited to closing the test gap the Done-when checks require; no production code was changed. The manual `sce setup --hooks` / `git commit` check in Verification notes was not run in this session — the four automated integration tests directly assert the same properties (foreign-prefix preservation, executable bit, idempotent rerun, legacy upgrade, advisory) that the manual check exists to spot-confirm.

- [x] T05: `Inspect hook content by SCE managed block currency` (status:done)
  - Task ID: T05
  - Goal: `sce doctor` decides a hook's content state from its SCE block rather than whole-file bytes, so a hook a repository has extended reports `[PASS]`.
  - Boundaries (in/out of scope): In — `inspect_hook_content_state` and `inspect_hook_content_state_without_problem` in `cli/src/services/doctor/inspect.rs` compare via a `hook_merge` currency predicate (merging canonical into the file is a no-op) instead of `bytes == expected_hook.bytes`; `HookReadFailed` handling, remediation text, and the `Missing`/`Current`/`Stale`/`Unknown` vocabulary are preserved; doctor tests for current-with-foreign-content, drifted block, and `--fix` repair. Out — new problem kinds, new status vocabulary, and any change to `--fix` plumbing, which already reuses canonical setup hook installation.
  - Dependencies: T04
  - Done when: A hook with foreign content plus a current block reports `Current`; deleting or corrupting the block reports `Stale`; `sce doctor --fix` restores the block and a follow-up inspection reports `Current` with the foreign content intact; no doctor status string, section order, or problem taxonomy changes.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::`; manual `sce doctor` / `sce doctor --fix` in a temp checkout with an extended `pre-commit`.
  - Implementation evidence: `cli/src/services/doctor/inspect.rs` — `inspect_hook_content_state_without_problem` and `inspect_hook_content_state` (the latter unreachable dead code, since its only caller `inspect_repository_hooks` has no callers) now classify hook content via `hook_merge::merge_or_create_hook(Some(bytes), canonical, hook_name)`: `Current` when merging is a no-op (`merge.bytes == bytes`), `Stale` otherwise, including an unbalanced/partial managed block (previously `Unknown`, now `Stale` so `--fix` repairs it); `HookReadFailed` handling and the `Missing`/`Unknown` cases for a read failure or unrecognized hook name are unchanged. Added a shared private helper `hook_managed_block_content_state` used by both functions. Added three filesystem-backed tests: `hook_with_foreign_content_and_current_block_reports_current`, `hook_with_drifted_managed_block_reports_stale`, `fix_repairs_drifted_hook_content_while_preserving_foreign_content` (drifts a foreign-plus-block hook, reinstalls via `install_required_git_hooks`, and asserts the foreign prefix survives and the state returns to `Current`). `cli/src/services/hooks/lifecycle.rs` — `inspect_hook_content_state` (the function that actually feeds the live `HookContentStale` problem and `--fix` eligibility, found during investigation to be a separate near-duplicate of the doctor/inspect.rs function of the same name) updated with the identical currency-based classification, since the acceptance criteria do not hold end-to-end without this change.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` — 9 passed, 0 failed. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 55 passed, 0 failed (no regression). `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (full suite) — 230 passed, 0 failed. `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` — clean, no warnings.
  - Deviations/assumptions: Updated `cli/src/services/hooks/lifecycle.rs::inspect_hook_content_state` beyond the file named in Boundaries, because that function — not the one in `doctor/inspect.rs` — is the one reachable from `sce doctor`'s actual problem list and `--fix` eligibility; the `doctor/inspect.rs` function of the same name is dead code. Without this, AC6's `--fix` repair and stale-detection behavior would not hold in the live CLI path. No other deviation from the reviewed scope; the manual `sce doctor` / `sce doctor --fix` check in Verification notes was not run in this session — the three automated tests directly assert the same current/stale/repair properties it exists to spot-confirm.

## Open questions

- The appended block is cooperative, not authoritative. husky and lefthook rewrite their own hooks on `npm install` or `lefthook install`, which drops the SCE block silently — SCE stops running until the next `sce setup --hooks`, and nothing tells the user. The predecessor plan reached the same conclusion and proposed a `<name>.d/` dispatcher instead, which loses the block just as easily. If silent breakage is the concern, the fix is `sce doctor` catching it, which T05 gives you for free; if it is not a concern, this note can be dropped.
- The unreachable-block advisory (AC7, T03's heuristic) exists because appending after a foreign hook that ends in `exec` or `exit` produces a block that never runs, which is otherwise invisible. It is deliberately narrow — last effective line, zero indentation — so it will miss an early `exit 0` inside a conditional. A heuristic that catches some cases and not others may be worse than none; say if you would rather T03 and AC7 be dropped.
- `context/plans/non-destructive-setup-install.md` still records `**Status:** failed` with a `Retry` instruction, from a `cargo fmt --check` failure that no longer reproduces (`cargo fmt --manifest-path cli/Cargo.toml -- --check` exits 0 on the current tree). That plan covers the code T01 modifies, so its report will stay misleading unless someone reruns `/validate` on it. Explicitly out of scope here.

## Validation Report

**Status:** validated  
**Date:** 2026-08-07

### Commands run

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (230 passed, 0 failed)
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` -> exit 0 (clean, no warnings)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 101 files)
- `nix flake check` -> exit 0 (all checks passed, including `checks.x86_64-linux.cli-fmt`; previously failed because `cli/src/services/setup/hook_merge.rs` was untracked by git, now staged and visible to the Nix-sandboxed build)
- `sh -n cli/assets/hooks/pre-commit && sh -n cli/assets/hooks/commit-msg && sh -n cli/assets/hooks/post-commit` -> exit 0 (no syntax errors)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::hook_merge` -> exit 0 (13 passed, 0 failed)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` -> exit 0 (9 passed, 0 failed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Rename failure preserves prior config-asset content and leaves no staging artifact -> `hook_install_leaves_prior_hook_intact_on_rename_failure` and the updated asset rename-failure test pass in the `setup::` suite.
- [x] AC2: Rename failure preserves prior hook content and executable bit -> `hook_install_leaves_prior_hook_intact_on_rename_failure` passes.
- [x] AC3: Foreign `pre-commit` preserved as exact prefix with block appended, executable -> `foreign_pre_commit_hook_keeps_its_content_and_gains_the_sce_block` passes.
- [x] AC4: Idempotent rerun reports `Skipped` with identical bytes for both shapes -> `rerunning_hook_install_is_idempotent_for_block_only_and_foreign_plus_block_shapes` passes.
- [x] AC5: Legacy pre-marker payload replaced wholesale with marker form -> `legacy_pre_marker_hook_upgrades_to_the_managed_block_form` (integration) and `legacy_pre_marker_payload_is_replaced_wholesale` (`setup::hook_merge` unit test) pass.
- [x] AC6: `sce doctor` reports current/stale by block currency and `--fix` repairs while preserving foreign content -> `hook_with_foreign_content_and_current_block_reports_current`, `hook_with_drifted_managed_block_reports_stale`, and `fix_repairs_drifted_hook_content_while_preserving_foreign_content` pass in `doctor::`.
- [x] AC7: Unreachable-block advisory fires only on trailing zero-indent `exec`/`exit` -> `foreign_hook_ending_in_exec_installs_the_block_and_reports_the_advisory` (integration) and the `advisory_*` unit tests in `setup::hook_merge` pass.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
