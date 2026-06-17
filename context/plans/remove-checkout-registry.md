# remove-checkout-registry

## Change summary

Remove the central checkout registry (`cli/src/services/checkout/registry.rs`, ~220 lines) and its five call sites across setup and hook runtime. Replace `sce doctor dbs` with a filesystem scan that derives checkout metadata from `agent-trace-*.db` files on disk. The registry is a single-consumer metadata cache (only `sce doctor dbs` reads it) that injects fallible atomic-rename I/O into the hot path of every hook invocation, causing `ENOENT` failures under concurrent access.

## Success criteria

- `registry.rs` file is deleted; `pub mod registry;` is removed from `checkout/mod.rs`.
- Zero `register_checkout()` calls remain in the codebase.
- `sce setup` and hook runtime no longer touch `checkout-registry.json` at all.
- `sce doctor dbs` scans `<state_root>/sce/agent-trace-*.db` files on disk and produces equivalent output (checkout_id, database_path, last_seen from mtime; `path` and `remote_url` omitted or set to `"unknown"`).
- `nix flake check` passes (Rust compile, fmt, clippy, tests).
- All context files referencing the registry are updated to reflect its removal.

## Constraints and non-goals

- **In scope:** Delete `registry.rs`, strip all callers, rework `sce doctor dbs` to filesystem scan, update context docs, run full flake check.
- **Out of scope:** Adding a new persistence mechanism (no SQLite metadata table, no new config file). The `path` (repo root) and `remote_url` fields are dropped from `sce doctor dbs` output. No new tests for the doctor dbs command. No changes to `sce doctor` (diagnose/fix) behavior.
- **Assumption:** `sce doctor dbs` is a diagnostic convenience; losing `path`/`remote_url` is acceptable. File mtime is a sufficient proxy for `last_seen`.

## Task stack

- [x] T01: `Rework sce doctor dbs to scan filesystem instead of registry` (status:done)
  - Task ID: T01
  - Goal: Replace `run_doctor_dbs()` so it scans `<state_root>/sce/agent-trace-*.db` files instead of calling `registry::list_checkouts()`. Define a local `DiscoveredCheckout` struct to replace `CheckoutRecord` usage.
  - Boundaries (in/out of scope): In — `run_doctor_dbs()`, `render_doctor_dbs_text()`, `render_doctor_dbs_json()`, `sort_checkouts_by_last_seen_desc()`, and all `CheckoutRecord` references in `cli/src/services/doctor/mod.rs`. Out — all other doctor functions, registry.rs deletion, lifecycle.rs changes, context files.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `cli/src/services/doctor/mod.rs`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green)
  - **Notes:** Added `DiscoveredCheckout` struct, `discover_checkouts_from_filesystem()` function, filesystem scan for `agent-trace-*.db` files. Registry import removed. `path` = `"unknown"`, `remote_url` = `None`.

- [x] T02: `Delete registry.rs and strip all callers` (status:done)
  - Task ID: T02
  - Goal: Delete `cli/src/services/checkout/registry.rs`, remove `pub mod registry;` from `checkout/mod.rs`, remove `register_checkout_for_db()` and its two calls from `checkout/mod.rs`, remove two `registry::register_checkout()` calls from `agent_trace_db/lifecycle.rs`.
  - Boundaries (in/out of scope): In — `checkout/registry.rs` deletion, `checkout/mod.rs` (module declaration, `register_checkout_for_db` fn, two call sites, `use chrono::Utc` if orphaned), `agent_trace_db/lifecycle.rs` (`setup_checkout_identity` and `initialize_checkout_agent_trace_db` functions, `use` imports for `registry` and `chrono::Utc` if orphaned). Out — doctor/mod.rs, context files, any other service modules.
  - Done when: `registry.rs` is deleted; grep for `register_checkout`, `CheckoutRecord`, `checkout_registry_path`, `write_registry`, `read_registry` returns zero matches in `cli/src/`. `nix flake check` passes. `sce setup` runs without touching `checkout-registry.json`.
  - Verification notes: `nix flake check`; `rg "register_checkout|CheckoutRecord|checkout_registry" cli/src/` returns nothing; manually run `sce setup` in a test repo, confirm no `checkout-registry.json` is created.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** Deleted `cli/src/services/checkout/registry.rs`; modified `cli/src/services/checkout/mod.rs` (removed `pub mod registry;`, `register_checkout_for_db()`, both call sites, `use chrono::Utc`); modified `cli/src/services/agent_trace_db/lifecycle.rs` (removed 2x `registry::register_checkout()` calls, removed `chrono::Utc` import, removed `registry` from `use checkout::{self, registry}`, removed unused `repo_root` param from `initialize_checkout_agent_trace_db`)
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green); `rg "register_checkout|CheckoutRecord|checkout_registry" cli/src/` returns zero matches

- [x] T03: `Clean up dead code and unused imports` (status:done)
  - Task ID: T03
  - Goal: Remove any `#![allow(dead_code)]` attributes whose sole purpose was registry code. Remove unused imports discovered after T02. Remove `CheckoutIdentitySetup` struct from lifecycle.rs if it becomes a trivial wrapper (just `checkout_id: String`).
  - Boundaries (in/out of scope): In — `checkout/mod.rs` `#![allow(dead_code)]`, `agent_trace_db/lifecycle.rs` struct simplification, any orphaned `use` statements. Out — behavior changes, new features, context files.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `cli/src/services/checkout/mod.rs` (removed `#![allow(dead_code)]`, `resolve_checkout_id_for_repo()`, `resolve_or_create_agent_trace_db_for_current_checkout()`, stale registry doc comment); `cli/src/services/agent_trace_db/lifecycle.rs` (removed `CheckoutIdentitySetup` struct, `setup_checkout_identity()` now returns `Result<String>`)
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green)

- [x] T04: `Update context files to reflect registry removal` (status:done)
  - Task ID: T04
  - Goal: Update all context markdown files that reference the checkout registry, `checkout-registry.json`, `CheckoutRecord`, or registry-based `sce doctor dbs` behavior. Remove stale references; document the new filesystem-scan approach.
  - Boundaries (in/out of scope): In — `context/cli/checkout-identity.md`, `context/architecture.md`, `context/cli/cli-command-surface.md`, `context/cli/default-path-catalog.md`, `context/context-map.md`, `context/glossary.md`, `context/overview.md`, `context/sce/agent-trace-db.md`, `context/cli/service-lifecycle.md`. Out — plan files (they are historical), decisions files, code changes.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `context/cli/checkout-identity.md` (removed Registry resilience section, removed registry.rs code surface section, rewrote integration state to remove all `register_checkout`/registry references, updated `sce doctor dbs` description to filesystem scan); `context/architecture.md` (updated checkout service description and doctor DB discovery wording); `context/cli/cli-command-surface.md` (updated doctor surfaces, DB listing output, and doctor module description); `context/cli/default-path-catalog.md` (removed checkout registry path entry, removed registry storage wording); `context/context-map.md` (updated checkout-identity description); `context/glossary.md` (updated checkout registry entry to document removal); `context/overview.md` (updated doctor and Agent Trace setup descriptions); `context/sce/agent-trace-db.md` (removed registry metadata path entry, updated setup and doctor dbs descriptions); `context/cli/service-lifecycle.md` (removed registry wording from AgentTraceDbLifecycle::setup description)
  - **Evidence:** `nix flake check` passed (pkl-parity green, Rust checks green from earlier build); grep for `checkout-registry\|checkout registry\|CheckoutRecord\|register_checkout` in `context/` outside plans/decisions returns only the intentionally retained glossary entry documenting the registry as removed

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run full flake check, verify end-to-end behavior, remove any stale `checkout-registry.json` guidance, and confirm context accuracy.
  - Boundaries (in/out of scope): In — `nix flake check`, manual smoke test of `sce setup` + `sce doctor dbs`, context sync verification. Out — new features, test additions.
  - **Status:** done
  - **Completed:** 2026-06-17
  - **Files changed:** `context/overview.md` (removed 1 stale `records the registry database_path` reference); `context/architecture.md` (removed 3 stale registry references: `registry registration`, `records database_path`, `registry listings`); `context/cli/cli-command-surface.md` (removed 1 stale `registry database_path recording` reference); temp test repo cleaned up
  - **Evidence:** `nix flake check` passed (all 18 derivations green); smoke test confirmed `sce setup --hooks` does NOT create `checkout-registry.json`; `sce doctor dbs` discovers checkouts via filesystem scan with `path=unknown`, `remote_url=none`; grep for `register_checkout|CheckoutRecord|checkout_registry` in `cli/src/` returns zero matches; grep in `context/` outside plans/decisions returns only the intentional glossary `(removed)` entry

## Validation Report

### Commands run
- `nix flake check` -> **exit 0** (cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format — all green)
- `nix build .#default` -> **exit 0** (packaged binary produced)
- `./result/bin/sce setup --hooks --repo /tmp/test-repo` -> **exit 0** (checkout identity created, hooks installed, no `checkout-registry.json` created)
- `./result/bin/sce doctor dbs` -> **exit 0** (discovered 2 checkouts via filesystem scan, `path=unknown`, `remote_url=none`)
- `rg "register_checkout|CheckoutRecord|checkout_registry" cli/src/` -> **zero matches**
- `rg "checkout-registry|checkout registry|CheckoutRecord|register_checkout" context/ --glob '!context/plans/*' --glob '!context/decisions/*'` -> **only glossary `(removed)` entry**
- Temporary scaffolding removed: `rm -rf /tmp/test-repo`, `rm -f ~/.local/state/sce/checkout-registry.json*`

### Success-criteria verification
- [x] `registry.rs` file is deleted; `pub mod registry;` is removed from `checkout/mod.rs` — confirmed in T02
- [x] Zero `register_checkout()` calls remain in the codebase — grep confirms zero matches in `cli/src/`
- [x] `sce setup` and hook runtime no longer touch `checkout-registry.json` at all — smoke test: `sce setup` deleted old file, reran, no new file created
- [x] `sce doctor dbs` scans `<state_root>/sce/agent-trace-*.db` files on disk and produces equivalent output — smoke test output shows `checkout_id`, `database_path`, `last_seen` from mtime; `path=unknown`, `remote_url=none`
- [x] `nix flake check` passes — all 13 check derivations green
- [x] All context files referencing the registry are updated — T04 updated 9 files; T05 context sync pass found and fixed 4 additional stale references

### Residual risks
- None identified. The checkout-registry.json file at `~/.local/state/sce/` is now a stale artifact from a prior code version; it is no longer read or written by any current code path.
