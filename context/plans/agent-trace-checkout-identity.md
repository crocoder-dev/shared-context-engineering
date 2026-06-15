# agent-trace-checkout-identity

## Change summary

Give each cloned repository (and linked Git worktree) its own `agent-trace` checkout identity and its own database. Currently every checkout shares one global database at `<state_root>/sce/agent-trace.db`. After this plan, `sce setup` detects the current checkout, assigns a stable checkout ID stored in `.git/sce/checkout-id`, registers it in a central JSON registry, and resolves a per-checkout database at `<state_root>/sce/agent-trace-{checkout_id}.db` so each local checkout gets isolated storage.

## Success criteria

- `sce setup` run inside a cloned repository creates a stable UUIDv7 checkout ID in `<git-dir>/sce/checkout-id` and registers the checkout in `<state_root>/sce/checkout-registry.json` — but does **not** eagerly initialize the per-checkout database.
- A second `sce setup` in the same checkout reuses the existing ID (idempotent).
- A linked Git worktree gets its own checkout ID distinct from the main checkout.
- The per-checkout database (`agent-trace-{checkout_id}.db`) is created lazily on the first write — when a hook fires and finds a checkout ID but no database (or an uninitialized database), it auto-creates the DB, runs migrations, and updates the registry `database_path`.
- If no checkout ID exists yet (cloned but never ran `sce setup`), the first hook invocation creates the checkout ID, registers it, and creates the DB in one pass.
- Each checkout resolves its own `agent-trace-{checkout_id}.db`; data is isolated per local checkout.
- Per-checkout database isolation means every row in `agent-trace-{checkout_id}.db` inherently belongs to that checkout — no `checkout_id` column is needed in any table.
- `sce doctor` shows the current checkout identity (checkout ID + per-checkout DB status) instead of a generic global Agent Trace DB health row.
- `sce doctor dbs` lists all registered checkouts from the registry with `checkout_id`, `path`, `database_path`, `last_seen`, and `remote_url`.
- All existing hook flows (`post-commit`, `diff-trace`, `session-model`, `conversation-trace`) continue to work, resolving their database from the current checkout with lazy initialization.
- Global lifecycle operations (`sce doctor`, `sce setup` on a fresh machine) still work.
- `nix flake check` and `nix run .#pkl-check-generated` pass.

## Constraints and non-goals

- In scope: checkout ID creation/reuse during `sce setup`, central JSON registry, per-checkout DB path, hook and lifecycle refactoring to use the per-checkout DB.
- In scope: `git rev-parse --git-dir` is the canonical way to locate the checkout metadata directory; the checkout ID file lives at `<git-dir>/sce/checkout-id` and is never committed.
- In scope: the checkout ID is a UUIDv7 (consistent with the existing `agent_trace_id` convention in this codebase).
- In scope: hooks resolve the checkout by calling `git rev-parse --git-dir` from the current working directory (or passed repository root) and reading `<git-dir>/sce/checkout-id`.
- Out of scope: a separate `DatabaseResolver` abstraction layer — the DB path is derived directly from the checkout ID.
- Out of scope: purging or deleting old checkout DBs; checkout migration/rename handling; `sce sync` command changes.
- Out of scope: adding `checkout_id` columns to any table — per-checkout DB files provide isolation without needing a discriminator column.
- Out of scope: changing OpenCode or Claude agent-trace plugin behavior.
- Out of scope: new external crate dependencies.
- Use `git rev-parse --git-dir` resolved from the repository root. For a normal clone this returns `.git`; for a worktree it returns `<main-repo>/.git/worktrees/<name>`. Store the checkout ID file relative to that resolved path.

## Assumptions

- `git rev-parse --git-dir` is available in all Git environments where `sce setup` already works (the setup preflight already requires a Git repository).
- The checkout registry file (`<state_root>/sce/checkout-registry.json`) can be safely read/written with standard filesystem operations; it does not require locking beyond atomic write-and-rename.
- Per-checkout databases use the same `AgentTraceDbSpec` migration set as the global DB; the migration runner is idempotent.
- Hooks resolve the checkout ID from the current working directory (which is always the repository root when invoked by Git) or from an explicitly passed repository root. Hook entrypoints that don't currently receive a repository root (`diff-trace`, `session-model`, `conversation-trace`) will resolve it from `std::env::current_dir()`.
- The global `agent-trace.db` is no longer created or used for new setups after this plan; existing global DB files are left in place but not accessed by the new code paths.
- Per-checkout database creation is **lazy**: `sce setup` only establishes the checkout identity (ID + registry). The actual `agent-trace-{checkout_id}.db` file is created on the first hook invocation that needs it. If a hook fires in a checkout that has no checkout ID at all, the hook auto-creates the ID, registers it, and creates the DB in one pass.
- `TursoDb<AgentTraceDbSpec>` currently resolves its path exclusively through `AgentTraceDbSpec::db_path()` → `agent_trace_db_path()`. T03 must add an `open_at(path)` constructor (or path-override mechanism) so the per-checkout DB can be opened at an arbitrary path while still running migrations and retry logic from the shared adapter.
- The hook hot path currently opens with `open_for_hooks_without_migrations()` + `ensure_schema_ready_for_hooks()`. The lazy resolution helper must handle three scenarios: (a) **brand-new DB** — no file exists, or file exists but `__sce_migrations` table is absent → fall back to full `new()` (run all migrations); (b) **existing DB, migrations current** — `ensure_schema_ready_for_hooks()` passes → use fast no-migration path; (c) **existing DB, pending migrations** (e.g. after a `sce` version bump that ships new migrations) — `ensure_schema_ready_for_hooks()` fails readiness → auto-run `new()` to apply pending migrations, then proceed. This ensures hook upgrades are seamless; no manual `sce setup` is needed after a version bump.
- `AgentTraceDbLifecycle::fix()` currently bootstraps the global DB parent directory. After T03, when running in a checkout context, `fix()` targets the per-checkout DB path instead. T03 already covers this refactor but it applies to the fix/diagnose paths, not just setup.
- Existing tests in `agent_trace_db/` and `hooks/` that reference `agent_trace_db_path()` or assert on the global DB path will need updating. Verification through `nix flake check` will surface these.

## Task stack

- [ ] T01: `Checkout identity infrastructure` (status:todo)
  - Task ID: T01
  - Goal: Add a `cli/src/services/checkout/` module with checkout ID storage (`<git-dir>/sce/checkout-id`) and a central JSON registry (`<state_root>/sce/checkout-registry.json`).
  - Boundaries (in/out of scope): In — new `checkout/mod.rs` with `resolve_git_dir(repo_root)`, `get_or_create_checkout_id(git_dir)`, `read_checkout_id(git_dir)`, and `resolve_checkout_id_for_repo(repo_root)` (convenience combining the two); new `checkout/registry.rs` with `CheckoutRecord` struct carrying `checkout_id`, `path`, `last_seen` (RFC 3339), `remote_url` (optional), and `database_path` (optional); `CheckoutRegistry` with `register_checkout()`, `update_checkout_last_seen()`, `list_checkouts()`, and `remove_checkout()`; atomic write-through-rename for persistence. Checkout ID is a UUIDv7 generated with the `uuid` crate (already a dependency). Out — setup/hook integration, DB path changes, schema changes, doctor changes.
  - Done when: `cargo build` compiles the new modules; focused unit tests for checkout ID create/read/idempotent-reuse, `resolve_git_dir` against a real git repo, and registry register/list/update pass.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test checkout'` during development; final validation via `nix flake check`.

- [ ] T02: `Integrate checkout detection into setup lifecycle` (status:todo)
  - Task ID: T02
  - Goal: `AgentTraceDbLifecycle::setup()` resolves the checkout ID from the repo root, creates or reuses it, and registers the checkout in the central registry. The per-checkout database is **not** eagerly initialized — it will be created lazily on first write (T03).
  - Boundaries (in/out of scope): In — `AgentTraceDbLifecycle::setup()` calls `checkout::resolve_git_dir()` and `checkout::get_or_create_checkout_id()` using `ctx.repo_root()`, then `checkout::registry::register_checkout()`. Setup outcome messaging reports the resolved checkout ID and notes that the DB will be created on first write. The global `agent_trace_db_path()` DB is still opened for now (backward compat) — the switch to per-checkout paths comes in T03. Lifecycle `diagnose()` and `fix()` continue to target the global DB until T03. Out — hook flow changes, per-checkout DB path changes, registry query from doctor.
  - Done when: `sce setup` in a cloned repo prints the checkout ID; a second run reuses the same ID; `cat .git/sce/checkout-id` shows the same UUID; `<state_root>/sce/checkout-registry.json` contains the registered record (with `database_path` still absent/`null` since the DB hasn't been created yet); existing setup tests pass.
  - Verification notes (commands or checks): Manual end-to-end: run `sce setup` in a test repo, inspect `.git/sce/checkout-id` and registry file, re-run `sce setup` and confirm idempotent ID reuse. Run existing setup tests through `nix flake check`.

- [ ] T03: `Enable per-checkout database resolution with lazy initialization` (status:todo)
  - Task ID: T03
  - Goal: Switch all Agent Trace DB consumers from the shared global path to per-checkout paths. Each checkout gets its own `agent-trace-{checkout_id}.db`, created lazily on first write when a hook fires — no explicit setup step needed for the DB itself. Per-checkout DB files provide isolation without any `checkout_id` column in the schema.
  - Boundaries (in/out of scope): In —
    - Add `agent_trace_db_path_for_checkout(checkout_id: &str) -> Result<PathBuf>` to `cli/src/services/default_paths.rs`, resolving to `<state_root>/sce/agent-trace-{checkout_id}.db`.
    - Add a path-override constructor to `TursoDb<AgentTraceDbSpec>` (or `AgentTraceDb`) so the per-checkout DB can be opened at an arbitrary path while still running migrations and retry logic. The existing `AgentTraceDbSpec::db_path()` + `TursoDb::new()` path is not parameterizable — it always resolves the global path.
    - Add a lazy DB resolution helper in `checkout/mod.rs`: `resolve_or_create_agent_trace_db_for_current_checkout() -> Result<(AgentTraceDb, String)>` that: (1) resolves the current checkout via `git rev-parse --git-dir`, (2) reads or creates the checkout ID, (3) registers the checkout if newly created, (4) computes the per-checkout DB path, (5) attempts the fast no-migration open path; if the DB file doesn't exist yet or `ensure_schema_ready_for_hooks()` fails (either brand-new DB with no `__sce_migrations` table, or existing DB with pending migrations after a version bump), falls back to the path-override constructor to run migrations, (6) on success updates the registry `database_path` if the DB was just created, (7) returns the `AgentTraceDb` handle and checkout ID.
    - Refactor `AgentTraceDbLifecycle::setup()` to only handle identity (checkout ID + registration) — no longer opens the global DB. `diagnose()` checks per-checkout DB path/health when a checkout ID is available. `fix()` bootstraps the per-checkout DB parent directory (not the global one) when running in a checkout context. Fall back to global path only when no checkout context exists (e.g. bare `sce doctor` outside any repo).
    - Refactor `hooks/mod.rs`: all hook entrypoints call the lazy resolution helper to get their `AgentTraceDb` handle + checkout ID. Replace direct `AgentTraceDb::new()` and `open_agent_trace_db_for_hook_runtime()` calls.
    - Update existing tests that reference `agent_trace_db_path()` or assert on the global DB path to use the per-checkout path or the test harness temp directories.
    Out — schema migrations, `checkout_id` columns, data migration from old global DB, doctor checkout identity display.
  - Done when: A fresh clone with `sce setup` run (no hooks yet) has a checkout ID and registry entry but no `agent-trace-{checkout_id}.db` file. The first hook invocation (e.g. post-commit) auto-creates the DB, runs migrations, and updates the registry `database_path`. A `git worktree add` followed by a hook invocation auto-creates a new checkout ID and DB for the worktree. Two checkouts have independent databases with no data leakage. `nix flake check` passes.
  - Verification notes (commands or checks): Manual end-to-end: run `sce setup` in a test repo, verify no per-checkout DB file exists yet, trigger a commit (post-commit hook), verify `agent-trace-{checkout_id}.db` now exists and registry `database_path` is populated. Create a worktree, trigger a hook there, verify a second DB exists independently. Run `nix flake check` for full validation.

- [ ] T04: `Surface checkout identity in doctor and add 'sce doctor dbs'` (status:todo)
  - Task ID: T04
  - Goal: Extend `sce doctor` to report the current checkout identity, and add a new `sce doctor dbs` subcommand that lists all registered checkouts from the central registry.
  - Boundaries (in/out of scope): In —
    - Normal `sce doctor` output (both text and JSON) gains a checkout identity section/row showing the resolved checkout ID and whether the per-checkout DB exists (path and health status). This replaces the current generic Agent Trace DB health row in the Configuration section — the checkout ID + its DB status is the meaningful information now.
    - New `sce doctor dbs` subcommand (text and JSON via `--format`) reads `<state_root>/sce/checkout-registry.json` and lists all registered checkouts with: `checkout_id`, `path`, `database_path`, `last_seen`, and `remote_url` (if available). Sorted by `last_seen` descending. When the registry file doesn't exist or is empty, reports "no registered checkouts" gracefully.
    - CLI schema: add `dbs` variant to the doctor subcommand enum, with `--format text|json` support.
    - `AgentTraceDbLifecycle::diagnose()` is updated to report checkout identity health instead of/in addition to the generic global DB path health.
    Out — modifying checkout rows, purging/cleanup commands, changing the existing Environment/Repository/Git Hooks/Integrations sections.
  - Done when: `sce doctor` in a repo with a checkout ID shows the checkout ID and DB status; `sce doctor dbs` lists all registered checkouts; `sce doctor dbs --format json` outputs stable machine-readable fields; `nix flake check` passes.
  - Verification notes (commands or checks): Run `sce doctor` inside a setup repo, verify checkout ID appears in output. Run `sce doctor dbs` and `sce doctor dbs --format json`, verify output shape. Run `nix flake check` for full validation.

- [ ] T05: `Validation and context sync` (status:todo)
  - Task ID: T05
  - Goal: Run full repository validation, verify all success criteria, and update durable context files to reflect the new checkout identity and per-checkout database architecture.
  - Boundaries (in/out of scope): In — `nix run .#pkl-check-generated`, `nix flake check`, context sync for `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hook-doctor.md`, `context/cli/default-path-catalog.md`, `context/cli/cli-command-surface.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`. Add a new `context/cli/checkout-identity.md` context file describing the checkout identity service. Remove any temporary/debug artifacts. Out — new behavior beyond what was implemented in T01-T04.
  - Done when: All verifications pass; context files accurately describe the current checkout identity architecture including the new `checkout/` service module, `.git/sce/checkout-id` storage, `checkout-registry.json` format, per-checkout DB naming convention (`agent-trace-{checkout_id}.db`), lazy initialization flow, `sce doctor` checkout identity reporting, and `sce doctor dbs` registry listing.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; review context files for stale references to a shared global database path or outdated architecture descriptions.

## Open questions

- None blocking. The lazy initialization model means `git worktree add` followed by a hook invocation just works — the hook creates the checkout ID and DB on first write without requiring `sce setup` in the worktree. `sce setup` remains the canonical way to pre-establish the checkout identity, but hooks are self-sufficient when it hasn't been run.
