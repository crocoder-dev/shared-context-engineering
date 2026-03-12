# Plan: Nix Flake Checks Refactor

## Change summary
Refactor `cli/flake.nix` and root `flake.nix` to have a cleaner, more idiomatic Nix flake structure with proper separation between builds and checks.

**cli/flake.nix changes:**
- Remove the `mkCheck` helper that rebuilds packages just for checks
- Add separate check derivations: `cli-tests`, `cli-clippy`, `cli-fmt`
- Remove the `clippy` app (redundant with check)
- Keep package build with `doCheck = false` (tests run as separate checks)

**root flake.nix changes:**
- Re-export all CLI checks with consistent naming
- Add `pkl-parity` check that runs the pkl generated-output drift check
- Keep existing apps (sync-opencode-config, pkl-check-generated, token-count-workflows)

## Success criteria
- `nix build ./cli#sce` produces the CLI binary without running tests
- `nix flake check ./cli` runs: tests, clippy, fmt
- `nix flake check` (root) runs: all CLI checks + pkl-parity
- All existing functionality preserved

## Constraints and non-goals
- Keep `../.` src pattern in CLI flake (access to parent assets)
- Do not change dev shell setup
- Do not modify any application code
- Keep flake inputs unchanged

## Task stack

- [x] T01: Refactor cli/flake.nix checks structure (status:done)
  - Task ID: T01
  - Goal: Replace mkCheck helper with direct check derivations for tests, clippy, and fmt
  - Boundaries (in/out of scope):
    - IN: cli/flake.nix only
    - OUT: root flake, application code, dev shell
  - Done when:
    - `nix flake check ./cli` runs tests, clippy, and fmt checks
    - `nix build ./cli#sce` succeeds without running tests
    - `clippy` app removed (redundant)
  - Verification notes:
    - `nix build ./cli#sce`
    - `nix flake check ./cli`
    - `nix eval ./cli#checks --apply builtins.attrNames`

- [x] T02: Update root flake.nix to re-export CLI checks and add pkl-parity (status:done)
  - Task ID: T02
  - Goal: Re-export CLI checks with consistent naming and add pkl-parity as a check
  - Boundaries (in/out of scope):
    - IN: flake.nix at root
    - OUT: cli/flake.nix, application code, pkl scripts
  - Done when:
    - `nix flake check` runs all CLI checks plus pkl-parity
    - Check names are consistent (cli-tests, cli-clippy, cli-fmt, pkl-parity)
    - Existing apps still work
  - Verification notes:
    - `nix flake check`
    - `nix eval .#checks --apply builtins.attrNames`
    - `nix run .#sync-opencode-config -- --help`
  - Completed: 2026-03-12
  - Evidence:
    - `nix eval ".#checks.x86_64-linux" --apply builtins.attrNames` → `["cli-clippy", "cli-fmt", "cli-tests", "pkl-parity"]`
    - `nix flake check` passed all 4 checks
    - All apps verified working

- [x] T03: Validation and cleanup (status:done)
  - Task ID: T03
  - Goal: Verify full flake check passes and update context if needed
  - Boundaries (in/out of scope):
    - IN: running checks, context sync
    - OUT: code changes
  - Done when:
    - All `nix flake check` passes for both flakes
    - Context reflects current state if any documentation needed
  - Verification notes:
    - `nix flake check ./cli`
    - `nix flake check`
    - Review context/ for any needed updates
  - Completed: 2026-03-12
  - Evidence:
    - `nix flake check ./cli` passed 3 checks (cli-tests, cli-clippy, cli-fmt)
    - `nix flake check` passed 4 checks (cli-tests, cli-clippy, cli-fmt, pkl-parity)
    - Updated `context/glossary.md`: replaced outdated `cli-setup-command-surface` entry with current `cli flake checks` entry
    - Updated `context/overview.md`: updated references to reflect new check names

## Open questions
None - requirements clarified via user input.

## Validation Report (2026-03-12)

### Commands run
| Command | Exit Code | Result |
|---------|-----------|--------|
| `nix flake check ./cli` | 0 | 3 checks passed (cli-tests, cli-clippy, cli-fmt) |
| `nix flake check` | 0 | 4 checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity) |
| `nix build ./cli#sce` | 0 | Build succeeded without running tests |
| `nix run .#pkl-check-generated` | 0 | Generated outputs up to date |

### Success criteria verification
| Criterion | Status | Evidence |
|-----------|--------|----------|
| `nix build ./cli#sce` produces CLI binary without tests | ✓ | Build succeeded, no test output |
| `nix flake check ./cli` runs: tests, clippy, fmt | ✓ | 3 checks evaluated and passed |
| `nix flake check` (root) runs: all CLI checks + pkl-parity | ✓ | 4 checks evaluated and passed |
| All existing functionality preserved | ✓ | Apps verified, dev shell unchanged |

### Context updates
- `context/glossary.md`: Replaced outdated `cli-setup-command-surface` entry with current `cli flake checks` entry
- `context/overview.md`: Updated references to reflect new check names

### Residual risks
None - all checks pass and context is aligned.

### Plan status
**COMPLETE** - All tasks executed successfully.
