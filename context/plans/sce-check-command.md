# Plan: `sce check` command — deterministic project check runner

## Change summary

Add a new `sce check` CLI command that runs project-configured test/lint/build/format checks from `.sce/config.json`. This gives the SCE workflow a deterministic, version-controlled way to verify project code quality at every step, instead of having agent skills guess commands by ad-hoc reading of `package.json`, `Makefile`, or CI config.

The three SCE agent skills (`sce-plan-authoring`, `sce-task-execution`, `sce-validation`) are updated to consume `sce check --list --format json` and `sce check <name>` instead of hardcoded or discovered commands.

No `sce check validate` subcommand is created — `sce config validate` already covers config parseability and `sce doctor` covers environment health.

## Success criteria

1. A new `sce check` command is registered and invocable: `sce check`, `sce check --help`
2. Checks are configurable via a `checks` key in `.sce/config.json`, validated by `sce config validate`
3. `sce check --list --format json` returns a structured list of available check names + commands
4. `sce check <name>` runs the configured command and reports exit code + output in text and JSON
5. `sce check --all` runs every configured check and aggregates results
6. Fallback discovery detects Cargo, npm, Makefile, flake.nix when no `checks` config exists
7. `sce-plan-authoring` skill runs `sce check --list --format json` during planning and uses discovered names in task verification notes
8. `sce-task-execution` skill runs `sce check --list --format json` during the implementation stop phase, selects the most targeted check, and runs it
9. `sce-validation` skill runs `sce check --list --format json` then `sce check --all` for final validation
10. `nix flake check` passes; `nix run .#pkl-check-generated` passes

## Constraints and non-goals

- No `sce check validate` subcommand is created — `sce config validate` covers config, `sce doctor` covers environment
- No parallel check execution in this plan — checks run sequentially
- No interactive check selection — selection logic lives in agent skills, not the CLI
- No built-in check runners (e.g., SCE does not run your tests itself) — the CLI shells out to configured commands
- No timeout enforcement in this plan — deferred to a future task if needed
- No changes to `sce doctor` — environment health is already covered
- The `checks` schema key is added to Pkl sources and regenerated, then consumed by the CLI

## Check config shape

Each check in `.sce/config.json` accepts two forms:

1. **String shorthand** — `"test": "cargo test"` (command only, timeout uses check-specific default)
2. **Object form** — full control with optional fields:

| Field | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | Shell command to execute |
| `description` | string | no | Human-readable purpose, surfaced by `--list --format json` |
| `timeout` | string | no | Max duration: `"30s"`, `"5m"`, `"2m30s"`. Defaults to check-specific value (see below) |

All checks support the object form. The string shorthand is syntactic sugar equivalent to `{ "command": "..." }` with default timeout.

Example:
```json
{
  "$schema": "https://sce.crocoder.dev/config.json",
  "checks": {
    "test": "cargo test",
    "lint": {
      "command": "cargo clippy -- -D warnings",
      "timeout": "5m"
    },
    "format": "cargo fmt --check",
    "build": "cargo build",
    "pkl-parity": {
      "command": "nix run .#pkl-check-generated",
      "description": "Verify generated config parity with canonical Pkl sources",
      "timeout": "2m"
    }
  }
}
```

### Standard checks

Standard checks are well-known names the CLI understands with defined semantics and default timeout values.

| Standard name | Semantics | Default timeout | Fallback discovery (T05) |
|---|---|---|---|
| `test` | Run project test suite | `10m` | Cargo → `cargo test`; npm → `npm test`; Bun → `bun test` |
| `lint` | Run linter | `2m` | Cargo → `cargo clippy`; detect eslint, biome, ruff |
| `format` | Check formatting | `1m` | Cargo → `cargo fmt --check`; detect prettier, biome |
| `build` | Verify project compiles/builds | `10m` | Cargo → `cargo build`; npm → `npm run build` |
| `typecheck` | Run type checker | `3m` | TypeScript → `tsc --noEmit`; detect mypy, pyright |

**Non-standard checks** (any name outside the standard set, e.g. `pkl-parity`, `integration`, `test:t01`, `docs-check`) default to **`3m`** timeout. They use the object form with optional `description` and optional `timeout` so `--list --format json` output is meaningful to agent skills.

**Timeout behavior:** When a check exceeds its timeout, the CLI kills the subprocess and reports a timeout failure with exit code 124 (standard timeout exit code) and a clear `TIMEOUT after <duration>` message. The check is marked as failed in both text and JSON output.

## Task stack

### T01: Add `checks` key to SCE config schema

- **Goal**: Extend the canonical Pkl-authored config schema with a `checks` block and regenerate, so `sce config validate` accepts `checks`. Each check supports command string, optional description, and optional timeout (string like `"30s"`, `"5m"`, `"2m30s"`).
- **Boundaries**: In — `config/pkl/base/sce-config-schema.pkl`, JSON schema output at `config/schema/sce-config.schema.json`, Rust config types in `cli/src/services/config/mod.rs` (add `TOP_LEVEL_CONFIG_KEYS` entry, parse `ParsedChecksConfigDocument` with timeout parsing from human-readable duration strings, add `checks` field to `FileConfig`/`RuntimeConfig`), regenerate with `nix develop -c pkl eval -m . config/pkl/generate.pkl`. Out — no CLI command yet, no fallback detection, the checks value is stored but unused until T02.
- **Done when**: `sce config validate` accepts valid `checks` config (including both string and object forms with optional `description` and `timeout`) without error; `sce config show` reports checks (if configured); `nix run .#pkl-check-generated` passes.
- **Verification notes**:
  - `nix flake check`
  - `nix run .#pkl-check-generated`
  - Manual: write a `.sce/config.json` with a `checks` block, run `sce config validate`

### T02: Add `sce check` command skeleton (parser, registry, help, --list, --all)

- **Goal**: Register the `sce check` command in the CLI schema, parser, and command registry. Implement `--list`, `--all`, and single-name parsing. Wire `--format <text|json>`.
- **Boundaries**: In — `cli/src/cli_schema.rs` (add `Check` variant + `CheckSubcommand`), `cli/src/services/command_registry.rs` (register check command), `cli/src/services/check/` (new service module with `mod.rs`, `command.rs`), `cli/src/services/check/mod.rs` (parse: `CheckRequest`, `CheckSubcommand::List | All | Single(Vec<String>)`), `cli/src/services/check/command.rs` (`CheckCommand` struct + `RuntimeCommand` impl), `cli/src/command_surface.rs` (add help section). Out — no actual check execution yet, no config loading, no fallback discovery.
- **Done when**: `sce check`, `sce check --help`, `sce check --list`, `sce check --all`, `sce check --list --format json`, `sce check --all --format json` all return valid output. `sce check foo` returns an actionable error ("Unknown check 'foo'. Run `sce check --list` to see available checks").
- **Verification notes**:
  - `cargo build` succeeds
  - `sce check --help` displays usage
  - `sce check --list --format json` returns `{"status":"ok","result":{"command":"check_list","checks":[]}}` (empty list since no config loaded yet)
  - `sce check foo` exits non-zero with clear error

### T03: Implement check runner — execute command, capture exit code + output

- **Goal**: When `sce check <name>` or `sce check --all` is invoked, load the checks config from `.sce/config.json`, find the matching command(s), execute them with timeout enforcement, capture stdout/stderr and exit code, and render results in text and JSON.
- **Boundaries**: In — `cli/src/services/check/runner.rs` (executes shell commands with timeout, kills subprocess on timeout, captures output), `cli/src/services/check/mod.rs` (load checks from resolved config, match names, resolve timeout from config or check-specific default or 3m non-standard default, delegate to runner), `cli/src/services/config/mod.rs` or `cli/src/services/check/` (expose loaded `ParsedChecksConfig` through a helper). Out — fallback discovery (T05), parallel execution.
- **Done when**: With a `.sce/config.json` containing `{"checks":{"test":"echo ok", "slow":{"command":"sleep 10","timeout":"1s"}}}`:
  - `sce check test` prints the check result and exits 0
  - `sce check test --format json` returns structured JSON with exit code and output
  - `sce check --all` runs both checks and reports each result
  - `sce check slow` exits 124 with `TIMEOUT after 1s` message
  - `sce check missing` exits non-zero with "Unknown check"
- **Verification notes**:
  - Manual test with local `.sce/config.json`
  - Unit tests for runner (exit code capture, stdout/stderr capture, non-zero exit propagation, timeout kill + exit 124)
  - `cargo test` in `cli/` passes

### T04: Implement check runner — --list --format json output with check metadata

- **Goal**: `sce check --list --format json` returns each check's name, configured command, and optionally a description field (if present in config). The agent skills parse this to discover what verification commands are available.
- **Boundaries**: In — `cli/src/services/check/mod.rs` (list rendering in text and JSON), config parsing exposes command + optional description per check. Out — no changes to this task in the agent skills themselves (T06, T07, T08).
- **Done when**:
  - Text `sce check --list` output is human-readable with name/command columns
  - JSON output includes `checks: [{name, command, description?}]`
- **Verification notes**:
  - `nix flake check`
  - Manual with sample config

### T05: Implement fallback discovery

- **Goal**: When no `checks` key is configured in `.sce/config.json`, `sce check --list` auto-discovers commands from project build files: `Cargo.toml` → `cargo test`, `cargo clippy`, `cargo fmt --check`; `package.json` → `npm test`, detect eslint/prettier/biome scripts; `Makefile` → `make test`, `make lint`; `flake.nix` → `nix flake check`. `sce check` prefers configured checks over fallback.
- **Boundaries**: In — `cli/src/services/check/discovery.rs` with detection order: config > flake.nix > Cargo.toml > package.json > Makefile. Out — only one fallback set is used (first match wins); no merging of multiple fallback sources.
- **Done when**: In a repo with only a `Cargo.toml` and no `checks` config, `sce check --list` shows `test`, `lint`, `format`, `build` checks with `cargo` commands. In a repo with `checks` config, fallback is ignored.
- **Verification notes**:
  - Unit tests for each detector
  - Manual test in repo without checks config
  - `nix flake check`

### T06: Update `sce-plan-authoring` skill to consume `sce check --list`

- **Goal**: The `sce-plan-authoring` SKILL.md is updated with instructions to run `sce check --list --format json` during plan authoring and reference discovered check names in task verification notes.
- **Boundaries**: In — `.opencode/skills/sce-plan-authoring/SKILL.md` (add Discovery section, update Task format section with `sce check` references). Out — changes to generated OpenCode content (covered by regeneration in T09), no changes to CLI code.
- **Done when**: The SKILL.md includes explicit instructions for running `sce check --list --format json` and using discovered check names in verification notes, with examples.
- **Verification notes**:
  - Content inspection of updated SKILL.md

### T07: Update `sce-task-execution` skill to consume `sce check --list`

- **Goal**: The `sce-task-execution` SKILL.md is updated to run `sce check --list --format json` during the implementation stop phase, select the most targeted check, run `sce check <name>`, and capture evidence.
- **Boundaries**: In — `.opencode/skills/sce-task-execution/SKILL.md` (update Required sequence steps 5-8 to reference `sce check --list` and `sce check <name>`). Out — no changes to CLI code.
- **Done when**: The SKILL.md's Required sequence includes explicit discovery, selection, and execution steps using `sce check`.
- **Verification notes**:
  - Content inspection of updated SKILL.md

### T08: Update `sce-validation` skill to consume `sce check --all`

- **Goal**: The `sce-validation` SKILL.md is updated to run `sce check --list --format json` then `sce check --all` for deterministic final validation, replacing the current ad-hoc "discover and run" behavior.
- **Boundaries**: In — `.opencode/skills/sce-validation/SKILL.md` (update Validation checklist steps 1-2 to reference `sce check --list` and `sce check --all`). Out — no changes to CLI code.
- **Done when**: The Validation checklist references `sce check --list`, `sce check --all`, and `sce check <name>` for re-running individual failing checks.
- **Verification notes**:
  - Content inspection of updated SKILL.md

### T09: Regenerate generated agent content and final validation

- **Goal**: Regenerate OpenCode/Claude agent and command outputs so the updated skill SKILL.md content propagates to generated targets. Run `nix flake check` and `nix run .#pkl-check-generated`. Update `context/context-map.md` with discoverability links for the new `sce check` service module and relevant context files.
- **Boundaries**: In — `nix develop -c pkl eval -m . config/pkl/generate.pkl`, `nix run .#pkl-check-generated`, `nix flake check`, `context/context-map.md` update. Out — no CLI code changes, no new context files beyond context-map updates.
- **Done when**: `nix flake check` passes, `nix run .#pkl-check-generated` passes, `context/context-map.md` has entries for `sce check` command surface and service module.
- **Verification notes**:
  - `nix flake check`
  - `nix run .#pkl-check-generated`
  - Content inspection of context-map.md

---

## Open questions

None — all design decisions are settled per conversation with the human:

- No `sce check validate` subcommand — `sce config validate` covers config, `sce doctor` covers environment
- No parallel execution in this plan
- Three skills updated: plan-authoring, task-execution, validation
- Fallback discovery is a single-source first-match strategy
