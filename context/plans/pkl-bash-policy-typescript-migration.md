# Plan: Pkl Bash Policy TypeScript Migration

## Change summary

Migrate OpenCode bash-policy generation to use TypeScript source files from `config/lib/bash-policy/` instead of non-existent `.js` files, remove all Claude bash-policy generation, and add Bun tests for `config/lib/` to the Nix flake with dependencies managed by Nix. OpenCode will copy TypeScript files directly (Bun-native), while Claude will have a full cleanup of bash-policy related hooks, settings, and lib files.

## Success criteria

1. `generate.pkl` reads TypeScript files from `config/lib/bash-policy/` for OpenCode generation
2. OpenCode generated output includes TypeScript files in `config/.opencode/lib/` and `config/automated/.opencode/lib/`
3. Claude generated output no longer includes bash-policy files in `lib/`, `hooks/`, or `settings.json`
4. `nix run .#pkl-check-generated` passes after regeneration
5. `nix flake check` passes after regeneration (including new `config-lib-tests` check)
6. Bun tests for `config/lib/bash-policy/` run as part of `nix flake check`

## Constraints and non-goals

**In scope:**
- Update `generate.pkl` to read from `config/lib/bash-policy/*.ts`
- Update OpenCode plugin registration to reference TypeScript files
- Remove Claude bash-policy generation (lib files, hooks, settings.json hooks)
- Update context documentation to reflect new ownership
- Add Nix flake check for `config/lib/` Bun tests with Nix-managed dependencies

**Out of scope:**
- Migrating `drift-collectors.js` (stays in `config/pkl/lib/`)
- Changing bash-policy runtime behavior
- Adding new bash-policy features

## Task stack

- [x] T01: `Update generate.pkl to read TypeScript from config/lib/bash-policy/` (status:done)
  - Task ID: T01
  - Goal: Modify `generate.pkl` to read TypeScript source files from `config/lib/bash-policy/` instead of non-existent `.js` files from `./lib/`
  - Boundaries (in/out of scope):
    - In: Update file read paths in `generate.pkl` for OpenCode bash-policy files
    - In: Change output file extensions from `.js` to `.ts` for OpenCode lib files
    - In: Update `opencodePackageJsonSource` to reference TypeScript dependencies
    - Out: Claude bash-policy removal (T02)
    - Out: Context documentation updates (T03)
    - Out: Nix flake test integration (T05)
  - Done when: `generate.pkl` reads `bash-policy-runtime.ts` and `opencode-bash-policy-plugin.ts` from `config/lib/bash-policy/` and outputs `.ts` files to `config/.opencode/lib/` and `config/automated/.opencode/lib/`
  - Verification notes:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` succeeds
    - Generated files have `.ts` extension in output paths
    - `git diff` shows correct path changes in `generate.pkl`
  - **Status:** done
  - **Completed:** 2026-03-23
  - **Files changed:** config/pkl/generate.pkl, config/pkl/base/opencode.pkl
  - **Evidence:** pkl eval succeeds, .ts files generated in config/.opencode/lib/ and config/.opencode/plugins/
  - **Notes:** Removed package.json generation as requested; TypeScript files copied directly from config/lib/bash-policy/

- [x] T02: `Remove Claude bash-policy generation from generate.pkl` (status:done)
  - Task ID: T02
  - Goal: Remove all Claude bash-policy related file generation from `generate.pkl`
  - Boundaries (in/out of scope):
    - In: Remove `claudeBashPolicyHookSource` read and output
    - In: Remove `bashPolicyRuntimeSource` and `bashPolicyPresetCatalogSource` outputs for Claude
    - In: Remove bash-policy hooks from `claudeSettingsSource` or update settings.json source
    - In: Remove `claude/lib/bash-policy-*.js` and `claude/hooks/sce-bash-policy-hook.js` output entries
    - Out: OpenCode bash-policy changes (T01)
    - Out: Context documentation updates (T03)
  - Done when: `generate.pkl` no longer generates any bash-policy files for Claude target, and `settings.json` has no `PreToolUse` hook for bash policy
  - Verification notes:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` succeeds
    - No `config/.claude/lib/bash-policy-*` or `config/.claude/hooks/sce-bash-policy-hook.js` in output
    - `git diff` shows removed Claude bash-policy entries
  - **Status:** done
  - **Completed:** 2026-03-23
  - **Files changed:** config/pkl/generate.pkl
  - **Evidence:** nix flake check passes, git diff shows removed Claude bash-policy entries (claudeBashPolicyHookSource, claudeSettingsSource variables removed; config/.claude/lib/bash-policy-*, config/.claude/hooks/sce-bash-policy-hook.js, config/.claude/settings.json outputs removed)
  - **Notes:** Claude no longer generates bash-policy files; drift-collectors.js retained for Claude

- [x] T03: `Update context documentation for bash-policy ownership` (status:done)
  - Task ID: T03
  - Goal: Update context files to reflect the new TypeScript source ownership and Claude removal
  - Boundaries (in/out of scope):
    - In: Update `context/sce/generated-opencode-plugin-registration.md` to reference TypeScript source
    - In: Update `context/sce/bash-tool-policy-enforcement-contract.md` to remove Claude references
    - In: Update `context/overview.md` to reflect Claude no longer has bash-policy enforcement
    - Out: Code changes (T01, T02)
  - Done when: Context files accurately describe the new ownership model
  - Verification notes:
    - `git diff` shows updated context files
    - No references to Claude bash-policy hooks in context files
  - **Status:** done
  - **Completed:** 2026-03-23
  - **Files changed:** context/sce/generated-opencode-plugin-registration.md, context/sce/bash-tool-policy-enforcement-contract.md, context/glossary.md, context/architecture.md, context/patterns.md, context/cli/placeholder-foundation.md
  - **Evidence:** nix run .#pkl-check-generated passes, nix flake check passes, git diff shows 6 files updated with Claude references removed and .ts extensions
  - **Notes:** Updated all context files to reflect OpenCode-only bash-policy enforcement with TypeScript source files; removed cross-target parity section and Claude references

- [x] T04: `Add config/lib Bun tests to Nix flake` (status:done)
  - Task ID: T04
  - Goal: Add a Nix flake check that runs Bun tests for `config/lib/` with Nix-managed dependencies
  - Boundaries (in/out of scope):
    - In: Add `config-lib-tests` check to `flake.nix` that runs `bun test` in `config/lib/bash-policy/`
    - In: Ensure dependencies are managed by Nix (bun is already in devShell)
    - In: Add `config/lib/bash-policy/` source files to the check's input
    - In: Use `bun install` and `bun test` in a Nix derivation
    - Out: Pkl generation changes (T01-T03)
    - Out: Changing test behavior or adding new tests
  - Done when: `nix flake check` includes `config-lib-tests` and it passes
  - Verification notes:
    - `nix flake check` shows `config-lib-tests` in check list
    - `nix build .#checks.x86_64-linux.config-lib-tests` (or equivalent) succeeds
    - Tests run against `config/lib/bash-policy/*.test.ts`
  - **Status:** done
  - **Completed:** 2026-03-23
  - **Files changed:** flake.nix
  - **Evidence:** nix flake check passes, config-lib-tests check included and passing
  - **Notes:** Added fixed-output derivation for Bun dependencies with cached node_modules; removed Bun's .cache symlinks to avoid Nix store issues

- [x] T05: `Regenerate and validate output` (status:done)
  - Task ID: T05
  - Goal: Run Pkl generation and validate all outputs are correct, including new Nix check
  - Boundaries (in/out of scope):
    - In: Run `nix run .#pkl-check-generated` to verify parity
    - In: Run `nix flake check` for full validation (including new `config-lib-tests`)
    - In: Manually inspect generated files for correct content
    - Out: Code changes (T01-T04)
  - Done when: All checks pass and generated files match expected structure
  - Verification notes:
    - `nix run .#pkl-check-generated` exits 0
    - `nix flake check` exits 0 (all checks including `config-lib-tests`)
    - `config/.opencode/lib/bash-policy-runtime.ts` exists
    - `config/.opencode/lib/bash-policy-presets.json` exists
    - `config/.opencode/plugins/sce-bash-policy.js` references TypeScript correctly
    - `config/.claude/lib/` has no bash-policy files
    - `config/.claude/hooks/` has no bash-policy files
    - `config/.claude/settings.json` has no bash-policy hooks
  - **Status:** done
  - **Completed:** 2026-03-23
  - **Files changed:** config/.claude/lib/bash-policy-runtime.js (deleted), config/.claude/lib/bash-policy-presets.json (deleted), config/.claude/hooks/sce-bash-policy-hook.js (deleted), config/.claude/settings.json (deleted)
  - **Evidence:** nix run .#pkl-check-generated passes, nix flake check passes, all acceptance criteria verified
  - **Notes:** Removed orphaned Claude bash-policy files that were missed in T02; all generated outputs now match Pkl sources

## Open questions

None - all clarifications resolved.

## Related files

- `config/pkl/generate.pkl` - main generation entrypoint
- `config/pkl/base/opencode.pkl` - OpenCode plugin registration
- `config/pkl/base/bash-policy-presets.pkl` - preset catalog
- `config/lib/bash-policy/` - TypeScript source directory
- `config/lib/bash-policy/package.json` - Bun dependencies for tests
- `config/lib/bash-policy/bash-policy-runtime.test.ts` - Bun test file
- `config/pkl/lib/drift-collectors.js` - stays as-is
- `flake.nix` - Nix flake with checks
- `context/sce/bash-tool-policy-enforcement-contract.md` - policy contract
- `context/sce/generated-opencode-plugin-registration.md` - plugin registration docs

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity, config-lib-tests)
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date.")
- Removed: `context/tmp/sce-bash-policy-debug.json`, `context/tmp/sce.log` (temporary scaffolding)

### Success-criteria verification
- [x] `generate.pkl` reads TypeScript files from `config/lib/bash-policy/` for OpenCode generation -> confirmed via `config/pkl/generate.pkl` lines 10-11
- [x] OpenCode generated output includes TypeScript files in `config/.opencode/lib/` and `config/automated/.opencode/lib/` -> confirmed via file listing
- [x] Claude generated output no longer includes bash-policy files in `lib/`, `hooks/`, or `settings.json` -> confirmed: files deleted, no bash-policy files in `config/.claude/`
- [x] `nix run .#pkl-check-generated` passes after regeneration -> exit 0
- [x] `nix flake check` passes after regeneration (including new `config-lib-tests` check) -> exit 0
- [x] Bun tests for `config/lib/bash-policy/` run as part of `nix flake check` -> confirmed: `config-lib-tests` check included and passing

### Files verified
- `config/.opencode/lib/bash-policy-runtime.ts` exists
- `config/.opencode/lib/bash-policy-presets.json` exists
- `config/.opencode/plugins/sce-bash-policy.ts` exists
- `config/automated/.opencode/lib/bash-policy-runtime.ts` exists
- `config/automated/.opencode/lib/bash-policy-presets.json` exists
- `config/automated/.opencode/plugins/sce-bash-policy.ts` exists
- `config/.claude/lib/bash-policy*` does not exist
- `config/.claude/hooks/sce-bash-policy*` does not exist
- `config/.claude/settings.json` does not exist

### Residual risks
- None identified.