# Plan: sce-setup-multi-target-installer

## 1) Change summary
Implement `sce setup` so it can install repository config for OpenCode, Claude, or both. The setup flow should use `inquire` for interactive selection by default, support explicit non-interactive flags for automation, and install config by copying embedded assets from the CLI binary into repository-root `.opencode/` and/or `.claude/` with backup-and-replace safety.

## 2) Success criteria
- `sce setup` supports three install targets: OpenCode, Claude, and both.
- Default setup flow is interactive via `inquire` selection.
- Non-interactive automation is supported via explicit flags (`--opencode`, `--claude`, `--both`).
- CLI embeds config assets for both targets at compile time from generated source trees (`config/.opencode/**`, `config/.claude/**`).
- Install operation writes to repository-root targets (`.opencode/`, `.claude/`) from embedded assets and does not depend on runtime access to `config/`.
- Existing target directories are handled with backup-and-replace semantics.
- Tests and command-level verification cover selection flow, flag behavior, overwrite safety, and install parity.

## 3) Constraints and non-goals
- In scope: setup command behavior, selection UX, embedded asset loading, and repository-root install behavior.
- In scope: deterministic copy/install behavior for both target trees.
- In scope: backup-and-replace handling when target directories already exist.
- Out of scope: changing canonical config generation ownership (Pkl remains source for generated target trees).
- Out of scope: implementing runtime network sync, MCP behavior, or hook execution.
- Non-goal: introducing interactive prompts for unrelated commands.

## 4) Task stack (T01..T07)
- [x] T01: Finalize setup command contract and target-selection API (status:done)
  - Task ID: T01
  - Goal: Define CLI contract for `sce setup` interactive and non-interactive modes, including valid combinations and deterministic precedence.
  - Boundaries (in/out of scope):
    - In: command arguments/options schema, target enum/model, precedence rules between prompt and flags, user-facing help text contract.
    - Out: actual file copy implementation.
  - Done when:
    - `setup` contract clearly defines default interactive behavior and explicit flag modes (`--opencode`, `--claude`, `--both`).
    - Invalid argument combinations have deterministic actionable errors.
  - Verification notes (commands or checks):
    - Unit tests for parsing/validation and command-surface help output expectations.

- [x] T02: Add `inquire`-based interactive selection flow (status:done)
  - Task ID: T02
  - Goal: Implement prompt-driven target selection for setup default path.
  - Boundaries (in/out of scope):
    - In: `inquire` dependency integration, prompt model, selected-target mapping to internal setup request.
    - Out: non-interactive install execution internals beyond dispatch wiring.
  - Done when:
    - Running `sce setup` without explicit target flags presents a selection prompt with OpenCode, Claude, and both.
    - Prompt cancellation/interrupt behavior is handled with clear non-destructive exits.
  - Verification notes (commands or checks):
    - Service tests for selection mapping and cancellation behavior.

- [x] T03: Implement compile-time embedded asset manifest for config trees (status:done)
  - Task ID: T03
  - Goal: Bundle `config/.opencode/**` and `config/.claude/**` into the CLI binary and expose a deterministic asset access API.
  - Boundaries (in/out of scope):
    - In: compile-time embedding mechanism, internal asset manifest/index, path normalization rules, target-scoped asset iterators.
    - Out: changing generator ownership or editing generated files directly.
  - Done when:
    - Binary can enumerate embedded files for both targets without reading runtime `config/` paths.
    - Asset API returns stable relative paths and content bytes for install use.
  - Verification notes (commands or checks):
    - Unit tests for embedded manifest completeness and path normalization.

- [x] T04: Build repository-root install engine with backup-and-replace safety (status:done)
  - Task ID: T04
  - Goal: Install selected embedded assets into `.opencode/` and/or `.claude/` using safe replacement semantics.
  - Boundaries (in/out of scope):
    - In: target directory backup strategy, staged write/copy, atomic swap where possible, cleanup/rollback on failure.
    - Out: syncing unrelated root files or deleting non-target directories.
  - Done when:
    - Existing `.opencode/` or `.claude/` is backed up and replaced when selected.
    - Partial-failure scenarios do not leave corrupted target state.
  - Verification notes (commands or checks):
    - Integration-style tests in temp directories validating backup creation, replacement, and rollback behavior.

- [x] T05: Wire setup orchestration and user-facing messaging (status:done)
  - Task ID: T05
  - Goal: Connect parser, selection mode, asset manifest, and install engine through `setup` service contract.
  - Boundaries (in/out of scope):
    - In: command dispatcher wiring, status output messaging, deterministic success/failure reporting.
    - Out: redesigning unrelated command handlers.
  - Done when:
    - `setup` executes end-to-end for interactive and flag-driven flows.
    - Output clearly states selected target(s), backup action, and completion result.
  - Verification notes (commands or checks):
    - Command-level tests/smoke runs for `setup`, `setup --opencode`, `setup --claude`, and `setup --both`.

- [x] T06: Update CLI docs and context for current-state behavior (status:done)
  - Task ID: T06
  - Goal: Reflect implemented setup behavior in crate docs and SCE context files.
  - Boundaries (in/out of scope):
    - In: `cli/README.md` usage updates and context updates for setup command state/terminology.
    - Out: retrospective implementation logs in core context files.
  - Done when:
    - Documentation accurately describes interactive default, flags, embedded assets, and backup-and-replace behavior.
    - Relevant `context/` files reflect current state with no known drift.
  - Verification notes (commands or checks):
    - Manual docs-to-behavior parity check and context consistency pass.

- [x] T07: Validation and cleanup (status:done)
  - Task ID: T07
  - Goal: Run final checks, verify install safety behavior, and close out with clean context alignment.
  - Boundaries (in/out of scope):
    - In: compile/tests/build checks, fixture/temp cleanup, final context sync verification.
    - Out: net-new feature work beyond setup scope.
  - Done when:
    - All success criteria are verified with command/test evidence.
    - Temporary artifacts are removed or intentionally retained with rationale.
    - No known code-context drift remains for this plan scope.
  - Verification notes (commands or checks):
    - `cargo fmt --check && cargo check && cargo test && cargo build` (from `cli/`).
    - Focused setup safety checks covering overwrite/backup scenarios.
    - Final context sync verification across updated context files.

## 5) Open questions
- None.

## 6) Execution evidence (T06)

- Updated `cli/README.md` to reflect current `setup` behavior as implemented today:
  - interactive default target selection (`OpenCode`, `Claude`, `Both`)
  - mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`)
  - compile-time embedded assets from `config/.opencode/**` and `config/.claude/**`
  - repository-root install destinations (`.opencode/`, `.claude/`) with backup-and-replace and rollback safety
- Added repository-level verification guidance to `cli/README.md` documenting that `nix flake check` runs targeted setup command-surface checks.
- Context sync updates were applied to keep behavior discoverable and current-state aligned: `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` now document the new flake check contract for targeted CLI setup command-surface verification.

## 7) Validation report (T07)

- Commands and evidence:
  - `nix flake check` (from repository root)
  - check `checks.x86_64-linux.cli-setup-command-surface` now runs from `cli/` with:
    - `cargo fmt --check`
    - `cargo test command_surface::tests::help_text_mentions_setup_target_flags`
    - `cargo test parser_routes_setup`
    - `cargo test run_setup_reports`
- Result:
  - Exit code `0`
  - Flake check completed with expected non-blocking warning about omitted incompatible systems unless `--all-systems` is used.
- Cleanup/status:
  - No new temporary artifacts required retention.
  - Code and context are aligned for T06/T07 scope.
