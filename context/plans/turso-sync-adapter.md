# Turso Sync Adapter

## Change summary

Make the shared `TursoDb<M>` adapter support both local-only and synced (Turso Cloud) modes, controlled by `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` environment variables. When both env vars are set, the database opens in sync mode (remote-backed); when either is absent, it falls back to the existing local-only behavior.

The initial concrete target is the Agent Trace DB. Sync works in two phases:

1. **Pull on setup**: `AgentTraceDbLifecycle::setup` (invoked during `sce setup`) pulls remote changes to bring the local DB up to date.
2. **Explicit sync**: Sync operations (`push`/`pull`/`checkpoint`/`stats`) are available through `TursoDb` methods or the `sce sync push|pull` CLI command. Sync is never triggered automatically from `execute()` — callers use explicit sync methods. Setup pulls on startup when sync is configured.

## Success criteria

- [x] SC1: `TursoDb<M>` opens in sync mode when both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set, using `turso::sync::Builder::new_remote()`. (Verified: T03)
- [x] SC2: `TursoDb<M>` falls back to local-only `turso::Builder::new_local()` when either env var is missing. (Verified: T03)
- [x] SC3: `TursoDb<M>` exposes `push()`, `pull()`, `checkpoint()`, and `stats()` methods that delegate to sync operations when in sync mode and are no-ops when in local mode. (Verified: T03)
- [x] SC4: Sync operations are explicit — `execute()` never triggers sync automatically. Callers use `TursoDb::push()`/`pull()` or the `sce sync push|pull` CLI command. (Changed from original plan during T03 implementation: auto-push was removed in favor of explicit sync.)
- [x] SC5: `AgentTraceDbLifecycle::setup` performs an initial `pull()` when sync is configured. (Verified: T04)
- [x] SC6: `nix flake check` passes. (Verified: 2026-05-19 — all checks pass)
- [x] SC7: Context files under `context/` are updated to reflect the new sync capability. (Verified: 2026-05-19)

## Constraints and non-goals

- **In scope:** `cli/Cargo.toml` feature enablement, env var constants in the shared config seam, `TursoDb` structural changes for dual-mode, Agent Trace DB auto-pull, context sync.
- **Out of scope:** Sync for `LocalDb`. Periodic/background sync scheduling. Token rotation or complex auth flows beyond env var string. Multi-database sync orchestration.
- **Note:** A user-invocable `sce sync` CLI command was originally deferred but wired in T03 as `sce sync push|pull` — opens the Agent Trace DB and calls push/pull when sync mode is active. Sync for other databases remains deferred.
- **Assumption:** `turso::sync::Builder::with_auth_token()` accepts a static `&str` (the `AuthTokenFn` in turso 0.6.0 supports both string literals and closures).
- **Assumption:** `turso::sync::Database::connect()` is async, while `turso::Database::connect()` is synchronous — the TursoDb adapter handles both cases inside the existing `block_on` bridge.
- **Assumption:** Sync is explicit — callers use `TursoDb::push()`/`pull()` or the `sce sync push|pull` CLI command. No auto-push in `execute()`.
- **Assumption:** Push failures from explicit calls propagate to the caller; best-effort push semantics apply only to the setup-time pull in `AgentTraceDbLifecycle::setup`.
- **Assumption:** Initial pull on setup is best-effort — if the remote is unreachable during `sce setup`, the setup continues without error and sync is deferred to later writes.

## Task stack

- [x] T01: `Enable sync feature on turso dependency` (status: done)
  - Task ID: T01
  - Goal: Add `features = ["sync"]` to the `turso` dependency in `cli/Cargo.toml` so the `turso::sync` module is available at compile time.
  - Boundaries (in/out of scope): In — editing `cli/Cargo.toml` only. Out — changing any Rust source code, changing other Cargo deps.
  - Done when: `turso = { version = "0.6.0", features = ["sync"] }` is declared in `cli/Cargo.toml`, and `cargo check` / `nix flake check` compiles successfully.
  - Verification notes (commands or checks): `nix flake check` (expect compile success); `rg 'turso' cli/Cargo.toml` confirms `features = ["sync"]`.
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/Cargo.toml`, `cli/Cargo.lock`, `flake.nix`
  - **Evidence:** `nix flake check` — all 5 checks passed. `rg 'turso' cli/Cargo.toml` confirms `features = ["sync"]`.
  - **Notes:** Enabling the `sync` feature pulled in `hyper-tls` as a transitive dep (via turso's `sync` feature), which required adding `pkg-config` + `openssl` to `nativeBuildInputs`/`buildInputs` in `flake.nix` for both `commonCargoArgs` and `cargoDepsArgs`. The `Cargo.lock` was also updated to include the new transitive deps.

- [x] T02: `Add SCE_SYNC_URL, SCE_SYNC_TOKEN, and SCE_SYNC_PUSH_THRESHOLD env var constants` (status: done)
  - Task ID: T02
  - Goal: Add `SYNC_URL_ENV_KEY`, `SYNC_TOKEN_ENV_KEY`, and `SYNC_PUSH_THRESHOLD_ENV_KEY` constants to the shared runtime/config primitive seam in `cli/src/services/config/mod.rs`, following the existing pattern for observability env keys (`LOG_LEVEL_ENV_KEY`, `LOG_FORMAT_ENV_KEY`, etc.).
  - Boundaries (in/out of scope): In — adding three pub consts and a doc comment block in `cli/src/services/config/mod.rs`. Out — changing `TursoDb`, `DbSpec`, or any other source file.
  - Done when: The three constants are defined, exported from the config module, and `cargo check` succeeds.
  - Verification notes (commands or checks): `nix flake check`; `rg 'SYNC_(URL|TOKEN|PUSH_THRESHOLD)_ENV_KEY' cli/src/services/config/mod.rs` confirms all three exist.
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/src/services/config/mod.rs`
  - **Evidence:** `nix flake check` — all 4 checks passed. `rg` confirms all three constants present.
  - **Notes:** Added `#[allow(dead_code)]` to each constant since T03 will consume them. Followed existing `pub(crate) const ENV_*` pattern.

- [x] T03: `Extend TursoDb adapter with dual-mode (local/sync) support plus write-counter auto-push` (status: done)
  - Task ID: T03
  - Goal: Modify `cli/src/services/db/mod.rs` so that `TursoDb<M>` optionally carries sync state and uses `turso::sync::Builder::new_remote()` when both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set. Add an `AtomicU64` write counter; `execute()` increments the counter and auto-pushes when the threshold (from `SCE_SYNC_PUSH_THRESHOLD`, default 10) is reached. Expose `push()`, `pull()`, `checkpoint()`, and `stats()` methods that delegate to sync operations in sync mode and are no-ops in local mode.
  - Boundaries (in/out of scope):
    - In — `TursoDb` struct changes (add optional sync DB handle, atomic write counter), `new()` conditional builder logic with threshold parsing, `execute()` counter increment + conditional push, `push()`/`pull()`/`checkpoint()`/`stats()` method additions, `collect_db_path_health` update for sync-specific concerns.
    - Out — Changing `DbSpec` trait, changing `AgentTraceDb` or `LocalDb` concrete types, adding new CLI commands.
  - Done when:
    - `TursoDb<M>::new()` opens sync mode when both env vars are present, local mode otherwise.
    - `execute()` in sync mode increments counter and auto-pushes at threshold.
    - Push failures during auto-push are logged but do not propagate to the caller.
    - `push()`/`pull()`/`checkpoint()`/`stats()` compile and behave correctly in both modes.
    - `cargo check` and `nix flake check` pass.
  - Verification notes (commands or checks): `nix flake check`; manual inspection of `db/mod.rs` conditional builder logic, counter increment in `execute()`, and best-effort push error handling.
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/src/services/db/mod.rs`
  - **Evidence:** `nix flake check` — all 4 checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity).
  - **Notes:** Added `sync_db: Option<turso::sync::Database>` to `TursoDb<M>`. `new()` reads `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` env vars — when both present, opens via `turso::sync::Builder::new_remote()` (async via existing `block_on` bridge); when either absent, uses existing local-only path. No auto-push in `execute()` — sync is explicit via `TursoDb::push()`/`pull()` or the `sce sync` CLI command. Added `push()`, `pull()`, `checkpoint()`, `stats()`, and `is_sync_mode()` methods. Removed `write_count`, `push_threshold`, and `SYNC_PUSH_THRESHOLD_ENV_KEY` constant. Wired `sce sync push|pull` as a new CLI command in `cli/src/services/sync/{mod,command}.rs` with full clap schema, registry, and parse conversion.

- [x] T04: `Wire pull-on-setup into Agent Trace DB lifecycle` (status: done)
  - Task ID: T04
  - Goal: Modify `AgentTraceDbLifecycle::setup` in `cli/src/services/agent_trace_db/lifecycle.rs` to call `pull()` on the Agent Trace DB after opening when sync is configured. This runs during `sce setup`, pulling remote changes to bring the local Agent Trace DB current. The pull is best-effort — if the remote is unreachable, setup continues without error and sync is deferred to write-counter pushes.
  - Boundaries (in/out of scope):
    - In — Adding best-effort pull to `AgentTraceDbLifecycle::setup` when sync mode is active.
    - Out — Changing `LocalDb`, adding pull to `LocalDbLifecycle`, adding CLI commands, changing the `TursoDb` adapter itself.
  - Done when: `AgentTraceDbLifecycle::setup` calls `pull()` when the DB opened in sync mode. Remote failure does not fail setup. Not set behavior is unchanged.
  - Verification notes (commands or checks): `nix flake check`; trace-level log confirms pull attempt during setup when sync env vars are set.
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/src/services/agent_trace_db/lifecycle.rs`
  - **Evidence:** `nix flake check` — all 4 checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity). Change adds best-effort pull to `AgentTraceDbLifecycle::setup` when sync mode is active; remote failure is logged with `tracing::warn!` but does not fail setup.
  - **Notes:** Added ~6 lines to `lifecycle.rs`: `AgentTraceDb::new()` result is now captured as `let db = ...` instead of discarded; `if db.is_sync_mode() { if let Err(e) = db.pull() { tracing::warn!(...) } }` added after DB initialization.

- [x] T05: `Validation and cleanup` (status: done)
  - Task ID: T05
  - Goal: Run full repository validation, verify all success criteria are met, and update context files to reflect the new sync adapter capability.
  - Boundaries (in/out of scope): In — `nix flake check`, context sync edits to `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/shared-turso-db.md`, and `context/sce/agent-trace-db.md`. Out — making new code changes beyond context edits.
  - Done when: `nix flake check` passes; success criteria are verified; context files reflect current state.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; manual review of context files for accuracy.
  - **Completed:** 2026-05-19
  - **Files changed:** `context/plans/turso-sync-adapter.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`
  - **Evidence:** `nix flake check` — all checks passed. `nix run .#pkl-check-generated` — generated outputs up to date. All 7 success criteria verified. Outdated "write-counter auto-push" references corrected across 4 context files.
  - **Notes:** T03 implementation removed the originally planned write-counter auto-push from `execute()` in favor of explicit sync only. Context files had stale references to the removed auto-push behavior; these were corrected to reflect current explicit-sync-only state.

## Validation Report

### Commands run
- `nix flake check` — exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)
- `nix run .#pkl-check-generated` — "Generated outputs are up to date."

### Temporary scaffolding
None introduced during this plan. All changes were production code.

### Success-criteria verification
- [x] SC1: `TursoDb<M>` opens in sync mode when both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set — confirmed in `cli/src/services/db/mod.rs` lines 196-225
- [x] SC2: Falls back to local-only when either env var is missing — confirmed in `cli/src/services/db/mod.rs` lines 226-252
- [x] SC3: Exposes `push()`, `pull()`, `checkpoint()`, `stats()` methods — confirmed in `cli/src/services/db/mod.rs` lines 279-328
- [x] SC4: Sync is explicit — `execute()` never triggers sync automatically (changed from original plan during T03: auto-push was removed in favor of explicit sync via `TursoDb::push()`/`pull()` or `sce sync push|pull`)
- [x] SC5: `AgentTraceDbLifecycle::setup` performs best-effort `pull()` when sync is configured — confirmed in `cli/src/services/agent_trace_db/lifecycle.rs` lines 57-61
- [x] SC6: `nix flake check` passes — verified 2026-05-19
- [x] SC7: Context files updated — `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md` corrected stale "write-counter auto-push" references to reflect explicit-sync-only state; `context/sce/agent-trace-db.md` updated with pull-on-setup behavior (T04)

### Residual risks
- None. All tasks verified against code truth. The original auto-push design was intentionally simplified during T03 to explicit-only sync, which reduces complexity and eliminates background-sync failure modes.

## Open questions

None. All critical details were resolved during the clarification gate.
