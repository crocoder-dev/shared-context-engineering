# Plan: Claude Bash Policy Rust Hook

## Change summary

Migrate SCE bash-tool policy enforcement from the OpenCode TypeScript runtime into the Rust `sce` CLI, then expose it to Claude Code through a generated `PreToolUse` command hook. Keep OpenCode and Claude on the same Rust policy engine so both targets enforce the same configured `policies.bash` presets/custom rules, command parsing, shell-operator handling, nested `nix ... -c` / `sh|bash -c` unwrapping, policy precedence, and denial message format.

## Success criteria

- Claude generated `.claude/settings.json` includes a `PreToolUse` hook for the `Bash` tool that invokes `sce policy ...` in command-hook exec form.
- The Claude hook denies blocked commands using Claude Code hook JSON with `hookSpecificOutput.hookEventName = "PreToolUse"`, `permissionDecision = "deny"`, and the canonical SCE policy denial reason.
- Bash policy evaluation is owned by Rust under `cli/`; the OpenCode TypeScript implementation no longer owns independent policy logic.
- OpenCode and Claude both delegate to the same Rust policy behavior and remain parity-equivalent for current bash-policy tests.
- Generated outputs are produced from canonical Pkl/source inputs rather than manual generated-file edits.
- Obsolete generated Claude bash-policy absence statements in context are repaired after implementation.

## Constraints and non-goals

- In scope: Rust policy engine, CLI policy command surface, generated Claude hook configuration, OpenCode delegation to the Rust engine, tests, generated-output parity, and context sync.
- Out of scope: changing the `policies.bash` config schema or preset catalog semantics, adding new preset IDs, broadening policy matching beyond the current OpenCode parity contract, or changing Agent Trace hooks.
- Generated outputs under `config/.claude/**`, `config/.opencode/**`, and `config/automated/.opencode/**` must be regenerated from `config/pkl/**` and `config/lib/**` sources.
- Hook behavior should fail open only for malformed/unparseable policy input/config in the same circumstances the current OpenCode runtime allows commands; actual matching policy denials must block before subprocess execution.
- CLI stdout/stderr contracts remain important: hook decision JSON must be emitted only as the intended command payload, with diagnostics routed predictably.

## Assumptions

- The implementation may introduce a new hidden/internal `sce policy` command family if no suitable command exists; the exact subcommand name should be stable and documented in tests/context.
- The `sce policy` command should support Claude raw `PreToolUse` event JSON from STDIN and a normalized request shape for OpenCode delegation so both integrations can call the same Rust engine without duplicating parsing logic.
- OpenCode can keep a thin generated TypeScript plugin entrypoint whose only policy responsibility is invoking `sce policy`; the business logic should move to Rust.

## Task stack

- [x] T01: `Port bash-policy evaluator to Rust` (status:done)
  - Task ID: T01
  - Goal: Implement a Rust-owned bash-policy evaluator that matches the current OpenCode TypeScript runtime behavior.
  - Boundaries (in/out of scope): In - command tokenization, shell segment splitting for `|`, `&&`, `||`, `;`, `&`, wrapper/env stripping, executable basename normalization, nested `nix ... -c` / `nix ... --command` and `sh|bash -c` recursion, preset/custom active-policy construction, longest-prefix/custom-over-preset precedence, canonical block-message formatting, focused Rust tests ported from the current Bun suite. Out - CLI command routing, Claude hook output JSON, generated config changes.
  - Done when: Rust tests cover allowed/blocked decisions, custom-policy precedence, malformed config/catalog fail-open cases, shell operators, nested shell/nix parsing, and canonical `Blocked by SCE bash-tool policy '<id>': <message>` formatting.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test bash_policy'`; inspect that policy logic imports the existing config/preset data rather than hardcoding a divergent catalog.
  - Completed: 2026-06-12
  - Files changed: `cli/src/services/bash_policy.rs`, `cli/src/services/config/policy.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed. Direct `nix develop -c sh -c 'cd cli && cargo test bash_policy'` was blocked by the repo's active bash policy (`use-nix-flake-check-over-cargo-test`), so the equivalent Nix CLI test check was used.
  - Notes: Added a CLI-agnostic Rust evaluator that reuses the existing embedded bash-policy preset catalog from `config::policy`; T02 remains responsible for exposing it through `sce policy`.

- [x] T02: `Add sce policy command adapter` (status:done)
  - Task ID: T02
  - Goal: Expose the Rust evaluator through a deterministic `sce policy` CLI surface suitable for hook callers.
  - Boundaries (in/out of scope): In - command schema/dispatch for the policy subcommand, STDIN parsing for Claude `PreToolUse` raw hook JSON and an SCE-normalized bash-policy request, worktree/project-root resolution, hook-safe output modes including Claude deny JSON and machine-readable OpenCode/diagnostic JSON, stable invocation validation errors. Out - generated Claude settings, OpenCode plugin changes, policy matching behavior already covered by T01.
  - Done when: A blocked Claude `Bash` event produces a valid Claude Code deny decision on stdout; an allowed event exits successfully without a denial decision in Claude hook mode; normalized OpenCode calls can receive a structured allow/block result; invalid invocation/input returns deterministic validation diagnostics without executing target commands.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test policy'`; manual fixture checks with representative Claude `PreToolUse` JSON for allowed and blocked commands.
  - Completed: 2026-06-12
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/bash_policy.rs`, `cli/src/services/command_registry.rs`, `cli/src/services/config/mod.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/mod.rs`, `cli/src/services/parse/command_runtime.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, all JS checks); `nix run .#pkl-check-generated` passed; targeted policy adapter tests cover Claude PreToolUse parsing, normalized request parsing, Claude deny JSON rendering, allowed empty output, and normalized JSON result rendering; Rust evaluator tests now achieve parity with the OpenCode TypeScript test suite covering malformed custom policies (empty id/message/argv_prefix/empty-string elements), parseCommandSegments edge cases (empty string, single token, only operators, trailing operator, consecutive operators, quoted arguments, unclosed quotes), shell operator policy tests (||, &, first-matching-segment argv reporting), and standalone sh -c policy evaluation.
  - Design decision: The `sce policy bash` command uses explicit `--input` (default: `claude-pre-tool-use`) and `--output` (default: `claude-hook`) flags rather than auto-detection or a `--caller` identity flag. Claude Code hooks invoke `sce policy bash` with defaults; OpenCode plugin delegation (T03) will pass `--input normalized --output json`. This keeps the interface deterministic and avoids heuristic format sniffing.
  - Notes: The `policy` command is hidden from top-level help (`POLICY_SHOW_IN_TOP_LEVEL_HELP = false`). Config resolution falls back from git root to current directory when not in a git repo. The `resolve_bash_policy_runtime_config` function was added to the config resolver to support policy evaluation without requiring the full observability config resolution path.

- [x] T03: `Delegate OpenCode bash-policy plugin to sce policy` (status:done)
  - Task ID: T03
  - Goal: Replace OpenCode TypeScript policy business logic with a thin generated plugin wrapper that calls the Rust `sce policy` command.
  - Boundaries (in/out of scope): In - update `config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts` to invoke `sce policy` with a normalized request, preserve OpenCode denial text/throw behavior, remove or shrink TypeScript runtime ownership that duplicates Rust logic, adapt JS tests to wrapper behavior where still useful. Out - Claude hook generation and Rust evaluator behavior.
  - Done when: OpenCode generated plugin behavior still blocks/permits according to the current contract, but policy parsing/matching is not duplicated in TypeScript; generated OpenCode outputs point at the wrapper and no longer emit stale standalone runtime logic as an authority.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd config/lib && bun test bash-policy-plugin'`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-06-12
  - Files changed: `config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts`, `config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`, `config/pkl/generate.pkl`, `flake.nix`, `.opencode/plugins/sce-bash-policy.ts`, removed `config/lib/bash-policy-plugin/bash-policy/runtime.ts`, removed `config/.opencode/plugins/bash-policy/runtime.ts`, removed `config/automated/.opencode/plugins/bash-policy/runtime.ts`, removed `.opencode/plugins/bash-policy/runtime.ts`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format, all JS checks); `nix run .#pkl-check-generated` passed; 12 Bun tests pass covering allow/deny/fail-open behavior and input format validation.
  - Notes: The OpenCode bash-policy plugin now delegates to `sce policy bash --input normalized --output json` via `spawnSync` instead of importing a TypeScript runtime. The TypeScript runtime (`bash-policy/runtime.ts`) has been removed entirely. The Pkl generator no longer emits `bash-policy/runtime.ts`. The plugin fails open (allows commands) when `sce` is not found, exits non-zero, returns empty stdout, or returns invalid JSON. The `repoRoot` parameter is no longer needed since `sce policy bash` resolves the project root itself.

- [ ] T04: `Generate Claude PreToolUse bash-policy hook` (status:todo)
  - Task ID: T04
  - Goal: Add generated Claude Code settings that enforce bash-policy through the Rust `sce policy` hook path.
  - Boundaries (in/out of scope): In - update `config/pkl/renderers/claude-content.pkl` settings rendering, add a `PreToolUse` matcher for `Bash`, use command-hook exec form with `command: "sce"` and explicit args, preserve existing Claude Agent Trace `SessionStart` and `PostToolUse` hooks, regenerate `config/.claude/settings.json` and repo-root `.claude/settings.json` if applicable. Out - changing Agent Trace hook behavior, adding Claude TypeScript policy runtime files.
  - Done when: Generated `.claude/settings.json` includes both existing Agent Trace hooks and the new Bash `PreToolUse` policy hook; the hook command routes raw Claude event JSON to `sce policy`; generated-output parity passes.
  - Verification notes (commands or checks): Inspect generated settings; `nix run .#pkl-check-generated`; use Claude hook JSON fixtures to verify deny/allow behavior through the CLI command from T02.

- [ ] T05: `Clean up obsolete TypeScript policy ownership and docs` (status:todo)
  - Task ID: T05
  - Goal: Remove stale source/generated references that claim Claude bash-policy is absent or that TypeScript owns policy enforcement.
  - Boundaries (in/out of scope): In - remove obsolete generated runtime files from generation mappings if replaced by Rust, update context files such as `context/sce/generated-opencode-plugin-registration.md`, `context/sce/bash-tool-policy-enforcement-contract.md`, `context/overview.md`, `context/architecture.md`, and `context/glossary.md` to current-state wording after code changes, and add/update focused context for the new Rust policy command contract if needed. Out - broad unrelated context rewrites or historical plan cleanup.
  - Done when: Durable context accurately states that Rust owns bash-policy evaluation and Claude/OpenCode both call the `sce policy` path; no context file still states Claude bash-policy enforcement is removed/absent as current behavior.
  - Verification notes (commands or checks): Search context/generated sources for `Claude bash-policy enforcement has been removed`, `OpenCode is now the sole target`, and stale TypeScript runtime ownership claims; verify replacements reflect code truth.

- [ ] T06: `Validate migration and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run final repository validation and ensure the migration is coherent across Rust, generated config, OpenCode wrapper, Claude settings, and context.
  - Boundaries (in/out of scope): In - full repo validation, generated parity, targeted Rust/Bun checks if not already run, cleanup of temporary fixtures/artifacts, final review of plan status and context sync. Out - new feature work or additional policy semantics.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; any narrow checks from earlier tasks pass or have documented reasons; no temporary files remain; the plan records completion evidence.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; optional final smoke fixtures for Claude deny JSON and OpenCode wrapper delegation.

## Open questions

- Exact CLI spelling for the new policy command should be finalized during T02 and then kept stable in generated hooks and context. Recommended default: a hidden/internal `sce policy bash` subcommand with explicit hook/output mode flags.
