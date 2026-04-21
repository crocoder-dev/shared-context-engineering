# Plan: CLI Patch Fixture Reconstruction From tmp_hunks

## Change summary

Create a **new** patch-fixture reconstruction scenario under `cli/src/services/patch/fixtures/` using the provided `tmp_hunks/` inputs (`*-message.part.updated.json` files plus the `*-post-commit.json` file). Keep existing fixture suites untouched, add a new test case in `cli/src/services/patch/tests.rs`, and ensure the reconstruction assertion passes (`combine_patches` + `intersect_patches` equals the scenario golden output).

## Success criteria

1. A new fixture suite is added under `cli/src/services/patch/fixtures/` (without editing `average_age_reconstruction` or `hello_world_reconstruction`).
2. The new suite includes:
   - `incremental_XX.patch` files reconstructed from all provided `*-message.part.updated.json` inputs (ordered deterministically by timestamp/filename),
   - `post_commit.patch` reconstructed from `input.head_patch_from_git` in `*-post-commit.json`,
   - `golden.json` representing the expected reconstruction target for `intersect_patches(&combine_patches(incrementals), post_commit)`.
3. `cli/src/services/patch/tests.rs` includes a new scenario and test that references only the new fixture suite via `include_str!`.
4. Existing fixture suites and their tests remain unchanged and still present.
5. The patch test module passes with the new scenario included.

## Constraints and non-goals

- **In scope**:
  - Reading all provided files in `tmp_hunks/`.
  - Extracting authoritative diff payloads from the `message.part.updated` JSON shape.
  - Creating one new deterministic fixture suite and wiring one new test scenario.
  - Path normalization needed for parser/file matching compatibility between incremental and post-commit patches.
- **Out of scope**:
  - Editing `cli/src/services/patch.rs` production logic.
  - Replacing or mutating existing fixture suites.
  - Refactoring current test helper structure beyond minimal additions for the new scenario.
  - Any non-patch service behavior changes.
- **Non-goals**:
  - Introducing runtime JSON-loading in tests (fixtures remain file-based and `include_str!`-driven).
  - Reformatting unrelated tests or fixture content.

## Assumptions

- All `*-message.part.updated.json` files in `tmp_hunks/` are intended as incremental inputs for this single new scenario.
- The single `*-post-commit.json` file in `tmp_hunks/` is the canonical post-commit target for that scenario.
- Deterministic fixture ordering should follow lexical timestamp ordering of filenames.

## Task stack

- [x] T01: `Define fixture scenario contract from tmp_hunks inputs` (status:done)
  - Task ID: T01
  - Goal: Establish deterministic mapping from `tmp_hunks/` JSON files to reconstruction fixture artifacts (incremental sequence + post-commit source + scenario name).
  - Boundaries (in/out of scope): In — scenario folder naming, input selection/ordering rules, extraction field mapping (`metadata.diff`/`metadata.files[].patch` and `input.head_patch_from_git`), parser-compatibility path normalization rules. Out — writing fixture files or editing tests.
  - Done when: A concrete scenario contract exists that specifies exactly which tmp files are used, their order, extraction source fields, and normalized path expectations for matching.
  - Verification notes (commands or checks): Review reconstructed contract against `tmp_hunks/` filenames and existing fixture conventions in `cli/src/services/patch/fixtures/*`.
  - Status update: Completed 2026-04-21
  - Execution notes:
    - Scenario folder name (for T02/T03 implementation): `text_file_lifecycle_reconstruction`.
    - Authoritative incremental input set: all files matching `tmp_hunks/*-message.part.updated.json` (26 files total), consumed in ascending lexical filename order.
    - Authoritative post-commit input: `tmp_hunks/2026-04-21T12-07-48-710Z-post-commit.json` from `input.head_patch_from_git`.
    - Extraction field contract:
      - Primary source for each incremental fixture file: `input.event.properties.part.state.metadata.diff`.
      - Fallback source (only if `metadata.diff` is missing/empty): concatenate `input.event.properties.part.state.metadata.files[].patch` entries in listed order, separated by one newline.
      - Post-commit fixture source: `input.head_patch_from_git`.
    - Path-normalization contract for incrementals:
      - Normalize `Index:`, `---`, and `+++` absolute paths by removing the repository prefix `/home/USER/Desktop/repository/shared-context-engineering/` so reconstructed fixtures use relative paths (for example `notes.txt`, `poem-one.txt`).
      - Keep hunk bodies unchanged.
    - Deterministic fixture numbering contract for T02:
      - `incremental_01.patch` ← `2026-04-21T11-10-35-162Z-message.part.updated.json`
      - `incremental_02.patch` ← `2026-04-21T11-17-29-066Z-message.part.updated.json`
      - `incremental_03.patch` ← `2026-04-21T11-24-09-248Z-message.part.updated.json`
      - `incremental_04.patch` ← `2026-04-21T11-25-28-943Z-message.part.updated.json`
      - `incremental_05.patch` ← `2026-04-21T11-27-17-570Z-message.part.updated.json`
      - `incremental_06.patch` ← `2026-04-21T11-28-34-894Z-message.part.updated.json`
      - `incremental_07.patch` ← `2026-04-21T11-29-47-072Z-message.part.updated.json`
      - `incremental_08.patch` ← `2026-04-21T11-30-44-525Z-message.part.updated.json`
      - `incremental_09.patch` ← `2026-04-21T11-31-41-391Z-message.part.updated.json`
      - `incremental_10.patch` ← `2026-04-21T11-33-51-197Z-message.part.updated.json`
      - `incremental_11.patch` ← `2026-04-21T11-35-02-060Z-message.part.updated.json`
      - `incremental_12.patch` ← `2026-04-21T11-35-52-264Z-message.part.updated.json`
      - `incremental_13.patch` ← `2026-04-21T11-36-58-290Z-message.part.updated.json`
      - `incremental_14.patch` ← `2026-04-21T11-37-44-668Z-message.part.updated.json`
      - `incremental_15.patch` ← `2026-04-21T11-39-21-539Z-message.part.updated.json`
      - `incremental_16.patch` ← `2026-04-21T11-39-55-555Z-message.part.updated.json`
      - `incremental_17.patch` ← `2026-04-21T11-44-55-676Z-message.part.updated.json`
      - `incremental_18.patch` ← `2026-04-21T11-45-41-528Z-message.part.updated.json`
      - `incremental_19.patch` ← `2026-04-21T11-46-59-889Z-message.part.updated.json`
      - `incremental_20.patch` ← `2026-04-21T11-47-46-068Z-message.part.updated.json`
      - `incremental_21.patch` ← `2026-04-21T11-49-20-062Z-message.part.updated.json`
      - `incremental_22.patch` ← `2026-04-21T11-50-05-157Z-message.part.updated.json`
      - `incremental_23.patch` ← `2026-04-21T11-54-15-505Z-message.part.updated.json`
      - `incremental_24.patch` ← `2026-04-21T11-54-48-801Z-message.part.updated.json`
      - `incremental_25.patch` ← `2026-04-21T11-55-56-370Z-message.part.updated.json`
      - `incremental_26.patch` ← `2026-04-21T11-56-17-453Z-message.part.updated.json`
    - Context-sync significance: verify-only root context pass expected (localized plan-state update only; no architecture/policy/terminology change).

- [x] T02: `Create new reconstruction fixture suite` (status:done)
  - Task ID: T02
  - Goal: Add a new folder under `cli/src/services/patch/fixtures/` containing deterministic incremental patch files and `post_commit.patch` derived from the scenario contract.
  - Boundaries (in/out of scope): In — new folder creation, writing `incremental_01.patch..incremental_N.patch`, writing `post_commit.patch`, deterministic newline/ordering consistency. Out — modifying any existing fixture files/folders.
  - Done when: The new suite exists with complete incremental sequence and post-commit patch payload, and existing fixture directories are unchanged.
  - Verification notes (commands or checks): File inventory check for the new suite; content spot-check that incrementals are unified-diff text and post-commit matches `head_patch_from_git` payload.
  - Status update: Completed 2026-04-21
  - Execution notes:
    - Added new fixture suite directory: `cli/src/services/patch/fixtures/text_file_lifecycle_reconstruction/`.
    - Added `incremental_01.patch` .. `incremental_26.patch` in deterministic lexical source order and `post_commit.patch` from `input.head_patch_from_git`.
    - Applied T01 path normalization for incremental patch headers (`/home/USER/Desktop/repository/shared-context-engineering/` removed to relative paths).
    - Verified fixture-source contract with a deterministic local check over all 26 incremental source JSON files plus post-commit JSON.
    - Verification evidence:
      - `python3` contract check: `validated 26 incrementals + post_commit.patch`
      - `nix run .#pkl-check-generated` (pass)
      - `nix flake check` (pass)
    - Context-sync significance: verify-only root context pass expected (localized fixture + plan-state update; no architecture/policy/terminology change).

- [x] T03: `Add golden snapshot and test scenario wiring` (status:done)
  - Task ID: T03
  - Goal: Add `golden.json` for the new suite and register a new `PatchScenario` test in `cli/src/services/patch/tests.rs` that validates reconstruction behavior.
  - Boundaries (in/out of scope): In — new `golden.json`, one new test function (or equivalent scenario invocation) using `include_str!` for the new suite. Out — changes to patch production algorithms or existing scenarios.
  - Done when: `tests.rs` references the new suite, the scenario asserts reconstruction equivalence, and existing tests remain intact.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests -- --nocapture'` (or the narrowest matching patch test target).
  - Status update: Completed 2026-04-21
  - Execution notes:
    - Added `cli/src/services/patch/fixtures/text_file_lifecycle_reconstruction/golden.json` as the deterministic expected output for `intersect_patches(&combine_patches(incrementals), post_commit)`.
    - Updated `cli/src/services/patch/tests.rs` with a new `text_file_lifecycle_reconstruction_matches_post_commit` scenario wired only to `fixtures/text_file_lifecycle_reconstruction/*` via `include_str!`.
    - Existing reconstruction scenarios (`average_age_reconstruction`, `hello_world_reconstruction`) were left unchanged.
    - Verification evidence:
      - `nix build .#checks.x86_64-linux.cli-tests` (pass)
      - `nix build .#checks.x86_64-linux.cli-fmt` (pass)
    - Context-sync significance: verify-only root context pass expected (localized patch fixture + test coverage update; no architecture/policy/terminology change).

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Run repo validation baseline, confirm acceptance criteria, and ensure context sync needs are addressed.
  - Boundaries (in/out of scope): In — validation commands, acceptance checklist confirmation, context follow-up if required. Out — new behavior changes.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; new scenario is present and passing; existing fixture suites remain untouched.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.
  - Status update: Completed 2026-04-21
  - Execution notes:
    - Ran required validation baseline commands and confirmed both pass:
      - `nix run .#pkl-check-generated` (pass)
      - `nix flake check` (pass)
    - Confirmed the new scenario remains wired and present in `cli/src/services/patch/tests.rs` (`text_file_lifecycle_reconstruction_matches_post_commit`).
    - Confirmed fixture suites for `average_age_reconstruction` and `hello_world_reconstruction` remain present alongside `text_file_lifecycle_reconstruction`.
    - Post-acceptance in-scope naming refinement: renamed fixture suite path and test scenario labels from `tmp_hunks_reconstruction` to `text_file_lifecycle_reconstruction` to better describe the mixed text-file lifecycle content represented by the hunks.
    - Context-sync significance: verify-only root context pass expected (validation + plan-state update only; no architecture/policy/terminology change).

## Validation report

### Commands run

- `nix run .#pkl-check-generated` → exit 0 (`Generated outputs are up to date.`)
- `nix flake check` → exit 0 (`all checks passed!`)

### Temporary scaffolding cleanup

- No temporary scaffolding or debug artifacts were introduced by T04.

### Context/state verification

- Plan task state is now updated to done for T04.
- Root context sync classification remains verify-only for this task (no root shared-file edits required).

### Success-criteria verification

- [x] New fixture suite exists under `cli/src/services/patch/fixtures/text_file_lifecycle_reconstruction/`.
- [x] Suite contains deterministic `incremental_XX.patch` files, `post_commit.patch`, and `golden.json`.
- [x] `cli/src/services/patch/tests.rs` contains the new scenario wired via `include_str!` to the new suite.
- [x] Existing fixture suites remain present and unchanged (`average_age_reconstruction`, `hello_world_reconstruction`).
- [x] Patch scenario coverage remains passing under the repository validation baseline (`nix flake check`).

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for this task scope.

## Open questions

- None.
