# Plan: Bash Tool Policy Enforcement

## Change summary

Add a repo-configurable policy layer in `.sce/config.json` that can stop LLM bash-tool execution before selected CLIs/binaries run, with the same operator-facing behavior across OpenCode and Claude.

The new capability should let a repository declare blocked bash-command policies such as "do not run `git`" or narrower presets like "forbid `git add` and `git commit`", and attach a per-policy message that explains the preferred alternative (for example, "use `jj` instead").

The implementation should fit the repo's existing single-source generation model: OpenCode behavior should come from generated plugin assets, Claude behavior should come from generated hook assets/configuration, and the Rust CLI config service should treat the new `.sce/config.json` keys as first-class validated config rather than opaque undocumented JSON.

The initial built-in preset catalog is fixed for this plan:

- `forbid-git-all`
- `forbid-git-commit`
- `use-pnpm-over-npm`
- `use-bun-over-npm`
- `use-nix-flake-over-cargo`

## Success criteria

- `.sce/config.json` supports deterministic policy configuration for blocked bash commands with per-policy messages and easy-to-enable preset entries.
- Policy matching happens before the bash tool executes and blocks the call with a stable, user-visible explanation instead of allowing the command to start.
- OpenCode and Claude enforce the same configured policies and surface equivalent denial guidance for the same blocked command.
- The policy model supports both full-binary blocks (for example `git`) and narrower preset blocks for common subcommands (for example `git add`, `git commit`, `git push`).
- Built-in presets carry fixed repo-owned messages/behavior and do not accept custom per-preset message overrides in config.
- The Rust config service validates, reports, and documents the new config shape through the existing `sce config show` / `sce config validate` contract.
- Generated-owned outputs include any new OpenCode plugin files, Claude hook/config files, and supporting manifests without introducing manual-only drift.
- Setup/install flows place the new enforcement assets in the correct repo-local target locations for both `.opencode/` and `.claude/`.
- Current-state context explains the policy model, preset catalog, cross-target enforcement approach, and any important matching limitations.

## Constraints and non-goals

- Keep enforcement scoped to policy-blocked bash-tool commands only; do not add generic messaging for unrelated tool failures.
- Preserve the existing config precedence model (`flags > env > config file > defaults`) and extend it deterministically rather than adding ad hoc repo-local parsing paths.
- Do not require application-code changes outside the Rust CLI, generated config pipeline, and repo-managed assistant assets.
- Do not depend on full shell parsing; the contract should define a deterministic command-normalization/matching approach that is good enough for obvious wrapper forms and common CLI invocations.
- Keep cross-target parity at the behavior level; implementation details may differ between OpenCode plugins and Claude hooks where platform capabilities differ.
- Make preset activation easy, but keep custom policy entries available so repositories can block other binaries beyond the built-in preset catalog.
- Treat `use-pnpm-over-npm` and `use-bun-over-npm` as mutually exclusive presets during validation.
- Treat `forbid-git-all` as making `forbid-git-commit` redundant but still valid.
- Do not broaden this task into sandboxing arbitrary subprocesses outside bash-tool interception.
- Do not attempt org/global machine policy management in this slice; the source of truth is repo/global SCE config resolution ending in `.sce/config.json`.

## Task stack

- [x] T01: Define the blocked-bash policy contract and preset model (status:done)
  - Task ID: T01
  - Goal: Freeze the current-state contract for bash-tool command blocking, including config schema, matching semantics, denial messaging, preset catalog shape, and cross-target parity expectations.
  - Boundaries (in/out of scope):
    - In: A focused context contract under `context/sce/` plus any root-context wording needed to describe the feature.
    - In: Definition of deterministic command normalization rules, preset-owned denial message behavior, and how full-binary vs subcommand presets are represented.
    - In: Freeze the initial preset catalog as `forbid-git-all`, `forbid-git-commit`, `use-pnpm-over-npm`, `use-bun-over-npm`, and `use-nix-flake-over-cargo`, including conflict/redundancy rules.
  - In: Definition of how OpenCode plugin behavior and Claude hook behavior are considered equivalent.
  - Out: Rust/Pkl/generated implementation.
  - Done when: The repo has one approved contract that names the config keys, preset catalog, matching rules, fixed preset messages, block/allow behavior, and parity expectations clearly enough to implement without re-deciding architecture mid-task.
  - Verification notes (commands or checks): Read-through audit for unambiguous schema fields, preset semantics, fixed preset-message ownership, and cross-target enforcement rules; confirm the contract explicitly limits injected messages to policy-blocked commands only.
  - Completed: 2026-03-17
  - Files changed: `context/sce/bash-tool-policy-enforcement-contract.md`, `context/context-map.md`
  - Evidence: Read-through audit of the new contract for schema clarity, preset semantics, precedence/conflict rules, and policy-only denial behavior; verified discoverability via `context/context-map.md`.
  - Context sync classification: verify-only root context pass; this task adds the canonical focused contract and map entry without changing implemented repo behavior.

- [x] T02: Extend the CLI config contract for policy parsing, validation, and reporting (status:done)
  - Task ID: T02
  - Goal: Teach the Rust config service to load the new policy fields from global/local config files, validate them deterministically, and expose them through `sce config show` / `sce config validate`.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/config.rs`, CLI schema/output updates if needed, and focused config tests.
    - In: Validation for preset references, custom blocked-command entries, per-policy messages, and stable JSON/text reporting.
    - Out: OpenCode/Claude asset generation and runtime enforcement.
  - Done when: Invalid policy config is rejected with actionable errors, valid policy config resolves through the existing precedence model, and `config show|validate` output includes the new fields deterministically.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test config'`; targeted assertions for valid config, unknown keys, invalid preset names, and merged global+local policy resolution.
  - Completed: 2026-03-17
  - Files changed: `cli/src/services/config.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config && cargo clippy --all-targets --all-features'`; `nix flake check`
  - Context sync classification: important change; root config-contract context now needs to describe `policies.bash`, deterministic validation, and reporting/warning behavior.

- [x] T03: Add a shared preset catalog and reusable policy rendering source (status:done)
  - Task ID: T03
  - Goal: Introduce one canonical source for built-in blocked-command presets so the CLI validator and generated assistant assets stay aligned.
  - Boundaries (in/out of scope):
    - In: Canonical authored source under `config/pkl/` and any helper data files/templates needed for generation.
    - In: Presets exactly matching the approved initial catalog: `forbid-git-all`, `forbid-git-commit`, `use-pnpm-over-npm`, `use-bun-over-npm`, and `use-nix-flake-over-cargo`.
    - In: Built-in fixed denial messages and deterministic command-match definitions for each preset.
    - In: Support for custom non-preset binary blocks alongside presets.
  - Out: Runtime hook/plugin wiring.
  - Done when: The repo has one canonical preset catalog that can drive generated assets and validation without duplicating preset names, matchers, or fixed messages across targets.
  - Verification notes (commands or checks): Read-through audit against T01 contract, including preset conflicts and redundancy rules; `nix run .#pkl-check-generated` once generation wiring exists.
  - Completed: 2026-03-17
  - Files changed: `config/pkl/data/bash-policy-presets.json`, `config/pkl/generate.pkl`, `config/pkl/README.md`, `cli/src/services/config.rs`, `config/.opencode/lib/bash-policy-presets.json`, `config/automated/.opencode/lib/bash-policy-presets.json`, `config/.claude/lib/bash-policy-presets.json`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config'`; `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features && cargo build'`; `nix run .#pkl-check-generated`
  - Context sync classification: verify-only root context pass; this task centralizes preset data and shared generated assets without changing the already-documented operator-facing policy contract.

- [x] T04: Generate OpenCode plugin enforcement assets from canonical policy sources (status:done)
  - Task ID: T04
  - Goal: Add generated OpenCode plugin files that intercept bash-tool execution before command launch, evaluate configured policies, and emit the configured denial message for blocked commands.
  - Boundaries (in/out of scope):
    - In: `config/pkl/generate.pkl`, OpenCode renderer modules/metadata as needed, generated-owned `.opencode` plugin/package outputs, and any supporting shared JS/TS code.
  - In: Wiring for repo-local plugin auto-loading and any dependency manifest changes required for generated plugin code.
  - Out: Claude hook implementation.
  - Done when: Generated OpenCode assets contain a deterministic plugin implementation that blocks configured bash commands pre-execution and surfaces the per-policy message for blocked cases.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; inspect generated OpenCode plugin output for deterministic file paths and pre-execution enforcement logic.
  - Completed: 2026-03-17
  - Files changed: `config/pkl/generate.pkl`, `config/pkl/lib/bash-policy-runtime.js`, `config/pkl/lib/opencode-bash-policy-plugin.js`, `config/.opencode/lib/bash-policy-runtime.js`, `config/.opencode/plugins/sce-bash-policy.js`, `config/.opencode/package.json`, `config/automated/.opencode/lib/bash-policy-runtime.js`, `config/automated/.opencode/plugins/sce-bash-policy.js`, `config/automated/.opencode/package.json`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `bun --eval "await import('./config/.opencode/plugins/sce-bash-policy.js'); await import('./config/automated/.opencode/plugins/sce-bash-policy.js');"`; targeted runtime exercise confirmed custom-prefix and preset blocks for `git status` / `git commit` while allowing `git diff`; user-reported `nix flake check` passed after rerun.
  - Context sync classification: important change; operator-facing OpenCode behavior now includes repo-local pre-execution bash-policy enforcement and generated dependency/plugin assets.

- [x] T05: Generate Claude hook enforcement assets with equivalent blocking behavior (status:done)
  - Task ID: T05
  - Goal: Add generated Claude hook assets/configuration that deny matching Bash tool calls before execution and return the same configured policy message as OpenCode.
  - Boundaries (in/out of scope):
    - In: Claude renderer/generation updates for hook-capable generated-owned files, including any new `.claude` settings or bundled hook scripts required by the approved T01 contract.
    - In: Deterministic PreToolUse behavior for Bash tool calls and stable denial output formatting.
    - Out: OpenCode plugin implementation.
  - Done when: Generated Claude assets provide repo-local pre-execution blocking for the same policy set and produce equivalent denial guidance for the same blocked command inputs.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; inspect generated Claude hook/config outputs for deterministic matcher coverage and denial-message parity.
  - Completed: 2026-03-17
  - Files changed: `config/pkl/generate.pkl`, `config/pkl/lib/claude-bash-policy-hook.js`, `config/pkl/lib/claude-settings.json`, `config/.claude/lib/bash-policy-runtime.js`, `config/.claude/hooks/sce-bash-policy-hook.js`, `config/.claude/settings.json`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; imported `config/.claude/hooks/sce-bash-policy-hook.js`; exercised the generated Claude hook against custom `git status`, preset `git commit`, and allowed `git diff` commands; `nix run .#pkl-check-generated` passed. `nix flake check` and `nix flake check "path:$PWD"` were attempted but remain blocked by pre-existing generated-output drift and worktree-sensitive parity checks outside this task's new files.
  - Context sync classification: important change; operator-facing cross-target parity now includes Claude project hooks/settings for pre-execution bash-policy enforcement.

- [x] T06: Extend setup/install orchestration to place policy-enforcement assets for both targets (status:done)
  - Task ID: T06
  - Goal: Ensure repo-local setup/install flows carry the new plugin and hook files into `.opencode/` and `.claude/` as first-class SCE-managed assets.
  - Boundaries (in/out of scope):
    - In: `cli/build.rs`, `cli/src/services/setup.rs`, setup tests, and any manifest iteration helpers needed for new generated-owned paths.
    - In: Deterministic install/update/backup behavior for newly managed OpenCode plugin files and Claude hook/config files.
    - Out: New top-level setup commands or unrelated setup UX changes.
  - Done when: Setup embeds, installs, and updates the new enforcement assets with the same ownership and rollback guarantees as existing SCE-managed generated files.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test setup'`; targeted coverage for embedded asset discovery and install/update behavior on new paths.
  - Completed: 2026-03-17
  - Files changed: `cli/src/services/setup/tests.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test setup'`; `nix develop -c sh -c 'cd cli && cargo build'`; `nix run .#pkl-check-generated`; `nix flake check`
  - Context sync classification: verify-only root context pass; the setup pipeline already recursively embeds and installs generated target assets, and this task adds regression coverage proving the new bash-policy files are included with backup behavior.

- [x] T07: Sync current-state docs for config shape, presets, and cross-target behavior (status:done)
  - Task ID: T07
  - Goal: Update durable context so future sessions understand the policy feature, the config contract, the preset catalog, and where enforcement lives for OpenCode vs Claude.
  - Boundaries (in/out of scope):
    - In: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and any new focused `context/sce/` contract file created in T01.
    - In: `context/cli/` docs if the config/installation surface needs discoverability there.
    - Out: Historical narration or implementation diary content.
  - Done when: Current-state context reflects code truth for the new policy feature and clearly points readers to the canonical contract/preset documentation.
  - Verification notes (commands or checks): Read-through audit for stale config/setup wording; ensure new focused context is linked from `context/context-map.md`.
  - Completed: 2026-03-17
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/cli/placeholder-foundation.md`, `context/glossary.md`
  - Evidence: Read-through audit confirmed root and CLI context now point to the canonical preset catalog and bash-policy contract, describe `policies.bash` reporting, and document OpenCode/Claude enforcement asset locations plus setup discoverability.
  - Context sync classification: task-owned root context update; durable docs now match the implemented bash-policy config/reporting/enforcement surface.

- [x] T08: Validation and cleanup (status:done)
  - Task ID: T08
  - Goal: Run final verification, confirm generated-output parity, and make sure no target-specific drift remains between CLI config handling and generated OpenCode/Claude enforcement assets.
  - Boundaries (in/out of scope):
    - In: Relevant CLI tests, generation parity checks, and full lightweight post-task validation baseline.
    - In: Final audit that presets, config validation, OpenCode plugin assets, Claude hook assets, and setup embedding all align.
  - Out: New feature work.
  - Done when: Validation passes, generated-owned files are in sync, and current-state context matches the implemented policy feature.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test'`, `nix run .#pkl-check-generated`, `nix flake check`.
  - Completed: 2026-03-17
  - Files changed: `context/plans/bash-tool-policy-enforcement.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test'` (`255 passed; 0 failed`); `nix run .#pkl-check-generated`; `nix flake check`; final audit confirmed CLI config handling, generated OpenCode/Claude enforcement assets, setup embedding coverage, and current-state context remain aligned for the bash-policy feature.
  - Context sync classification: verify-only root context pass; this task validated existing implementation and documentation without changing operator-facing behavior.

## Open questions

- None. Scope is clarified: policy-blocked bash commands only, per-policy messages, parity across OpenCode plugins and Claude hooks, plus easy-to-enable built-in presets for common command bans.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test'` -> exit 0 (`255 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`)
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all 4 flake checks evaluated and built successfully: `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`)

### Lint/format coverage

- Covered by `nix flake check` via `cli-clippy` and `cli-fmt`

### Temporary scaffolding

- None removed; no temporary scaffolding was introduced during T08.

### Success-criteria verification

- [x] `.sce/config.json` supports deterministic policy configuration for blocked bash commands with per-policy messages and easy-to-enable preset entries -> covered by CLI config parsing/validation/reporting tests in `cli/src/services/config.rs` and revalidated by `cargo test`
- [x] Policy matching happens before the bash tool executes and blocks the call with a stable, user-visible explanation instead of allowing the command to start -> implemented in generated OpenCode and Claude enforcement assets already documented in `context/overview.md`; generated outputs remain in sync per `nix run .#pkl-check-generated`
- [x] OpenCode and Claude enforce the same configured policies and surface equivalent denial guidance for the same blocked command -> parity is documented in `context/sce/bash-tool-policy-enforcement-contract.md` and preserved by synced generated runtime assets validated through parity checks
- [x] The policy model supports both full-binary blocks and narrower preset blocks for common subcommands -> canonical preset catalog and config contract remain present and linked in context; full test suite passed
- [x] Built-in presets carry fixed repo-owned messages/behavior and do not accept custom per-preset message overrides in config -> enforced by config/runtime contract and validated by existing config tests; no drift detected in generated preset assets
- [x] The Rust config service validates, reports, and documents the new config shape through `sce config show` / `sce config validate` -> covered by `cargo test` and current-state context entries in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/cli/config-precedence-contract.md`
- [x] Generated-owned outputs include the new OpenCode plugin files, Claude hook/config files, and supporting manifests without manual drift -> confirmed by `nix run .#pkl-check-generated`
- [x] Setup/install flows place the new enforcement assets in the correct repo-local target locations for both `.opencode/` and `.claude/` -> existing setup coverage in `cli/src/services/setup/tests.rs` remained green under `cargo test`
- [x] Current-state context explains the policy model, preset catalog, cross-target enforcement approach, and important matching limitations -> verify-only context sync confirmed `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, and `context/sce/bash-tool-policy-enforcement-contract.md` still match code truth

### Failed checks and follow-ups

- None.

### Residual risks

- None identified within this plan scope.
