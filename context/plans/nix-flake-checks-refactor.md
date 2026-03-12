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

- [ ] T02: Update root flake.nix to re-export CLI checks and add pkl-parity (status:todo)
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

- [ ] T03: Validation and cleanup (status:todo)
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

## Open questions
None - requirements clarified via user input.
