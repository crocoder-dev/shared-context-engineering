# Plan: interactive-setup-behavior-prompts

## Change summary

Extend the existing interactive `sce setup` prompt/dispatch seam with two
independent confirmation questions for the user-facing Agent Trace behaviors:
automatic post-commit synchronization and SCE commit-attribution trailers.
Both confirmations use the existing `inquire` prompt boundary, display the
requested `[Y/n]` defaults, and treat Enter as `true`; explicit `n` produces
`false`.

Carry the selected values through setup and persist them as explicit nested
values in repo-local `.sce/config.json`, while retaining the current JSON merge,
formatting, and create-if-missing behavior. Non-interactive setup remains
prompt-free and deterministic: a newly created config explicitly enables both
behaviors, while an existing config is not rewritten merely because setup is
non-interactive. The existing config-only auto-sync resolver, attribution
environment/config precedence, `SCE_DISABLED` handling, post-commit launcher,
and canonical trailer semantics remain unchanged; setup only supplies the
selected persisted configuration.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: Interactive setup asks two separate questions with the exact labels
  `Enable automatic Agent Trace synchronization? [Y/n]` and
  `Enable SCE commit attribution trailers? [Y/n]`; Enter selects `true` for
  each independently, and `n` selects `false` for only the corresponding
  behavior.
  - Validate: setup prompter/dispatch tests using the test prompter seam cover
    both default-Yes answers and each independent explicit-No combination;
    prompt construction inspection confirms the exact labels and default.
- [x] AC2: A completed setup persists the selected values at
  `agent_trace.auto_sync` and `policies.attribution_hooks.enabled`, preserving
  unrelated config keys and the existing pretty-JSON/newline merge behavior.
  - Validate: setup persistence tests assert the nested JSON values for true and
    false selections and assert existing keys/formatting and both unrelated
    behavior values are preserved according to the existing-config policy.
- [x] AC3: Non-interactive setup never waits for prompts; a newly created
  repo-local config explicitly contains both behavior values as `true`, while
  an existing config's explicitly configured values and omitted keys remain
  untouched. The persisted auto-sync value controls the existing post-commit
  launch gate, and the persisted attribution value controls the existing
  canonical trailer gate without changing either hook's other semantics.
  - Validate: setup tests cover newly created and existing-config
  non-interactive runs; focused hook tests cover auto-sync enabled/disabled and
  attribution enabled/disabled paths using the existing injected seams.
- [x] AC4: Resolver behavior remains unchanged: `agent_trace.auto_sync` is
  config-file-only with its existing omitted-value fallback and global-before-
  local merge, while attribution enablement retains its existing
  `SCE_ATTRIBUTION_HOOKS_DISABLED` over config precedence, default, and
  `SCE_DISABLED` interaction.
  - Validate: focused `config::` tests cover omitted, explicit true/false,
  global/local, environment-over-config, and `SCE_DISABLED`-compatible hook
  gate behavior for both properties.
- [x] AC5: User-facing setup documentation and durable SCE context explain the
  two interactive choices, explicit persisted config shape, Enter-as-Yes
  behavior, safe existing-config handling, and unchanged runtime precedence and
  hook semantics; no generated artifact is edited manually.
  - Validate: manual review of the updated README/context against the setup,
  resolver, and hook implementations; `nix flake check` confirms generated
  surfaces remain valid.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::`
- `nix flake check`

### Context sync

- `README.md`
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/cli/cli-command-surface.md`
- `context/cli/config-precedence-contract.md`
- `context/cli/agent-trace-auto-sync.md`
- `context/sce/setup-repo-local-config-bootstrap.md`
- `context/sce/agent-trace-hooks-command-routing.md`
- `context/sce/agent-trace-commit-msg-coauthor-policy.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** the existing setup prompt/profiler seam and interactive dispatch;
  setup behavior-selection data flow; repo-local config persistence and the
  create-if-missing bootstrap payload; focused setup, resolver, and existing
  hook-gate regression tests; user-facing setup documentation and the listed
  durable context.
- **Out of scope:** changing the JSON schema shape or generated artifacts,
  adding CLI flags or environment variables, changing config precedence or
  fallback values, changing post-commit synchronization/trailer/hook execution
  semantics, changing the sync protocol/database, or redesigning setup target
  and optional-workflow selection.
- **Constraints:** preserve exact nested keys, existing pretty JSON formatting
  and trailing newline, existing unrelated config keys, target/optional-workflow
  merge behavior, cancellation/non-TTY behavior, and the current Nix-based
  validation commands. Use the existing `inquire` dependency and service-layer
  prompter seam; do not edit generated outputs manually.
- **Non-goal:** do not make setup a new runtime policy resolver or add a second
  control path for automatic sync or attribution.

## Assumptions

- An interactive answer is an intentional operator selection, including Enter
  accepting the required Yes default; it may replace an existing explicit
  behavior value. Non-interactive setup has no such new selection and therefore
  preserves an existing config file's explicit values and omissions, following
  the current create-if-missing/additive setup convention.
- The existing setup flow continues to run repository preflights and resolve
  prompts before side effects; the two behavior confirmations follow the target
  and optional-workflow selection and share its existing cancellation handling.
- Existing config files that are invalid remain byte-preserved under the current
  degraded setup behavior, so behavior persistence is skipped for that run
  rather than attempting to repair or rewrite the file.
- The existing hook tests and injected post-commit/commit-msg seams are the
  appropriate proof that persisted values control runtime behavior; no new
  integration harness or hook protocol is needed.

## Task stack

- [x] T01: `Add independent interactive behavior confirmations to setup dispatch` (status:complete)
  - Task ID: T01
  - Scope: In — extend the service-layer setup prompter seam and `SetupDispatch`
    with the two boolean selections, implement the two exact `inquire` confirm
    prompts with default `true`, preserve target/optional-workflow ordering and
    cancellation/non-TTY behavior, and add fake-prompter dispatch tests for
    default-Yes, each explicit-No, and independence. Out — config writes,
    resolver changes, hook runtime changes, and documentation.
  - Dependencies: none
  - Done when: interactive dispatch asks both prompts exactly once after the
    existing setup selections, carries both independent booleans, Enter/default
    maps to true, explicit false maps only its own field, and cancellation still
    returns the existing non-destructive outcome.
  - Verify: targeted setup tests through `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
   - Completed: 2026-09-09
   - Files changed:
     - `cli/src/services/setup/command.rs`
     - `cli/src/services/setup/mod.rs`
   - Result: Added independent interactive confirmations for automatic Agent Trace synchronization and SCE commit-attribution trailers, carried both selections through `SetupDispatch`, preserved cancellation and non-interactive behavior, and added dispatch coverage for defaults, independent explicit-No selections, and cancellation.
   - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` — passed (74 tests).
   - Context impact: local — setup prompt dispatch and its focused tests changed; no durable context files or runtime resolver semantics were changed.
   - Context synchronization: synced

- [x] T02: `Persist setup behavior selections without changing runtime semantics` (status:complete)
  - Task ID: T02
  - Scope: In — add the setup behavior-selection config merge using the existing
    repo-local JSON persistence conventions; include explicit `true` defaults for
    both properties in a newly created config; thread interactive selections into
    the successful setup write; leave existing non-interactive config values and
    omissions untouched; add setup persistence/bootstrap tests and resolver/hook
    regressions proving the selected values reach the existing auto-sync and
    attribution gates. Out — schema shape changes, new precedence layers,
    changes to launcher/trailer algorithms, and generated artifacts.
  - Dependencies: T01
  - Done when: the exact nested config shape is written for selected values,
    unrelated keys and existing merge formatting survive, new configs contain
    both true values, existing configs are not silently changed by non-interactive
    setup, invalid discovered configs remain byte-preserved, and existing resolver
    precedence plus post-commit/commit-msg gate semantics pass focused tests.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::`.
  - Completed: 2026-09-09
  - Files changed:
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/mod.rs`
  - Result: Threaded interactive behavior selections through setup persistence, added explicit true bootstrap values for both Agent Trace synchronization and attribution hooks, preserved existing non-interactive omissions and invalid-config byte preservation, and added persistence/bootstrap regression coverage.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` — passed (78 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` — passed (58 tests).
  - Context impact: local — setup persistence/bootstrap and focused regression tests changed; runtime resolver and hook semantics were preserved and no durable context files were changed.
   - Context synchronization: synced

- [x] T03: `Document interactive setup behavior choices and persistence` (status:complete)
  - Task ID: T03
  - Scope: In — update the root quick-start/setup documentation and the listed
    current-state context files to describe the two prompts, defaults, explicit
    nested config values, existing-config safety rule, and unchanged resolver and
    hook contracts. Out — application code, tests, generated target/schema
    outputs, and historical completed plan/decision records.
  - Dependencies: T02
  - Done when: setup documentation accurately shows the two independent
    confirmations and config shape, durable context consistently distinguishes
    interactive selection from non-interactive bootstrap and preserves the
    existing precedence/runtime semantics, and no generated artifact is manually
    modified.
  - Verify: manual review against the implemented setup/config/hooks code and
    the plan's full validation commands.
  - Completed: 2026-09-09
  - Files changed:
    - `README.md`
    - `context/overview.md`
    - `context/architecture.md`
    - `context/patterns.md`
    - `context/glossary.md`
    - `context/context-map.md`
    - `context/cli/cli-command-surface.md`
    - `context/cli/config-precedence-contract.md`
    - `context/cli/agent-trace-auto-sync.md`
    - `context/sce/setup-repo-local-config-bootstrap.md`
    - `context/sce/agent-trace-hooks-command-routing.md`
    - `context/sce/agent-trace-commit-msg-coauthor-policy.md`
  - Result: Documented the independent interactive setup confirmations, explicit nested persistence shape, Enter-as-Yes defaults, safe non-interactive existing-config behavior, and unchanged resolver and hook contracts across the quick-start and durable context surfaces.
  - Verify: manual review against setup/config/hooks implementation — passed; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` — passed (78 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` — passed (58 tests); `nix flake check` — passed (all checks passed).
  - Context impact: root — user-facing setup behavior and persistence contracts are cross-cutting durable knowledge; updated the listed root, CLI, and SCE context surfaces.
  - Context synchronization: synced

## Open questions

None. The request fixes the prompt text, default behavior, persisted schema
shape, runtime precedence/non-goals, existing-config safety requirement, test
coverage, and required validation; the remaining choice that Enter is an
intentional interactive answer follows the existing prompt conventions.

## Validation Report

**Status:** validated  
**Date:** 2026-09-09

### Commands run

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` -> exit 0 (78 setup tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` -> exit 0 (58 config tests passed)
- `nix flake check` -> exit 0 (all checks passed)
- `git diff --check` -> exit 0 (no whitespace errors)

### Success-criteria verification

- [x] AC1: Interactive setup asks two exact, independent Yes-default confirmation questions and carries default/explicit-No selections independently -> setup tests passed for both defaults, each independent No combination, both No, and cancellation; prompt construction inspection confirmed the exact labels and `with_default(true)`.
- [x] AC2: Setup persists the selected nested behavior values while preserving unrelated keys and formatting -> setup persistence and bootstrap tests passed, including true/false selections, preserved config content, omissions, and invalid-config byte preservation.
- [x] AC3: Non-interactive setup is prompt-free, bootstraps both values only for a new config, and existing gates retain their semantics -> setup tests passed for new/existing configs; full flake checks passed the injected auto-sync and attribution hook gate regressions.
- [x] AC4: Resolver precedence, omitted fallback, environment overrides, and `SCE_DISABLED` behavior remain unchanged -> config tests passed for omitted/explicit values, global/local precedence, attribution environment precedence, and disabled-hook interactions.
- [x] AC5: Documentation and durable context describe the setup choices, persistence shape, safety behavior, and unchanged runtime contracts -> manual review matched the setup/config/hooks implementation; `nix flake check` passed generated-surface validation.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
