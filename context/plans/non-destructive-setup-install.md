# Plan: non-destructive-setup-install

## Change summary

`sce setup --claude|--opencode|--pi|--all` currently destroys everything in the
target integration directory. `install_assets_for_concrete_target_with_rename`
(`cli/src/services/setup/mod.rs:1192`) stages the embedded SCE assets into a
temporary root, calls `remove_existing_install_target` on the whole `.claude/`,
`.opencode/`, or `.pi/` directory, then renames staging into place. A repository
whose `.claude/` holds the user's own skills, agents, commands,
`settings.local.json`, or `CLAUDE.md` loses all of it on a routine setup run. The
two generated JSON configs (`.claude/settings.json`, `.opencode/opencode.json`)
are the same problem one level down: they are written whole, so a user's
`permissions`, `env`, `model`, `mcp`, or non-SCE hook entries are replaced by the
SCE-only document.

This plan replaces the directory-level remove-and-replace policy with per-asset
installation plus catalog-derived pruning of SCE-owned paths, and adds JSON-aware
merging for the two generated config files so SCE-owned fragments are installed
into the user's document instead of over it. It preserves the existing swap
choreography (stage, then atomic rename) at file granularity, the existing
no-backup policy, and the existing optional-workflow deselection semantics — the
latter moves from "the whole tree is rebuilt" to "unselected catalog paths are
pruned". `sce doctor` integration checks are realigned in the same change, since
byte-exact `sha256` comparison stops being the right check for a merged file.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: Running setup for a target leaves every file in that target directory
  that SCE does not own exactly as it was — contents, mode, and mtime — including
  files nested inside SCE-owned parent directories such as
  `.claude/skills/my-own-skill/SKILL.md`.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — the setup integration tests seed a target directory with user-owned files at top level, inside `skills/`, and inside `commands/`, run install, and assert every seeded file survives byte-identical.
- [x] AC2: Running setup twice with an optional workflow selected and then
  deselected leaves no file of the deselected workflow on disk, and still leaves
  every unrelated file intact.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — a test installs with `brownfield` selected, reinstalls with an empty selection, and asserts the brownfield command file and skill directory are gone while a sibling user-owned skill directory remains.
- [x] AC3: Installing into an existing `.claude/settings.json` that carries user
  keys (`permissions`, `env`) and a user-authored hook entry yields a document
  that still carries those keys and that hook entry, plus exactly one current copy
  of each SCE hook entry, with no duplicate SCE entries after repeated runs.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — merge tests assert key preservation, SCE-entry replacement, and idempotence across two consecutive installs.
- [x] AC4: Installing into an existing `.opencode/opencode.json` that carries user
  keys and a user plugin path yields a document retaining both, with the canonical
  SCE plugin paths present exactly once and no stale SCE plugin path left behind.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — merge tests assert user-key and user-plugin preservation plus SCE plugin path reconciliation.
- [x] AC5: `sce doctor` reports `[PASS]` for a target whose merged JSON configs
  carry extra user content, and reports drift only when an SCE-owned fragment is
  missing or stale.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` plus a manual run of `sce doctor` in a checkout whose `.claude/settings.json` has a user `permissions` block — the `Claude` integration group shows `[PASS]`.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/sce/setup-no-backup-policy-seam.md` — this file currently states
  directory-level remove-and-replace as the unified policy for both config
  install and hook install. It must describe per-asset install, catalog-derived
  pruning, JSON merge targets, and the fact that the policy now differs between
  config install and required-hook install.
- `context/sce/setup-repo-local-config-bootstrap.md` — the optional-workflow
  section states deselection is expressed "under the existing remove-and-replace
  policy"; it must state catalog-derived pruning instead.
- `context/patterns.md` — the "For setup install execution" bullet and the
  optional-workflow "remove-and-replace installs already clear them" bullet both
  encode the old policy.
- `context/overview.md` — the setup paragraph describing the unified
  remove-and-replace policy.
- `context/sce/doctor-human-text-contract.md` — if the drift vocabulary for
  merge-target files changes in T05.
- `context/sce/generated-opencode-plugin-registration.md` — the generated
  `opencode.json` is now a merge fragment, not a whole-file payload.

## Constraints and non-goals

- **In scope:** `cli/src/services/setup/mod.rs` (install flow, staging, pruning,
  merge seam), a new JSON-merge module under `cli/src/services/setup/`,
  `cli/src/services/doctor/inspect.rs` integration-asset inspection, and the
  durable-context files named under Context sync.
- **Out of scope:** `install_required_git_hooks` and the `.git/hooks/*` payloads.
  Those still remove and replace an existing `pre-commit`/`commit-msg`/`post-commit`
  file wholesale; chaining shell hooks is a different problem (see Open questions).
- **Out of scope:** `.sce/config.json`, which is already create-if-missing with
  additive key writes and needs no change.
- **Out of scope:** Pkl authoring and the generated payload. Generation stays
  byte-identical; `nix run .#pkl-check-generated` must keep passing unchanged.
- **Constraints:** No backup artifacts (`context/sce/setup-no-backup-policy-seam.md`).
  No new crate dependency — `serde_json` is already a CLI dependency and is
  sufficient for the merge work. Unit tests must stay filesystem-free
  (`context/patterns.md`, "Unit testing in Nix sandbox"): merge logic is pure and
  unit-tested; install/prune behavior belongs in integration tests.
- **Non-goal:** A general-purpose declarative config-merge engine. Two files, two
  known shapes, one shared ownership marker.
- **Non-goal:** A persisted install manifest. Pruning is derived from the
  compiled-in asset catalog (see Open questions for the residue this leaves).

## Assumptions

- SCE-owned files are overwritten without asking. A user who edited
  `.claude/skills/sce-commit/SKILL.md` loses that edit, exactly as today. "Unrelated"
  in the change request means files SCE never authored, not SCE files a user
  modified.
- On a merge conflict inside a JSON config, the SCE-owned value wins for
  SCE-owned keys and entries; every other key and entry is preserved untouched.
  Setup cannot do its job otherwise.
- SCE ownership inside `.claude/settings.json` is identified by the hook command
  string containing `run-sce-or-show-install-guidance.sh`, which every generated
  Claude hook entry routes through (`config/pkl/renderers/claude-content.pkl:8`).
- SCE ownership inside `.opencode/opencode.json` is identified by a `plugin` entry
  matching a canonical SCE plugin path (`./plugins/sce-bash-policy.ts`,
  `./plugins/sce-agent-trace.ts`), authored in `config/pkl/base/opencode.pkl`.
- A malformed pre-existing JSON config is a hard error with actionable guidance,
  not a silent overwrite. Silently replacing an unparseable user file is the same
  data loss this plan exists to remove.
- Pruning stays stateless and catalog-derived, with no persisted install
  manifest. Orphan files installed by an older `sce` under names the current
  binary no longer knows are accepted residue — decided by the user when this was
  raised as an open question.

## Task stack

- [x] T01: `Install setup assets per file instead of replacing the target directory` (status:done)
  - Task ID: T01
  - Goal: `sce setup` writes each embedded asset into its own path under the target directory, creating parent directories as needed, and never removes the target root or any path it did not author.
  - Boundaries (in/out of scope): In — `install_assets_for_concrete_target_with_rename` and its staging/swap helpers in `cli/src/services/setup/mod.rs`; per-file stage-then-rename with cleanup of the staging file on failure; the existing writability probe and recovery guidance retargeted to the individual asset path. Out — pruning stale SCE paths (T02), JSON merging (T03/T04), doctor (T05), git hooks.
  - Dependencies: none
  - Done when: installing into a target directory seeded with user-owned files at the top level, inside `skills/`, and inside `commands/` leaves every seeded file byte-identical while every embedded asset for the selected set is present with correct content; `remove_existing_install_target` is no longer called on an integration root; swap failure on one asset still cleans its staging artifact and returns recovery guidance naming that asset path.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml`.
  - Implementation evidence: `install_assets_for_concrete_target_with_rename` (`cli/src/services/setup/mod.rs`) no longer stages a whole directory and swaps it over `destination_root`. It now loops over each embedded asset and calls new `install_single_asset_with_rename`, which stages a per-asset temp file next to the asset's real destination (`create_asset_staging_path`, mirroring the existing hook-install staging pattern), removes only that single existing destination file if present (bailing instead of deleting if the destination is unexpectedly a directory), then renames the staged file into place. Staging cleanup and `setup_install_recovery_guidance` are retargeted to the individual asset path on failure. Dead whole-directory helpers `create_staging_root` and `write_assets_to_staging` were removed; `remove_existing_install_target` is retained only for the untouched, out-of-scope git-hooks path. `install_embedded_setup_assets_with_rename` was widened from private to `pub(super)` so tests can inject a failing `rename_fn`.
  - Verification evidence: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 16 passed, including two new tests: `install_preserves_user_owned_files_and_writes_sce_assets` (seeds top-level, `skills/`, and `commands/` user files in `.claude`, installs, asserts all three survive byte-identical and `commands/next-task.md` matches the embedded catalog bytes) and `install_cleans_up_staging_and_reports_asset_path_on_rename_failure` (forces a rename failure for `commands/next-task.md` via the injected `rename_fn`, asserts the error names that destination path and includes "does not create backups", and asserts no leftover `.sce-setup-staging-` file in that asset's parent directory). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean.
  - Deviations/assumptions: None beyond the review assumptions already recorded in the plan.

- [x] T02: `Prune unselected and stale SCE-owned asset paths after install` (status:done)
  - Task ID: T02
  - Goal: Restore deselection and stale-asset cleanup, which T01 removed, by deleting exactly those paths the full embedded catalog for the target claims but the resolved selection does not install.
  - Boundaries (in/out of scope): In — a prune step in `cli/src/services/setup/mod.rs` computing `full catalog for target` minus `installed set`, deleting each such file, and removing SCE-owned skill directories left empty by that deletion. Out — deleting any path outside the compiled-in catalog; persisted install manifests; merge targets, which are never pruned because the file is shared with the user.
  - Dependencies: T01
  - Done when: installing with `brownfield` selected and then reinstalling with an empty selection removes `.claude/commands/brownfield.md` and `.claude/skills/sce-brownfield/` entirely, leaves a sibling user-owned `.claude/skills/my-skill/` untouched, and leaves a user file placed inside an SCE-owned skill directory intact (so that directory is not removed as empty).
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`.
  - Implementation evidence: `install_assets_for_concrete_target_with_rename` (`cli/src/services/setup/mod.rs`) now calls new `prune_stale_assets_for_concrete_target` after the per-asset install loop. It diffs `embedded_assets_for_concrete_target` (the full unfiltered catalog for the concrete target) against the just-installed `assets` slice by `relative_path`, and removes the destination file for every catalog path not in that installed set (a no-op when the file is already absent, covering assets an older or renamed catalog left behind). Each successful removal calls new `remove_empty_ancestor_directories`, which walks upward from the removed file's parent directory calling `fs::remove_dir` until it reaches `destination_root` or a directory removal fails (a non-empty directory, such as one still holding a user file, fails `fs::remove_dir` and stops the walk, so it survives). `embedded_assets_for_concrete_target` was added to the `install` submodule's `use super::{...}` import list; no other signature changed.
  - Verification evidence: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 18 passed, including two new tests: `reinstall_with_empty_selection_prunes_deselected_workflow_without_touching_sibling_skill` (installs Claude with `brownfield` selected, seeds a sibling `.claude/skills/my-skill/SKILL.md`, reinstalls with an empty selection, asserts `.claude/commands/brownfield.md` and `.claude/skills/sce-brownfield/` are both gone entirely and the sibling skill file is untouched) and `reinstall_with_empty_selection_keeps_pruned_skill_dir_holding_a_user_file` (same flow but seeds `.claude/skills/sce-brownfield/MY_OVERRIDE.md` before reinstalling, asserts the SCE `SKILL.md` is pruned, the directory survives because it still holds the user file, and that file's content is intact). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean.
  - Deviations/assumptions: None beyond the review assumptions already recorded in the plan.

- [x] T03: `Merge SCE hook entries into an existing .claude/settings.json` (status:done)
  - Task ID: T03
  - Goal: Install `.claude/settings.json` by merging the generated document into the user's existing one rather than replacing it, preserving every non-SCE key and hook entry.
  - Boundaries (in/out of scope): In — a new pure merge module under `cli/src/services/setup/` (for example `config_merge.rs`) exposing a `serde_json`-based merge for the Claude settings shape; a merge-target classification for asset relative path `settings.json` in the Claude install path; deterministic error on an unparseable existing file. Out — OpenCode (T04); doctor (T05); any change to the generated Pkl payload.
  - Done when: merging the generated document into a settings file carrying `permissions`, `env`, and a user `PreToolUse` hook entry yields a document retaining all three; SCE hook entries (identified by `run-sce-or-show-install-guidance.sh` in the command) are replaced rather than appended, so two consecutive installs produce byte-identical output; an SCE hook entry the current generated document no longer contains is removed; a missing file is created from the generated document verbatim; an unparseable existing file fails with a message naming the path and does not write.
  - Dependencies: T01
  - Verification notes (commands or checks): pure merge unit tests in the new module (no filesystem); `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::config_merge`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`.
  - Implementation evidence: New pure module `cli/src/services/setup/config_merge.rs` (declared via `mod config_merge;` in `cli/src/services/setup/mod.rs`) exposes `merge_or_create_claude_settings(existing_bytes: Option<&[u8]>, generated_bytes: &[u8], source_path: &str) -> Result<Vec<u8>>`. It returns `generated_bytes` verbatim when `existing_bytes` is `None`; otherwise it parses both as JSON (a parse failure on the existing document is a hard error naming `source_path`) and calls pure `merge_claude_settings(&Value, &Value, &str) -> Result<Value>`, which copies `$schema` from the generated document (SCE-owned), and for each event key the generated `hooks` object declares (currently `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`), replaces only the SCE-owned entries in `existing.hooks[event]` — identified via `hook_entry_is_sce_owned`, which checks whether any `hooks[].command` contains the marker `run-sce-or-show-install-guidance.sh` — with the generated entries for that event, appended after the surviving non-SCE entries; every other top-level key and every hook event key the generated document does not declare are left untouched. The result is re-serialized with `serde_json::to_string_pretty` plus a trailing newline. In `mod install` (`cli/src/services/setup/mod.rs`), `install_single_asset_with_rename` gained a new `is_claude_settings_merge_target(target, relative_path)` check (true only for `SetupTarget::Claude` + `claude_asset::SETTINGS_FILE`); when true it reads the existing destination bytes (if the file exists) before staging, computes `install_bytes` via `config_merge::merge_or_create_claude_settings`, and stages/renames those bytes instead of `asset.bytes` directly — the rest of the stage-then-rename, cleanup, and `setup_install_recovery_guidance` behavior is unchanged.
  - Verification evidence: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — clean. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::config_merge` — 7 passed, covering: user-key/non-SCE-hook-entry preservation, SCE-entry replacement producing byte-identical output across two merges, an SCE hook entry the generated document no longer declares being dropped, a user-owned event key absent from the generated document being left untouched, a missing file returning the generated bytes verbatim, an unparseable existing file failing with an error naming the path, and a missing `hooks` key in the existing document being populated from generated. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 26 passed, including new integration test `install_merges_into_existing_claude_settings_json_and_stays_idempotent` (seeds `.claude/settings.json` with `permissions`, `env`, and a user `PreToolUse` hook entry, installs, asserts the user keys and hook entry survive alongside an SCE-owned `PreToolUse` entry, reinstalls, and asserts the two installs produce byte-identical `settings.json` content). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean.
  - Deviations/assumptions: Scoped the merge to only the four hook event keys the generator currently emits (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`); a user-owned event key the generated document never declares (e.g. `Notification`) is left completely untouched by every merge, consistent with the plan's non-goal of a general-purpose config-merge engine. No other deviations beyond the review assumptions already recorded in the plan.

- [x] T04: `Merge SCE plugin registrations into an existing .opencode/opencode.json` (status:done)
  - Task ID: T04
  - Goal: Install `.opencode/opencode.json` by merging the canonical SCE `plugin` entries into the user's existing document, preserving every other key and plugin.
  - Boundaries (in/out of scope): In — an OpenCode merge in the T03 module; classifying asset relative path `opencode.json` as a merge target in the OpenCode install path. Out — Claude (T03); doctor (T05); Pkl payload changes.
  - Dependencies: T03
  - Done when: merging into a document carrying `model`, `mcp`, and a user plugin path retains all three, contains each canonical SCE plugin path exactly once after two consecutive installs, and drops a stale SCE-shaped plugin path the current catalog no longer declares; a missing file is created from the generated document verbatim; an unparseable existing file fails with a message naming the path and does not write.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::config_merge`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`.
  - Implementation evidence: `cli/src/services/setup/config_merge.rs` gained `merge_or_create_opencode_config(existing_bytes: Option<&[u8]>, generated_bytes: &[u8], source_path: &str) -> Result<Vec<u8>>`, mirroring the Claude settings merge: returns `generated_bytes` verbatim when `existing_bytes` is `None`; otherwise parses both as JSON (an existing-document parse failure is a hard error naming `source_path`) and calls pure `merge_opencode_config(&Value, &Value, &str) -> Result<Value>`, which copies `$schema` from generated (SCE-owned) and, for `plugin`, filters `existing.plugin` down to entries that are not SCE-shaped — via new `plugin_entry_is_sce_owned`, which checks whether the string starts with new marker constant `OPENCODE_SCE_PLUGIN_PREFIX = "./plugins/sce-"` — then appends `generated.plugin`'s entries; ownership is matched structurally (by path shape) rather than by membership in the current generated array, so a plugin path an older or renamed catalog installed is still recognized and dropped even when the current generated document no longer declares it. Every other top-level key and any `plugin` entry not shaped like an SCE path are left untouched. In `mod install` (`cli/src/services/setup/mod.rs`), new `is_opencode_config_merge_target(target, relative_path)` (true only for `SetupTarget::OpenCode` + `default_paths::repo_file::OPENCODE_MANIFEST`, i.e. relative path `opencode.json`) gated a new branch in `install_single_asset_with_rename` alongside the existing Claude-settings branch: when true, it reads the existing destination bytes (if present) before staging, computes `install_bytes` via `config_merge::merge_or_create_opencode_config`, and stages/renames those bytes instead of `asset.bytes` directly. The install submodule's `use` list gained `crate::services::default_paths` (previously only `default_paths::claude_asset` was imported there) to resolve `repo_file::OPENCODE_MANIFEST`.
  - Verification evidence: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — clean. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::config_merge` — 13 passed, including 7 new OpenCode-merge tests: user-key (`model`, `mcp`) and user-plugin preservation with the two canonical SCE plugin paths appended; idempotence across two merges producing an identical 2-entry `plugin` array; a stale SCE-shaped plugin path (`./plugins/sce-old-feature.ts`) not declared by the current generated document being dropped while a sibling user plugin and both canonical paths survive; a missing file returning the generated bytes verbatim; an unparseable existing file failing with an error naming the path; and a missing `plugin` key in the existing document being populated from generated. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 33 passed, including new integration test `install_merges_into_existing_opencode_config_json_and_stays_idempotent` (seeds `.opencode/opencode.json` with `model`, `mcp`, a user plugin path, and a stale SCE-shaped plugin path, installs, asserts the user keys and plugin survive, both canonical SCE plugin paths are present and the stale one is gone, reinstalls, and asserts the two installs produce byte-identical `opencode.json` content). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean.
  - Deviations/assumptions: SCE ownership of a `plugin` entry is identified structurally by the `./plugins/sce-` path prefix rather than by exact match against the two currently-canonical paths, so that a plugin path an earlier or renamed catalog once installed under that same directory convention is still recognized as SCE-owned and pruned even after the current generated document drops it — needed to satisfy the plan's "drops a stale SCE-shaped plugin path" done check, since a stale path by definition cannot be found by diffing against the current generated set. No other deviations beyond the review assumptions already recorded in the plan.

- [x] T05: `Check merge-target configs by SCE-owned fragment in sce doctor` (status:done)
  - Task ID: T05
  - Goal: `sce doctor` stops reporting drift for a merged JSON config that legitimately carries user content, and reports it only when an SCE-owned fragment is absent or stale.
  - Boundaries (in/out of scope): In — `build_integration_child_from_asset` / `inspect_integration_asset_state` in `cli/src/services/doctor/inspect.rs`, so merge-target assets are inspected by SCE-fragment presence and equality instead of whole-file `sha256`; `--fix` for those assets reusing the T03/T04 merge install. Out — the doctor text layout vocabulary unless the fragment check needs a new state; every non-merge asset, which keeps byte-exact `sha256` checking; hook health checks.
  - Dependencies: T03, T04
  - Done when: a `.claude/settings.json` merged with user `permissions` reports `[PASS]`; the same file with an SCE hook entry deleted or with a stale SCE hook command reports drift; `sce doctor --fix` repairs it by merging and leaves the user keys intact; `.opencode/opencode.json` behaves the same for SCE plugin entries.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::`; manual `sce doctor` in a checkout with a user-extended `.claude/settings.json`.
  - Implementation evidence: `cli/src/services/setup/config_merge.rs` gained `pub(crate) fn claude_settings_fragment_is_current(existing_bytes, generated_bytes) -> Result<bool>` and `pub(crate) fn opencode_config_fragment_is_current(...)`, each parsing both documents, running the existing private `merge_claude_settings`/`merge_opencode_config`, and comparing the merged `Value` against the existing one — a no-op merge means the SCE-owned fragment is already current. `cli/src/services/setup/mod.rs` widened `mod config_merge;` to `pub(crate) mod config_merge;` and added `pub(crate) fn repair_merge_target_asset(repository_root, target, relative_path)`, which delegates to a new `install::repair_merge_target_asset` that looks up the one embedded asset by relative path in `embedded_assets_for_concrete_target` and reinstalls only that asset through the existing `install_single_asset_with_rename` (the same per-asset merge-install path T03/T04 wired up), leaving every other asset untouched. `cli/src/services/doctor/inspect.rs`: `build_integration_child_from_asset` now takes `Option<&MergeTargetAsset>` (new two-variant enum `ClaudeSettings`/`OpenCodeConfig`); for those two assets it calls new `inspect_merge_target_asset_state`, which reads the existing file and calls the matching fragment-check function (a read/parse failure or drifted fragment both surface as `Mismatch`, since remediation is the same either way); every other asset keeps the prior `sha256` path unchanged via `inspect_integration_asset_state`. New `repair_merge_target_configs(repository_root)` re-collects the Claude/OpenCode integration groups, and for each merge-target child currently in `Mismatch`, calls `repair_merge_target_asset` and records a `DoctorFixResultRecord`; a merge target already `Match` or fully `Missing` is left untouched (missing files stay covered by the existing "reinstall assets" guidance). `cli/src/services/doctor/mod.rs`'s `execute_doctor_with_lifecycle_providers` now calls `repair_merge_target_configs(repository_root)` during `--fix`, before re-diagnosing for the final report, alongside the existing lifecycle-provider fixes.
  - Verification evidence: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::config_merge` — 21 passed, including 4 new fragment tests covering: a Claude settings file with extra user keys and a fully current SCE fragment reporting current, a deleted SCE hook entry reporting not-current, an OpenCode config with an extra user plugin reporting current, and a stale SCE-shaped plugin path reporting not-current. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` — 6 passed, including 3 new filesystem-backed tests: `claude_settings_reports_match_despite_extra_user_permissions` (user `permissions` alongside a current fragment reports `Match`), `claude_settings_reports_mismatch_when_sce_hook_entry_deleted_then_fix_repairs_it` (emptied hook arrays report `Mismatch`, `repair_merge_target_configs` fixes it, user `permissions` survive, and a second inspection reports `Match`), and the equivalent `opencode_config_reports_match_despite_extra_user_plugin_then_drift_and_fix` for `.opencode/opencode.json`. `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — 37 passed (no regressions). `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean. Manual verification: in a temp git checkout with `sce setup --claude` run, adding user `permissions`/`env` keys to `.claude/settings.json` and running `sce doctor` showed `[PASS] settings.json`; emptying `hooks.PreToolUse` showed `[FAIL] settings.json (... - content mismatch)`; `sce doctor --fix` printed `[fixed] Merged canonical SCE fragments into 'settings.json'.` and the subsequent `sce doctor` showed `[PASS]` again with the user `permissions`/`env` keys intact.
  - Deviations/assumptions: A merge-target file that fails to parse as JSON is reported the same as a drifted fragment (`Mismatch`), not a distinct state, since the plan's boundaries keep new doctor vocabulary out of scope unless the fragment check needs it, and both cases point to the same remediation. `repair_merge_target_configs` only repairs a merge-target child already in `Mismatch`; a fully missing merge-target file is left to the existing generic "reinstall assets" missing-file guidance rather than being created in isolation by the fix path, since creating just that one file when the rest of the integration is absent would be a surprising partial repair. No other deviations beyond the review assumptions already recorded in the plan.

## Open questions

- `sce setup --hooks` still removes and replaces `.git/hooks/pre-commit`,
  `commit-msg`, and `post-commit` wholesale, so a husky or lefthook repository
  loses its hook on a setup run. Asked whether shell hooks can be merged: not
  textually, but a dispatcher works. SCE would keep `.git/hooks/<name>` as a thin
  dispatcher, relocate a pre-existing foreign hook to `.git/hooks/<name>.d/10-local`
  preserving its mode, and have the dispatcher run every executable in `<name>.d/`
  in lexical order, abort on the first non-zero exit, then run the SCE logic. It is
  tractable here because all three hooks have simple contracts — `pre-commit` and
  `post-commit` take no arguments, `commit-msg` takes one message-file path, none
  read stdin — and the ordering falls out correctly, with SCE's `commit-msg` last
  so its trailer lands on the final message. Two caveats: husky and lefthook set
  `core.hooksPath`, which `install_required_git_hooks` does not currently honour
  (it resolves via `git rev-parse --git-path hooks`), so SCE and the manager write
  to different directories until that resolution is fixed; and even where the paths
  do collide, a manager reinstalling its own hooks overwrites the dispatcher, so the
  scheme is cooperative rather than authoritative. Adding it means a marker line in
  the hook templates, `core.hooksPath`-aware resolution, the dispatcher template,
  and relocation logic — roughly two more tasks. Undecided: say whether to add them.

## Validation Report

**Status:** failed  
**Date:** 2026-08-03

### Commands run

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` -> exit 0 (37 passed, 0 failed — covers AC1-AC4)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` -> exit 0 (6 passed, 0 failed — covers AC5's automated portion)
- Manual `sce doctor` in a temp checkout with a user-extended `.claude/settings.json` -> pass (AC5's manual portion: `[PASS] settings.json` with user `permissions`/`env` present; emptying `hooks.PreToolUse` produced `[FAIL] settings.json (... - content mismatch)`; `sce doctor --fix` reported `[fixed] Merged canonical SCE fragments into 'settings.json'.` and a follow-up `sce doctor` showed `[PASS]` again with the user keys intact)
- `nix flake check` -> exit 1 (`checks.x86_64-linux.cli-fmt` failed: `cargo fmt -- --check` reports unformatted diffs in `cli/src/services/setup/mod.rs` and `cli/src/services/setup/config_merge.rs`; `cli-clippy` and `cli-tests` build successfully in isolation)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 101 files, inventory sha256 a1da453613edc8ecb1e04f35f37471ac02674bad5f2564ae70994e9f1acc6775)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Running setup for a target leaves every non-owned file in that target directory untouched, including files nested inside SCE-owned parent directories -> `install_preserves_user_owned_files_and_writes_sce_assets` passes
- [x] AC2: Deselecting an optional workflow removes only that workflow's files while leaving unrelated files intact -> `reinstall_with_empty_selection_prunes_deselected_workflow_without_touching_sibling_skill` and `reinstall_with_empty_selection_keeps_pruned_skill_dir_holding_a_user_file` pass
- [x] AC3: Installing into an existing `.claude/settings.json` preserves user keys and hook entries, with exactly one current SCE hook entry per event and no duplicates after repeated runs -> `install_merges_into_existing_claude_settings_json_and_stays_idempotent` and `config_merge` unit tests pass
- [x] AC4: Installing into an existing `.opencode/opencode.json` preserves user keys and plugin paths, with each canonical SCE plugin path present exactly once and no stale SCE plugin path left behind -> `install_merges_into_existing_opencode_config_json_and_stays_idempotent` and `config_merge` unit tests pass
- [x] AC5: `sce doctor` reports `[PASS]` for a target whose merged JSON configs carry extra user content, and reports drift only when an SCE-owned fragment is missing or stale -> `doctor::` tests pass; manual verification above confirms `[PASS]` with extra user content, drift detection on a deleted SCE hook entry, and `--fix` repair preserving user keys

### Failed checks and follow-ups

- `nix flake check` / `checks.x86_64-linux.cli-fmt`: `cargo fmt -- --check` fails against the current tree; evidence: the fmt derivation's build log shows reflow diffs across roughly a dozen sites in `cli/src/services/setup/mod.rs` (e.g. `is_opencode_config_merge_target`, several test bodies around lines 2090-2246) and `cli/src/services/setup/config_merge.rs` (test bodies around lines 350-469); required: run `cargo fmt --manifest-path cli/Cargo.toml` in a normal work session to reformat the affected files, then rerun `nix flake check`. Also required before any Nix check can see it: `cli/src/services/setup/config_merge.rs` was untracked in git going into this validation run (Nix flake source filtering only includes git-tracked files, so the module was invisible to `nix flake check` until staged) — it has been `git add`ed as part of this validation run; no file content was changed by that action.

### Residual risks

- None identified.

### Retry

After repairs, rerun:

`/validate context/plans/non-destructive-setup-install.md`
