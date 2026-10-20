# Plan: claude-only-sce-info

## Change summary

Include the supplied `.claude/skills/sce-info` package in the canonical Pkl-generated Claude payload so `sce setup --claude` and `sce setup --all` install its runtime files. Other targets must receive no `sce-info` assets. Today the package exists only as an untracked local directory, so a built CLI cannot distribute it.

Adopt the package into canonical authoring sources under `config/lib/sce-info`, emit an explicit runtime-file allowlist through the Claude renderer, and carry those sources through generator input tracking and CLI embedding. Keep the supplied runtime behavior and plugin identity. Keep development tests in authoring sources, but exclude them and the development-only TypeScript configuration from generated and installed payloads. This extends existing target-specific asset generation; it does not introduce a shared workflow or a new plugin activation mechanism.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: The generated Claude payload contains exactly the three `sce-info` runtime assets: `skills/sce-info/.claude-plugin/plugin.json`, `skills/sce-info/hooks/hooks.json`, and `skills/sce-info/hooks/register.ts`. The manifest retains the `sce-splash` identity, hook registration still points to `./register.ts`, and registration code retains the supplied behavior.
  - Validate: `nix run .#pkl-check-generated`; generation-contract assertions must check the exact package inventory and runtime content against canonical sources.
- [x] AC2: No generated OpenCode, Pi, or Codex payload contains `sce-info`; no generated or embedded `sce-info` package contains a test file, a `tests/` directory, or `tsconfig.json`.
  - Validate: `nix run .#pkl-check-generated`; target-specific generation-contract assertions must reject any non-Claude `sce-info` path and any package path outside the runtime allowlist. The Rust regression test `sce_info_assets_install_only_for_claude_and_exclude_development_files` must also inspect embedded catalogs for all targets.
- [x] AC3: Setup into a temporary repository installs all three runtime files for Claude, including when all targets are selected, and installs none for an individual non-Claude target. Test source files are never copied by setup, and existing unrelated user files remain preserved.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sce_info_assets_install_only_for_claude_and_exclude_development_files`; the regression test must exercise actual installation with temporary repositories and inspect resulting paths and contents.
- [x] AC4: Repository-native generation and build consume tracked canonical sources, and edits to any shipped `sce-info` source participate in stale-input detection and rebuild tracking without depending on the local `.claude` runtime tree.
  - Validate: Inspect `config/pkl/generator-inputs.txt`, `cli/build.rs` canonical input declarations and fixture coverage, and the Claude renderer's source reads; all three runtime sources must be declared consistently and no generation source may read `.claude/skills/sce-info`. Generation/build checks must pass with these declarations.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/patterns.md` must record the Claude-only package's canonical sources, explicit runtime allowlist, and exclusion from shared workflow inventories.
- `context/sce/setup-no-backup-policy-seam.md` must describe installation of the Claude-only package and distinguish not copying development files from deleting pre-existing unowned files.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** Canonical `config/lib/sce-info` package sources and development fixtures; Claude-specific Pkl emission; `config/pkl/generate.pkl`; `config/pkl/renderers/claude-content.pkl`; `config/pkl/renderers/generation-contract-check.pkl`; `config/pkl/generator-inputs.txt`; CLI build input declarations and associated fixtures; focused setup regression coverage; context synchronization for this packaging contract.
- **Out of scope:** New behavior for the splash UI, new Claude runtime APIs or dependencies, plugin discovery/activation changes, other target integrations, and changes to shared workflow catalogs.
- **Constraints:** Generate only into temporary payload roots. Keep `config/.claude`, `config/.opencode`, `config/.pi`, and `cli/assets/generated/` absent. Preserve setup's existing staging, atomic rename, ownership, and unrelated-file preservation behavior. Preserve the user's untracked local package while establishing canonical sources. Run non-coreutils tools through Nix and Cargo through `scripts/run-cli-cargo.sh`.
- **Non-goal:** A global setup filename blacklist or recursive packaging mechanism for arbitrary local skills. Exclude development assets at generation by allowlisting the package's runtime files.

## Assumptions

- The requested installation path remains `.claude/skills/sce-info`; the supplied package is plugin-shaped and has no `SKILL.md`, so this work packages it as supplied rather than inventing a workflow skill document.
- Preserve the existing `sce-splash` manifest name and runtime hook behavior; the request concerns distribution of the package under `sce-info`, not a plugin rename or activation redesign.
- The three manifest/hook files are the runtime payload. The supplied `tests/sce-splash.test.ts` and `tsconfig.json` are development assets; the latter currently extends an absent generated types file and is not needed by the runtime hook.
- Excluding test files means future setup copies do not contain them. Setup continues to preserve an already-present unowned local test file, following its existing no-backup installation contract.

## Task stack

- [x] T01: `Package sce-info only in Claude setup assets` (status:done)
  - Task ID: T01
  - Scope: In — adopt supplied package sources under `config/lib/sce-info` without removing the user's local directory; add an explicit three-file runtime mapping to Claude-only Pkl output; update generator-input declarations, build input fixtures, and exact artifact inventory; add generation guards and an actual-installation regression test covering Claude, all-target selection, non-Claude targets, excluded development files, and unrelated-file preservation. Out — changing splash behavior or manifest identity, runtime activation, shared workflow registration, global filename filtering, and removal of local unowned files.
  - Dependencies: none
  - Done when: Canonical generation and embedded setup catalogs contain only the three runtime assets under the Claude package path; source tracking covers every shipped canonical input; generation guards enforce target isolation and development-file exclusion; the focused installation regression passes; relevant durable context records the packaging contract during task synchronization.
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sce_info_assets_install_only_for_claude_and_exclude_development_files`; `git diff --check`.
  - Completed: 2026-10-20
  - Files changed: `cli/build.rs`, `cli/src/services/setup/mod.rs`, `config/lib/sce-info/.claude-plugin/plugin.json`, `config/lib/sce-info/hooks/hooks.json`, `config/lib/sce-info/hooks/register.ts`, `config/lib/sce-info/tests/sce-splash.test.ts`, `config/lib/sce-info/tsconfig.json`, `config/pkl/generate.pkl`, `config/pkl/generator-inputs.txt`, `config/pkl/renderers/claude-content.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `flake.nix`, `scripts/test-run-cli-cargo.sh`, `context/plans/claude-only-sce-info.md`
  - Result: Adopted the supplied package unchanged into config/lib/sce-info; emitted only three Claude runtime assets with exact inventory/content and target-isolation guards; tracked runtime inputs through generation, CLI rebuilds, and filtered Nix source sets; covered installation and preservation of unowned development files.
  - Verify outcomes:
    - `nix run .#pkl-check-generated` — passed; 145 files, inventory SHA-256 b0ec2ffb20ec9f4ecd41b149ecd5c5144845b896924dc797154eb9670531e7b5.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sce_info_assets_install_only_for_claude_and_exclude_development_files` — passed; 1 test, covering embedded catalogs and actual installations for Claude, All, OpenCode, Pi, and Codex, plus preservation on reinstall.
    - `git diff --check` — passed; staged changes also checked with git diff --cached --check.
    - `bash scripts/test-run-cli-cargo.sh` — passed.
    - `nix build .#checks.x86_64-linux.cli-generated-input .#checks.x86_64-linux.pkl-generated --no-link` — passed; filtered Nix source sets provide all canonical runtime inputs.
    - `nix develop -c rustc --test cli/build.rs (with compiled sha2/serde_json dependencies), then /tmp/sce-info-build-tests` — passed; 7 build-script tests, including stale-input rejection for each shipped sce-info source.
    - `Canonical-source comparison against supplied local package` — passed; all five adopted files are byte-identical and the local package is unchanged.
  - Done-check evidence: Exact generated inventory and canonical content passed Pkl assertions; filtered Nix generation checks passed; installation regression proved target isolation, excluded development files, and preservation; each runtime source invalidated stale build input in the build-script tests. Durable context is handed to mandatory synchronization below.
  - Context impact: domain — Claude setup distribution and canonical package ownership; update context/patterns.md and context/sce/setup-no-backup-policy-seam.md.
  - Implementation notes: Added the required runtime files to flake.nix filtered source sets. Refreshed build/wrapper fixtures, including missing baseline version/mutation-scope fixtures and the build fixture loop that treated config/pkl as a file. No runtime behavior or activation changes.
  - Context synchronization: synced
  - Synchronization evidence: Updated context/patterns.md and context/sce/setup-no-backup-policy-seam.md; corrected artifact counts in context/overview.md and the generation architecture; moved detailed generation ownership to context/sce/config-generation.md and linked it from context/architecture.md and context/context-map.md to keep edited files within 250 lines. All five root files verified, including unchanged context/glossary.md; relative links, line limits, source fidelity, and whitespace checks passed. No architecture decision qualified.

## Open questions

None. The supplied runtime package and existing target-specific generation and setup contracts define the requested packaging outcome without requiring a new runtime integration decision.

## Validation Report

**Status:** validated  
**Date:** 2026-10-20

### Commands run

- `nix flake check` -> exit 0 (all native x86_64-linux checks passed; 610 Rust tests passed, with Clippy and formatting checks successful)
- `nix run .#pkl-check-generated` -> exit 0 (145 files; inventory SHA-256 b0ec2ffb20ec9f4ecd41b149ecd5c5144845b896924dc797154eb9670531e7b5)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sce_info_assets_install_only_for_claude_and_exclude_development_files` -> exit 0 (1 passed; target isolation, actual installation, and unowned-file preservation)
- `git diff --check` -> exit 0 (no whitespace errors)
- `git diff --cached --check` -> exit 0 (no staged whitespace errors)
- `AC4 canonical-input inspection via nix shell nixpkgs#python3` -> exit 0 (tracked sources agree across generator inputs, build declarations, renderer reads, and all three Nix source sets; source fidelity and absent forbidden generated trees confirmed)

### Success-criteria verification

- [x] AC1: The generated Claude payload contains exactly the three `sce-info` runtime assets: `skills/sce-info/.claude-plugin/plugin.json`, `skills/sce-info/hooks/hooks.json`, and `skills/sce-info/hooks/register.ts`. The manifest retains the `sce-splash` identity, hook registration still points to `./register.ts`, and registration code retains the supplied behavior. -> Pkl generation passed with exactly 145 total artifacts. The generation contract independently checks the three package paths, byte equality with canonical sources, sce-splash identity, and ./register.ts registration. Canonical runtime sources match the supplied local files.
- [x] AC2: No generated OpenCode, Pi, or Codex payload contains `sce-info`; no generated or embedded `sce-info` package contains a test file, a `tests/` directory, or `tsconfig.json`. -> Generation rejects every sce-info path outside the Claude runtime allowlist. The focused Rust regression passed and checks all four embedded catalogs against that same exact three-path package inventory.
- [x] AC3: Setup into a temporary repository installs all three runtime files for Claude, including when all targets are selected, and installs none for an individual non-Claude target. Test source files are never copied by setup, and existing unrelated user files remain preserved. -> The focused regression passed actual installation for Claude, All, OpenCode, Pi, and Codex in isolated temporary directories; verified installed runtime bytes, excluded development files, unrelated user files, and preservation of pre-existing unowned tests/configuration on reinstall.
- [x] AC4: Repository-native generation and build consume tracked canonical sources, and edits to any shipped `sce-info` source participate in stale-input detection and rebuild tracking without depending on the local `.claude` runtime tree. -> Inspected generator-inputs.txt, cli/build.rs canonical declarations, rerun-if-changed emission, stale-input fixture coverage, Claude renderer reads, and the three filtered Nix source sets. All three runtime files are tracked and consistently declared; generation reads config/lib/sce-info rather than the local .claude tree. Full generation and build checks passed.

### Failed checks and follow-ups

- None.

### Residual risks

- Validation ran on x86_64-linux; the default flake check omits incompatible Darwin and aarch64 systems.
