# Turso Sync Adapter

## Change summary

Make the shared `TursoDb<M>` adapter support both local-only and synced (Turso Cloud) modes, controlled by `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` environment variables. When both env vars are set, the database opens in sync mode (remote-backed); when either is absent, it falls back to the existing local-only behavior.

The initial concrete target is the Agent Trace DB. Sync works in two phases:

1. **Pull on setup**: `AgentTraceDbLifecycle::setup` (invoked during `sce setup`) pulls remote changes to bring the local DB up to date.
2. **Bounded write-counter push**: Every `execute()` on the Agent Trace DB increments an atomic counter. When the counter reaches a threshold (default 10, configurable via `SCE_SYNC_PUSH_THRESHOLD`), it auto-pushes and resets. This batches writes without background threads or per-write network latency.

## Success criteria

- [ ] SC1: `TursoDb<M>` opens in sync mode when both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set, using `turso::sync::Builder::new_remote()`.
- [ ] SC2: `TursoDb<M>` falls back to local-only `turso::Builder::new_local()` when either env var is missing.
- [ ] SC3: `TursoDb<M>` exposes `push()`, `pull()`, `checkpoint()`, and `stats()` methods that delegate to sync operations when in sync mode and are no-ops when in local mode.
- [ ] SC4: `TursoDb::execute()` in sync mode increments a write counter and auto-pushes when the threshold is reached (default 10 writes). The threshold is configurable via `SCE_SYNC_PUSH_THRESHOLD`.
- [ ] SC5: `AgentTraceDbLifecycle::setup` performs an initial `pull()` when sync is configured.
- [ ] SC6: `nix flake check` passes.
- [ ] SC7: Context files under `context/` are updated to reflect the new sync capability.

## Constraints and non-goals

- **In scope:** `cli/Cargo.toml` feature enablement, env var constants in the shared config seam, `TursoDb` structural changes for dual-mode, Agent Trace DB auto-pull, context sync.
- **Out of scope:** A user-invocable `sce sync` CLI command (deferred to a later plan). Sync for `LocalDb`. Periodic/background sync scheduling. Token rotation or complex auth flows beyond env var string. Multi-database sync orchestration.
- **Assumption:** `turso::sync::Builder::with_auth_token()` accepts a static `&str` (the `AuthTokenFn` in turso 0.6.0 supports both string literals and closures).
- **Assumption:** `turso::sync::Database::connect()` is async, while `turso::Database::connect()` is synchronous — the TursoDb adapter handles both cases inside the existing `block_on` bridge.
- **Assumption:** The write-counter push uses an `AtomicU64` on `TursoDb`; the counter is checked and pushed synchronously inside `execute()` via `block_on`, so background threads are not needed.
- **Assumption:** Push failures during write-counter auto-push are logged but do not fail the `execute()` call (best-effort push).
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

- [ ] T02: `Add SCE_SYNC_URL, SCE_SYNC_TOKEN, and SCE_SYNC_PUSH_THRESHOLD env var constants` (status: todo)
  - Task ID: T02
  - Goal: Add `SYNC_URL_ENV_KEY`, `SYNC_TOKEN_ENV_KEY`, and `SYNC_PUSH_THRESHOLD_ENV_KEY` constants to the shared runtime/config primitive seam in `cli/src/services/config/mod.rs`, following the existing pattern for observability env keys (`LOG_LEVEL_ENV_KEY`, `LOG_FORMAT_ENV_KEY`, etc.).
  - Boundaries (in/out of scope): In — adding three pub consts and a doc comment block in `cli/src/services/config/mod.rs`. Out — changing `TursoDb`, `DbSpec`, or any other source file.
  - Done when: The three constants are defined, exported from the config module, and `cargo check` succeeds.
  - Verification notes (commands or checks): `nix flake check`; `rg 'SYNC_(URL|TOKEN|PUSH_THRESHOLD)_ENV_KEY' cli/src/services/config/mod.rs` confirms all three exist.

- [ ] T03: `Extend TursoDb adapter with dual-mode (local/sync) support plus write-counter auto-push` (status: todo)
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

- [ ] T04: `Wire pull-on-setup into Agent Trace DB lifecycle` (status: todo)
  - Task ID: T04
  - Goal: Modify `AgentTraceDbLifecycle::setup` in `cli/src/services/agent_trace_db/lifecycle.rs` to call `pull()` on the Agent Trace DB after opening when sync is configured. This runs during `sce setup`, pulling remote changes to bring the local Agent Trace DB current. The pull is best-effort — if the remote is unreachable, setup continues without error and sync is deferred to write-counter pushes.
  - Boundaries (in/out of scope):
    - In — Adding best-effort pull to `AgentTraceDbLifecycle::setup` when sync mode is active.
    - Out — Changing `LocalDb`, adding pull to `LocalDbLifecycle`, adding CLI commands, changing the `TursoDb` adapter itself.
  - Done when: `AgentTraceDbLifecycle::setup` calls `pull()` when the DB opened in sync mode. Remote failure does not fail setup. Not set behavior is unchanged.
  - Verification notes (commands or checks): `nix flake check`; trace-level log confirms pull attempt during setup when sync env vars are set.

- [ ] T05: `Validation and cleanup` (status: todo)
  - Task ID: T05
  - Goal: Run full repository validation, verify all success criteria are met, and update context files to reflect the new sync adapter capability.
  - Boundaries (in/out of scope): In — `nix flake check`, context sync edits to `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/shared-turso-db.md`, and `context/sce/agent-trace-db.md`. Out — making new code changes beyond context edits.
  - Done when: `nix flake check` passes; success criteria are verified; context files reflect current state.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; manual review of context files for accuracy.

## Open questions

None. All critical details were resolved during the clarification gate.
