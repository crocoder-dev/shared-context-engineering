# Plan: Nix Speedup via Crane Migration

## Change summary

Migrate from `buildRustPackage` to Crane for better incremental Rust build caching, apply local dev optimizations (user-level nix.conf, .envrc tuning), and document system-level speedup options.

## Success criteria

1. `nix develop` shell activation is measurably faster after initial build
2. Rust dependency changes are cached separately from source changes
3. `nix flake check` completes faster for incremental changes
4. All existing checks (`cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`) continue to pass
5. `nix build ./cli#default` and `nix run ./cli#sce -- --help` work unchanged
6. User-level `nix.conf` optimizations are documented and optional

## Constraints and non-goals

**In scope:**
- Migrate `cli/flake.nix` from `buildRustPackage` to Crane
- Update root `flake.nix` to use Crane-based CLI checks
- Add Crane-specific optimizations (dependency caching, incremental builds)
- Add user-level `~/.config/nix/nix.conf` tuning recommendations
- Optimize `.envrc` with targeted watches
- Keep existing CLI outputs (`packages.sce`, `apps.sce`, checks) unchanged from user perspective

**Out of scope:**
- Binary cache infrastructure (Cachix/Attic) - deferred
- CI workflow changes - local dev focus only
- Changes to `evals/` or `config/` - unrelated to Nix performance
- System-level `auto-optimise-store` - requires root/admin, documented as optional

**Non-goals:**
- Changing Cargo dependencies or versions
- Modifying Rust toolchain version
- Altering existing CLI behavior or output contracts

## Task stack

- [ ] T01: `Add Crane flake input to root flake.nix` (status:todo)
- [ ] T02: `Create Crane-based package definition in cli/flake.nix` (status:todo)
- [ ] T03: `Migrate CLI checks to Crane-based derivations` (status:todo)
- [ ] T04: `Add user-level nix.conf tuning recommendations` (status:todo)
- [ ] T05: `Optimize .envrc with targeted watches` (status:todo)
- [ ] T06: `Verify all checks pass and measure speedup` (status:todo)
- [ ] T07: `Update context documentation and AGENTS.md` (status:todo)

---

### T01: Add Crane flake input to root flake.nix

**Task ID:** T01

**Goal:** Add Crane as a flake input and wire it through to the CLI flake.

**Boundaries (in/out of scope):**
- In: Add `crane` input to root `flake.nix`, add `follows` for `nixpkgs`, pass to CLI flake
- Out: Changes to CLI flake itself (T02), changes to checks (T03)

**Done when:**
- `flake.nix` has `crane` input with proper `follows`
- `flake.lock` is updated with Crane dependency
- `nix flake check` passes

**Verification notes (commands or checks):**
```bash
nix flake update
nix flake check
```

---

### T02: Create Crane-based package definition in cli/flake.nix

**Task ID:** T02

**Goal:** Replace `buildRustPackage` with Crane's `buildPackage` for the SCE CLI, enabling separate dependency caching.

**Boundaries (in/out of scope):**
- In: Refactor `cli/flake.nix` to use Crane's `buildPackage`, add `crane.buildDependencies` for caching
- In: Preserve `SCE_GIT_COMMIT` environment variable injection
- In: Keep `packages.sce` and `apps.sce` outputs unchanged
- Out: Changes to root flake checks (T03), changes to CLI behavior

**Done when:**
- `cli/flake.nix` uses Crane for building
- `nix build ./cli#default` produces working `sce` binary
- `nix run ./cli#sce -- --help` shows help output
- Dependency derivation is separate from source derivation

**Verification notes (commands or checks):**
```bash
nix build ./cli#default
./result/bin/sce --help
nix run ./cli#sce -- --help
```

---

### T03: Migrate CLI checks to Crane-based derivations

**Task ID:** T03

**Goal:** Update `cli-tests`, `cli-clippy`, and `cli-fmt` checks to use Crane's toolchain and caching.

**Boundaries (in/out of scope):**
- In: Refactor `cli/flake.nix` checks to use Crane's `cargoBuild`, `cargoClippy`, `cargoFmt`
- In: Ensure checks reuse dependency cache from T02
- Out: Changes to `pkl-parity` check (root flake), changes to CLI package

**Done when:**
- `cli-tests` check passes with Crane
- `cli-clippy` check passes with Crane
- `cli-fmt` check passes with Crane
- `nix flake check` passes from repo root

**Verification notes (commands or checks):**
```bash
nix flake check
```

---

### T04: Add user-level nix.conf tuning recommendations

**Task ID:** T04

**Goal:** Document and optionally create user-level Nix configuration for faster builds.

**Boundaries (in/out of scope):**
- In: Create `docs/nix-performance.md` or add section to `AGENTS.md` with recommended `~/.config/nix/nix.conf` settings
- In: Document `max-jobs`, `cores`, and `auto-optimise-store` options
- In: Note that `auto-optimise-store` requires system-level config (root)
- Out: Changes to system-level `/etc/nix/nix.conf` (requires root, user action)
- Out: Changes to flake behavior

**Done when:**
- Documentation exists with recommended settings
- Example `nix.conf` snippet provided
- Clear distinction between user-level and system-level options

**Verification notes (commands or checks):**
```bash
# Verify docs exist
ls -la docs/nix-performance.md  # or check AGENTS.md section
```

**Recommended settings to document:**
```ini
# ~/.config/nix/nix.conf (user-level)
max-jobs = auto
cores = 0

# System-level (requires root/admin)
# /etc/nix/nix.conf
# auto-optimise-store = true
```

---

### T05: Optimize .envrc with targeted watches

**Task ID:** T05

**Goal:** Reduce direnv evaluation overhead by watching only relevant files.

**Boundaries (in/out of scope):**
- In: Update `.envrc` to add explicit `watch_file` directives for flake inputs
- In: Remove unnecessary re-evaluations on unrelated file changes
- Out: Changes to flake behavior, changes to dev shell contents

**Done when:**
- `.envrc` has targeted `watch_file` directives
- `direnv reload` is faster after unrelated file changes
- Dev shell still activates correctly

**Verification notes (commands or checks):**
```bash
direnv reload
nix develop -c echo "shell works"
```

**Proposed `.envrc`:**
```bash
use flake

# Watch only flake-related files for shell invalidation
watch_file flake.nix
watch_file flake.lock
watch_file cli/flake.nix
watch_file cli/Cargo.lock
```

---

### T06: Verify all checks pass and measure speedup

**Task ID:** T06

**Goal:** Confirm all existing functionality works and document performance improvement.

**Boundaries (in/out of scope):**
- In: Run full `nix flake check`, run `nix develop`, test incremental rebuild
- In: Document timing before/after for: clean build, dependency-only rebuild, source-only rebuild
- Out: Changes to flake structure (complete in T01-T05)

**Done when:**
- `nix flake check` passes
- `nix develop` enters shell successfully
- `nix run .#pkl-check-generated` passes
- `nix run .#sync-opencode-config` works
- Timing measurements recorded in plan

**Verification notes (commands or checks):**
```bash
nix flake check
nix develop -c sh -c 'cd cli && cargo test'
nix run .#pkl-check-generated
```

---

### T07: Update context documentation and AGENTS.md

**Task ID:** T07

**Goal:** Update documentation to reflect Crane-based build system and Nix performance recommendations.

**Boundaries (in/out of scope):**
- In: Update `AGENTS.md` with Crane-specific commands if needed
- In: Update `context/overview.md` to mention Crane in CLI flake description
- In: Add reference to Nix performance docs if created in T04
- Out: Changes to flake behavior (complete in T01-T06)

**Done when:**
- `AGENTS.md` reflects current build approach
- `context/overview.md` mentions Crane if relevant
- All verification commands pass

**Verification notes (commands or checks):**
```bash
nix flake check
```

---

## Open questions

None - scope clarified via user input:
- Binary cache: deferred
- Rust builder: Crane migration approved
- Target: local dev only

## Assumptions

1. Crane is compatible with the current Rust toolchain (stable latest via rust-overlay)
2. The `cli/Cargo.lock` path reference works with Crane's `lockFile` approach
3. No custom build phases beyond standard `cargo build` are needed
4. User has write access to `~/.config/nix/nix.conf` for optional tuning
5. System-level `auto-optimise-store` is optional and requires root access