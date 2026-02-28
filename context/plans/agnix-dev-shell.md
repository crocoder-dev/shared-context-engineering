# Plan: agnix-dev-shell

## 1) Change summary
Add `agnix` and `agnix-lsp` to the repository Nix dev environment in `flake.nix` using a reproducible Nix-first approach, with automatic `cargo install` for `agnix-cli` during `nix develop` when `agnix` is missing.

## 2) Success criteria
- `nix develop` exposes `agnix` and `agnix-lsp` on `PATH` for contributors.
- The solution is Nix-first and auto-runs `cargo install --locked agnix-cli` from shell startup when `agnix` is missing.
- If direct Nix packages are unavailable, fallback behavior is explicitly documented for remaining tools.
- Verification steps for command availability and basic execution are documented and pass.

## 3) Constraints and non-goals
- In scope: dev-shell changes in `flake.nix` and related developer docs/context updates.
- In scope: package resolution for both `agnix` and `agnix-lsp` in nixpkgs (or explicit non-auto fallback guidance).
- Out of scope: Home Manager activation logic and system-level package installation.
- Out of scope: changing application/runtime code outside environment and docs/context artifacts.
- Non-goal: adding networked auto-install behavior in `shellHook` for tools other than `agnix`.

## 4) Task stack (T01..T05)
- [x] T01: Confirm package sourcing strategy and exact package attrs (status:done)
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
  - Evidence:
    - Verified nixpkgs attribute availability in this repo flake context with `nix eval --json --impure --expr 'let flake = builtins.getFlake (toString ./.); pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; }; in { agnix = builtins.hasAttr "agnix" pkgs; agnix_lsp = builtins.hasAttr "agnix-lsp" pkgs; }'`, which returned `{ "agnix": false, "agnix_lsp": false }`.
    - Confirmed strategy for follow-up tasks: no direct nixpkgs attrs currently exist for `agnix` or `agnix-lsp`, so maintain Nix-first behavior where possible and document any fallback as explicit manual/non-automatic guidance (no networked install in `shellHook`).
    - Ran repository light check `nix flake check` after resolution; dev shell derivation evaluates successfully on the current system.

- [x] T02: Update `flake.nix` dev shell to include agnix tooling (status:done)
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
  - Evidence:
    - Updated `flake.nix` to add `agnix` and `agnix-lsp` fallback-safe PATH shims via `pkgs.writeShellScriptBin`, keeping behavior non-automatic and avoiding networked installs in `shellHook`.
    - Ran `nix flake check`; dev shell derivation evaluates successfully on the current system.
    - Ran `nix develop -c which agnix` and `nix develop -c which agnix-lsp`; both commands resolve to Nix store shim binaries in the dev shell.

- [x] T03: Add developer-facing usage and fallback notes (status:done)
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
  - Evidence:
    - Added `README.md` section "Dev shell agnix tooling" with Nix-first usage, explicit non-automatic fallback policy, manual Cargo fallback commands, optional `AGNIX_BIN`/`AGNIX_LSP_BIN` overrides, and `which`-based verification steps.
    - Performed docs dry-run command checks: `nix develop -c which agnix` and `nix develop -c which agnix-lsp`.

- [x] T04: Sync context records for current-state environment behavior (status:done)
  - Task ID: T04
  - Goal: Update relevant `context/` files so future sessions reflect the new dev-shell tooling state.
  - Boundaries (in/out of scope):
    - In: `context/patterns.md` and any directly impacted context references.
    - Out: speculative architecture expansions beyond this environment change.
  - Done when:
    - Context files accurately describe how `agnix` and `agnix-lsp` are provided in this repo.
  - Verification notes (commands or checks):
    - Context/code consistency spot-check between `flake.nix` and updated context entries.
  - Evidence:
    - Updated `context/patterns.md` to reflect current state: dev shell now includes `cargo` and `rustc`, exports `~/.cargo/bin` on `PATH`, and auto-installs `agnix-cli` when `agnix` is missing.
    - Updated `README.md` to match shell behavior and clarify that `agnix-lsp` remains shim/manual fallback based.
    - Applied user-approved scope change to include automatic networked install of `agnix-cli` during shell startup.
    - Spot-checked behavior with `nix flake check`, `nix develop -c which agnix`, `nix develop -c agnix --help`, and `nix develop -c which agnix-lsp`.

- [x] T05: Validation and cleanup (status:done)
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
  - Evidence:
    - Ran `nix flake check` (exit 0): `devShells.x86_64-linux.default` evaluated successfully; incompatible-system warning reported for non-host platforms only.
    - Ran `nix develop -c which agnix` (exit 0): resolved to `/home/davidabram/.cargo/bin/agnix`.
    - Ran `nix develop -c which agnix-lsp` (exit 0): resolved to `/home/davidabram/.cargo/bin/agnix-lsp`.
    - Ran `nix develop -c agnix --help` (exit 0): help text printed with command set (`validate`, `init`, `eval`, `telemetry`, `schema`).
    - Ran `nix develop -c agnix-lsp --help` (exit 0): command returned successfully in the dev shell.
    - Cleanup: removed temporary verification artifact directory `context/tmp/pkl-generated`; `context/tmp/` now only contains `.gitignore`.
    - Success-criteria check: all plan success criteria now have explicit command evidence and passing verification on the current host.

## 5) Open questions
- None. Scope and install-policy choices are currently: Nix-first shell, automatic `cargo install --locked agnix-cli` when `agnix` is absent, and manual fallback behavior retained for `agnix-lsp`.
