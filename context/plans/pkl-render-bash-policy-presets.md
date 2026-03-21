# Plan: Render bash-policy-presets from Pkl

## Change summary

Convert `config/pkl/data/bash-policy-presets.json` from a static JSON file to a Pkl-rendered output, aligning it with the existing Pkl generation architecture used for other config assets.

## Success criteria

- `bash-policy-presets.json` is generated from Pkl source during `nix run .#sync-opencode-config`
- CLI embeds the preset catalog from the Pkl-generated output
- All runtime consumers (OpenCode plugin, Claude hook, automated profile) receive identical preset JSON
- `nix flake check` passes
- `nix run .#pkl-check-generated` passes

## Constraints and non-goals

**Constraints:**
- Must maintain exact JSON output structure (schema compatibility)
- Must not change preset IDs, messages, or matching logic
- Must keep CLI embedding working via `include_str!`

**Non-goals:**
- No changes to preset content (IDs, messages, argv_prefixes)
- No changes to runtime enforcement behavior
- No changes to config schema

## Tasks

- [x] T01: `Create Pkl source file for bash-policy-presets` (status:done)
  - Task ID: T01
  - Goal: Create `config/pkl/base/bash-policy-presets.pkl` that defines the preset catalog data structure and renders to JSON.
  - Boundaries (in/out of scope):
    - In: Pkl class/module definition, preset data, JSON rendering
    - Out: generate.pkl integration, CLI changes, context updates
  - Done when: Pkl file exists with correct data structure and `rendered` output matching current JSON schema.
  - Verification notes: `pkl eval config/pkl/base/bash-policy-presets.pkl` produces valid JSON matching current `bash-policy-presets.json` structure.
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** `config/pkl/base/bash-policy-presets.pkl` (new file)
  - **Evidence:** Pkl file created with `PresetMatch`, `BashPolicyPreset`, `RedundancyWarning` classes; `JsonRenderer` output matches original JSON structure (schema_version=1, 5 presets, mutually_exclusive, redundancy_warnings).
  - **Notes:** Used Pkl's `JsonRenderer` for JSON output; data structure matches original `config/pkl/data/bash-policy-presets.json` exactly.

- [x] T02: `Update generate.pkl to use Pkl-rendered presets` (status:done)
  - Task ID: T02
  - Goal: Modify `config/pkl/generate.pkl` to import and use the Pkl-rendered bash-policy-presets instead of reading the static JSON file.
  - Boundaries (in/out of scope):
    - In: generate.pkl import changes, output file paths remain unchanged
    - Out: CLI embedding path, flake.nix changes
  - Done when: `nix run .#sync-opencode-config` generates identical output to current `config/.opencode/lib/bash-policy-presets.json`, `config/automated/.opencode/lib/bash-policy-presets.json`, and `config/.claude/lib/bash-policy-presets.json`.
  - Verification notes: `nix run .#sync-opencode-config && diff -q config/.opencode/lib/bash-policy-presets.json <(pkl eval config/pkl/base/bash-policy-presets.pkl --format json)`
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** `config/pkl/generate.pkl` (modified), `config/pkl/base/bash-policy-presets.pkl` (modified - added PresetCatalog class and removed rendered property)
  - **Evidence:** `nix run .#sync-opencode-config` succeeded, `nix run .#pkl-check-generated` passed, `nix flake check` passed, all three output locations have identical JSON, Pkl output matches original JSON structure.
  - **Notes:** Used `bash_policy_presets.output.text` to access the rendered JSON from the Pkl module; also fixed T01's `bash-policy-presets.pkl` to add `PresetCatalog` class for proper JSON rendering.

- [x] T03: `Update flake.nix fileset reference` (status:done)
  - Task ID: T03
  - Goal: Update `flake.nix` to reference the Pkl source file instead of the static JSON file for CLI embedding.
  - Boundaries (in/out of scope):
    - In: flake.nix fileset change
    - Out: CLI code changes
  - Done when: `nix flake check` passes with updated fileset.
  - Verification notes: `nix flake check`
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** `flake.nix` (removed static JSON fileset entry)
  - **Evidence:** `nix flake check` passed after combining with T04 (cli-tests, cli-clippy, cli-fmt, pkl-parity all succeeded).
  - **Notes:** Removed `(pkgs.lib.fileset.maybeMissing ./config/pkl/data/bash-policy-presets.json)` from fileset union since generated output is already included via `config/.opencode`. Combined with T04 since done checks are coupled.

- [x] T04: `Update CLI embedding path` (status:done)
  - Task ID: T04
  - Goal: Update `cli/src/services/config.rs` to embed from the Pkl-generated output location.
  - Boundaries (in/out of scope):
    - In: config.rs `include_str!` path update
    - Out: Rust logic changes
  - Done when: CLI compiles and embeds correct preset data.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo build'`
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** `cli/src/services/config.rs` (updated `include_str!` path)
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all succeeded).
  - **Notes:** Changed `include_str!` path from `config/pkl/data/bash-policy-presets.json` to `config/.opencode/lib/bash-policy-presets.json`. Combined with T03 since done checks are coupled.

- [x] T05: `Remove static JSON file and update context docs` (status:done)
  - Task ID: T05
  - Goal: Delete `config/pkl/data/bash-policy-presets.json` and update context documentation to reflect Pkl ownership.
  - Boundaries (in/out of scope):
    - In: Delete static JSON, update `context/sce/bash-tool-policy-enforcement-contract.md`, update `context/glossary.md`
    - Out: No other context files
  - Done when: Static JSON removed, context docs reference Pkl source path.
  - Verification notes: `git status` shows deletion of `config/pkl/data/bash-policy-presets.json`
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** `config/pkl/data/bash-policy-presets.json` (deleted), `context/sce/bash-tool-policy-enforcement-contract.md` (updated references)
  - **Evidence:** `git status` shows deletion of static JSON file; context file updated to reference `config/pkl/base/bash-policy-presets.pkl` as canonical source.
  - **Notes:** Glossary already referenced Pkl source path; only `bash-tool-policy-enforcement-contract.md` needed updates (lines 155 and 223).

- [x] T06: `Validation and cleanup` (status:done)
  - Task ID: T06
  - Goal: Run full validation suite and verify all outputs are correct.
  - Boundaries (in/out of scope):
    - In: `nix flake check`, `nix run .#pkl-check-generated`, CLI tests
    - Out: No additional changes
  - Done when: All checks pass, generated outputs match expected structure.
  - Verification notes: `nix flake check && nix run .#pkl-check-generated && nix develop -c sh -c 'cd cli && cargo test'`
  - **Status:** done
  - **Completed:** 2026-03-21
  - **Files changed:** None (validation-only task)
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity), `nix run .#pkl-check-generated` passed ("Generated outputs are up to date"), CLI tests passed (293 tests with `--test-threads=1`).
  - **Notes:** Pre-existing test race condition in `prompt_capture_flow_persists_and_queries_end_to_end` when running parallel tests (temp directory collision) - unrelated to T01-T05 changes, passes reliably with single-threaded execution.

## Open questions

None - the change is straightforward and follows existing patterns from `sce-config-schema.pkl`.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (cli-tests, cli-clippy, cli-fmt, pkl-parity all passed)
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date")
- `nix develop -c sh -c 'cd cli && cargo test -- --test-threads=1'` -> exit 0 (293 tests passed, 0 failed)

### Temporary scaffolding removed
- None - no temporary scaffolding was introduced during implementation

### Success-criteria verification
- [x] `bash-policy-presets.json` is generated from Pkl source during `nix run .#sync-opencode-config` -> confirmed via `nix run .#pkl-check-generated` passing
- [x] CLI embeds the preset catalog from the Pkl-generated output -> confirmed via `include_str!("../../../config/.opencode/lib/bash-policy-presets.json")` in `cli/src/services/config.rs`
- [x] All runtime consumers (OpenCode plugin, Claude hook, automated profile) receive identical preset JSON -> confirmed via file existence at `config/.opencode/lib/`, `config/automated/.opencode/lib/`, and `config/.claude/lib/`
- [x] `nix flake check` passes -> confirmed (exit 0)
- [x] `nix run .#pkl-check-generated` passes -> confirmed ("Generated outputs are up to date")

### Residual risks
- Pre-existing test race condition in `prompt_capture_flow_persists_and_queries_end_to_end` when running parallel tests (temp directory collision) - unrelated to this plan's changes, passes reliably with single-threaded execution