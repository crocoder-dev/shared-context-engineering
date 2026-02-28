# Plan: agnix-dev-shell

## 1) Change summary
Add `agnix` and `agnix-lsp` to the repository Nix dev environment in `flake.nix` using a reproducible Nix-first approach, without automatic networked Cargo installs during `nix develop`.

## 2) Success criteria
- `nix develop` exposes `agnix` and `agnix-lsp` on `PATH` for contributors.
- The solution is Nix-first and does not auto-run `cargo install` from shell startup.
- If direct Nix packages are unavailable, the fallback behavior is explicitly documented and non-automatic.
- Verification steps for command availability and basic execution are documented and pass.

## 3) Constraints and non-goals
- In scope: dev-shell changes in `flake.nix` and related developer docs/context updates.
- In scope: package resolution for both `agnix` and `agnix-lsp` in nixpkgs (or explicit non-auto fallback guidance).
- Out of scope: Home Manager activation logic and system-level package installation.
- Out of scope: changing application/runtime code outside environment and docs/context artifacts.
- Non-goal: adding networked auto-install behavior in `shellHook`.

## 4) Task stack (T01..T05)
- [ ] T01: Confirm package sourcing strategy and exact package attrs (status:todo)
  - Task ID: T01
  - Goal: Resolve the concrete Nix package names/attrs for `agnix` and `agnix-lsp` (or define explicit documented fallback if one is missing).
  - Boundaries (in/out of scope):
    - In: nixpkgs package discovery, version/source approach, fallback decision notes.
    - Out: implementing shell changes.
  - Done when:
    - Package attrs are identified for both tools, or a clear fallback path is recorded for missing package(s).
    - Strategy remains Nix-first and non-networked at shell startup.
  - Verification notes (commands or checks):
    - Evaluate package availability via flake/package checks used in this repo workflow.

- [ ] T02: Update `flake.nix` dev shell to include agnix tooling (status:todo)
  - Task ID: T02
  - Goal: Modify `devShells.default` package set so both commands are available in `nix develop`.
  - Boundaries (in/out of scope):
    - In: package list updates and minimal shell hook adjustments (only if needed for visibility).
    - Out: auto-install scripts, Home Manager logic, or host-level bootstrap steps.
  - Done when:
    - `flake.nix` includes the resolved packages/fallback-safe wiring for `agnix` and `agnix-lsp`.
    - Existing shell behavior for current tools remains intact.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix develop -c which agnix`
    - `nix develop -c which agnix-lsp`

- [ ] T03: Add developer-facing usage and fallback notes (status:todo)
  - Task ID: T03
  - Goal: Document how contributors get `agnix` tooling in dev shell and what to do if a package is unavailable.
  - Boundaries (in/out of scope):
    - In: concise runbook notes tied to this repository workflow.
    - Out: Home Manager tutorial duplication and unrelated environment docs.
  - Done when:
    - Documentation states Nix-first install behavior and explicitly avoids auto network installs.
    - Manual fallback instructions (if required) are clear and isolated.
  - Verification notes (commands or checks):
    - Docs dry-run review: follow written steps from clean shell entry to command verification.

- [ ] T04: Sync context records for current-state environment behavior (status:todo)
  - Task ID: T04
  - Goal: Update relevant `context/` files so future sessions reflect the new dev-shell tooling state.
  - Boundaries (in/out of scope):
    - In: `context/patterns.md` and any directly impacted context references.
    - Out: speculative architecture expansions beyond this environment change.
  - Done when:
    - Context files accurately describe how `agnix` and `agnix-lsp` are provided in this repo.
  - Verification notes (commands or checks):
    - Context/code consistency spot-check between `flake.nix` and updated context entries.

- [ ] T05: Validation and cleanup (status:todo)
  - Task ID: T05
  - Goal: Run full verification, confirm success criteria evidence, and remove temporary artifacts.
  - Boundaries (in/out of scope):
    - In: final checks, evidence capture in plan updates, cleanup of temporary files.
    - Out: new feature additions.
  - Done when:
    - All success criteria have explicit verification evidence.
    - Dev shell entry and command checks pass for both tools.
    - No temporary artifacts remain from verification work.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix develop -c agnix --help`
    - `nix develop -c agnix-lsp --help`
    - `nix develop -c which agnix`
    - `nix develop -c which agnix-lsp`

## 5) Open questions
- None. Scope and install-policy choices were confirmed: Nix packages first, and no auto network installs during shell startup.
