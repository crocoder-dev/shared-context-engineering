# Retire legacy checkout-scoped Agent Trace DB surface

## Change summary

The repository-scoped Agent Trace DB migration (`d7fb455..0d241db`, plan `repository-scoped-agent-trace-db.md`) is functionally complete: active setup, hooks, lifecycle, and trace UX resolve repository-scoped storage through `agent_trace_storage` and `RepositoryAgentTraceDb`. The migration intentionally retained a "legacy surface" — a dead per-checkout DB opener, stale `#[allow(dead_code)]` attributes, test-only legacy insert methods, the `sce trace --legacy` CLI surface, and the entire legacy `AgentTraceDb` / `AgentTraceDbSpec` adapter plus its 15 incremental migration files.

This plan fully retires that legacy surface and reverses the prior "retain legacy checkout DBs, inspectable via `--legacy`, byte-for-byte untouched" policy. After this change:

- There is exactly one Agent Trace DB adapter: `RepositoryAgentTraceDb`.
- `sce trace` has no `--legacy` flag; only repository-scoped DBs are discoverable.
- The legacy `AgentTraceDb` type, `AgentTraceDbSpec`, `agent_trace_db_path()`, `agent_trace_db_path_for_checkout()`, `AGENT_TRACE_MIGRATIONS`, and `cli/migrations/agent-trace/*.sql` are deleted.
- Existing on-disk legacy checkout DB files are still never migrated, imported, renamed, or deleted by SCE; they simply become uninspectable through the CLI.

Initial code inspection found the legacy surface in:

- `cli/src/services/checkout/mod.rs` — dead `resolve_or_create_agent_trace_db_for_checkout` (`#[allow(dead_code)]`, no code callers).
- `cli/src/services/default_paths.rs` — `agent_trace_db_path_for_checkout` (sole caller is the dead opener) and `agent_trace_db_path()` (legacy global sentinel).
- `cli/src/services/agent_trace_db/mod.rs` — legacy `AgentTraceDb` / `AgentTraceDbSpec`, dead `open_for_hooks_without_migrations`, test-only insert methods, and stale `#[allow(dead_code)]` on items actually used by `RepositoryAgentTraceDb`.
- `cli/src/services/agent_trace_db/lifecycle.rs` — no-repo-root fallback to the global `agent_trace_db_path()` sentinel.
- `cli/src/services/trace/` — `--legacy` flags, `LegacyCheckout` discovery kind, `discover_legacy_agent_trace_dbs*`, `resolve_current_legacy_status_in`, `StatusError`, and `if *legacy` branches in `command.rs`; `stats.rs` / `shell.rs` / `discovery.rs` reuse `AgentTraceDb` as the read-only wrapper for repository DBs.
- `cli/src/generated_migrations.rs` and `cli/migrations/agent-trace/` — the 15-file legacy migration chain.
- Context docs documenting the retained-legacy policy.

## Success criteria

- `AgentTraceDb`, `AgentTraceDbSpec`, `AgentTraceDbSpec`-only methods, `agent_trace_db_path()`, `agent_trace_db_path_for_checkout()`, `AGENT_TRACE_MIGRATIONS`, and all 15 `cli/migrations/agent-trace/*.sql` files no longer exist in the source tree.
- `RepositoryAgentTraceDb` is the sole Agent Trace DB adapter and is used by stats, shell, discovery/readiness, lifecycle, hooks, and setup.
- `sce trace status`, `trace db list`, `trace status --all`, and `trace db shell` have no `--legacy` flag and operate only on repository-scoped DBs.
- `sce doctor` / `sce setup` run outside a Git repository no longer probe a global `agent_trace.db` sentinel; they report an actionable "not running inside a Git repository" diagnostic instead.
- No stale `#[allow(dead_code)]` attributes remain on items that are actually used by `RepositoryAgentTraceDb` or active trace code.
- Existing on-disk legacy checkout DB files are not migrated, imported, renamed, or deleted by any SCE code path.
- Context docs and a decision record reflect the policy reversal; the prior retained-legacy wording is removed from current-state context.
- `nix flake check` passes; `nix run .#pkl-check-generated` reports generated outputs are up to date when generated assets are touched.

## Constraints and non-goals

- Do not migrate, import, copy, rename, delete, archive, or backfill existing on-disk legacy checkout DB files. They become uninspectable via the CLI, but SCE must not touch them.
- Do not add a new global/checkout fallback DB path. When repository identity cannot be resolved (including the no-repo-root doctor/setup case), produce an actionable error, not a sentinel path.
- Do not change repository-scoped DB schema, row shapes, or write/query semantics. This is a retirement of legacy code, not a schema change.
- Do not change hook attribution, post-commit intersection, or commit-msg preflight behavior beyond swapping the adapter type where they already use `RepositoryAgentTraceDb`.
- Keep each task landable as one coherent commit; do not bundle unrelated changes.
- Out of scope: removing the checkout identity infrastructure itself (`<git-dir>/sce/checkout-id`), which remains a repository-scoped diagnostic metadata input.

## Task stack

- [x] T01: `Remove dead checkout DB opener and its path helper` (status:done)
  - Task ID: T01
  - Goal: Delete the dead `resolve_or_create_agent_trace_db_for_checkout` function and the now-orphaned `agent_trace_db_path_for_checkout` path helper, plus their now-unused imports in `checkout/mod.rs`.
  - Boundaries (in/out of scope): In - `cli/src/services/checkout/mod.rs` (remove `resolve_or_create_agent_trace_db_for_checkout` at ~lines 140-177, its `#[allow(dead_code)]`, and the `agent_trace_db::AgentTraceDb` / `default_paths::agent_trace_db_path_for_checkout` imports once unused), `cli/src/services/default_paths.rs` (remove `agent_trace_db_path_for_checkout` at ~lines 294-310). Out - any other `AgentTraceDb` usage, the `--legacy` surface, `agent_trace_db_path()` (still used by lifecycle/spec), test seeders.
  - Done when: `resolve_or_create_agent_trace_db_for_checkout` and `agent_trace_db_path_for_checkout` no longer exist; `checkout/mod.rs` compiles without unused-import warnings; `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "resolve_or_create_agent_trace_db_for_checkout|agent_trace_db_path_for_checkout" cli/src` returns no matches; `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt .#checks.x86_64-linux.cli-tests` pass.
  - Status: done
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/checkout/mod.rs`, `cli/src/services/default_paths.rs`
  - Evidence: `rg` returns no matches for the two removed symbols; `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt .#checks.x86_64-linux.cli-tests` all pass (exit 0).
  - Notes: Pure dead-code removal; both `use` items became unused after deleting the function and were removed. Verify-only for context sync (no cross-cutting change).

- [ ] T02: `Remove dead open_for_hooks_without_migrations and fix stale allow(dead_code) attributes` (status:todo)
  - Task ID: T02
  - Goal: Remove the genuinely-dead `AgentTraceDb::open_for_hooks_without_migrations` (no-`_at` variant) and drop stale `#[allow(dead_code)]` from items now actively used by `RepositoryAgentTraceDb` or trace command code.
  - Boundaries (in/out of scope): In - `cli/src/services/agent_trace_db/mod.rs` (remove `open_for_hooks_without_migrations` at ~line 268; drop stale `#[allow(dead_code)]` + outdated T08 comment on `pub mod repository` at ~lines 21-24; drop stale `#[allow(dead_code)]` on `INSERT_MESSAGE_SQL`, `INSERT_PART_SQL`, `MessageRole`, `InsertMessageInsert`, `PartType`, `InsertPartInsert`); `cli/src/services/trace/discovery.rs` (drop stale `#[allow(dead_code)]` on items actively called by `command.rs`: `DiscoveredAgentTraceDb`, `ResolveAgentTraceDbError`, `resolve_agent_trace_db_identifier`, `discover_agent_trace_dbs`). Out - the block-level `#[allow(dead_code)]` on `impl AgentTraceDb` blocks (kept until T06 removes the whole type); the test-only `insert_*` methods on `AgentTraceDb` (retired in T04/T06); `agent_trace_db_path()`; the `--legacy` surface.
  - Done when: removed function has no references; dropped `#[allow(dead_code)]` attributes do not produce new warnings (items are genuinely used); `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "open_for_hooks_without_migrations\b" cli/src` returns no matches (note: `open_for_hooks_without_migrations_at` must still exist); `nix build .#checks.x86_64-linux.cli-clippy` passes with no new dead-code warnings.

- [ ] T03: `Remove the --legacy trace CLI surface` (status:todo)
  - Task ID: T03
  - Goal: Delete the `--legacy` flag, `LegacyCheckout` discovery kind, legacy discovery/status/shell branches, `StatusError`, and associated legacy-only tests, so `sce trace` operates only on repository-scoped DBs.
  - Boundaries (in/out of scope): In - `cli/src/cli_schema.rs` (remove `legacy` clap args on `Status`/`TraceDb::List`/`TraceDb::Shell`); `cli/src/services/trace/mod.rs` (remove `legacy` fields from `TraceSubcommandRequest::*` and the `discover_legacy_agent_trace_dbs` import); `cli/src/services/command_registry.rs` and `cli/src/services/parse/command_runtime.rs` (remove `legacy` plumbing and the `status-all` default-`false` shortcut); `cli/src/services/trace/command.rs` (remove `if *legacy` branches and the `"sce trace db shell --legacy requires a checkout ID or alias"` validation message); `cli/src/services/trace/discovery.rs` (remove `DiscoveredAgentTraceDbKind::LegacyCheckout`, `discover_legacy_agent_trace_dbs`, `discover_legacy_agent_trace_dbs_in`); `cli/src/services/trace/status.rs` (remove `StatusError`, `resolve_current_legacy_status_in`, the `#[cfg(test)]` `resolve_current_status_at_state_root` helper if legacy-only, and the legacy-retention module doc); `cli/src/services/trace/status_all.rs` (remove `legacy` parameter from `aggregate_current_status_all` / `aggregate_status_all_in`); `cli/src/services/trace/render_status.rs` (remove the `"legacy_checkout"` JSON discriminator branch); legacy-only tests in `render_list.rs` / `render_status_all.rs` / `status_all.rs` that exercise `discover_legacy_agent_trace_dbs_in`. Out - `AgentTraceDb` reuse in `probe_readiness` / `collect_agent_trace_db_stats` / `run_agent_trace_db_shell` (switched in T05); `agent_trace_db_path()`; test seeders (migrated in T04).
  - Done when: `rg -n "legacy" cli/src/cli_schema.rs cli/src/services/trace cli/src/services/command_registry.rs cli/src/services/parse/command_runtime.rs` returns no `--legacy` flag/field/branch references; `sce trace --help` shows no `--legacy` option; `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "\\blegacy\\b" cli/src/cli_schema.rs cli/src/services/trace cli/src/services/command_registry.rs cli/src/services/parse/command_runtime.rs` returns no matches; `nix develop -c sh -c 'cd cli && cargo run -- trace --help'` shows no `--legacy`; `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass.

- [ ] T04: `Migrate test seeders from AgentTraceDb to RepositoryAgentTraceDb` (status:todo)
  - Task ID: T04
  - Goal: Switch all test DB-seeding helpers from `AgentTraceDb::open_at` (legacy migration chain) to `RepositoryAgentTraceDb::new_at` (repository schema), so no test depends on the legacy adapter.
  - Boundaries (in/out of scope): In - test `seed_db` / fixture helpers in `cli/src/services/trace/stats.rs`, `cli/src/services/trace/status_all.rs`, `cli/src/services/trace/render_status_all.rs`, `cli/src/services/trace/render_list.rs`, and any other `trace` test module that opens a DB via `AgentTraceDb::open_at` / `AgentTraceDb::open_for_hooks_without_migrations_at` to seed rows; update imports from `agent_trace_db::{AgentTraceDb, ...}` to `agent_trace_db::repository::{RepositoryAgentTraceDb, ...}` (and the repository insert structs where names differ). Out - production `collect_agent_trace_db_stats` / `run_agent_trace_db_shell` / `probe_readiness` (switched in T05); the `AgentTraceDb` type itself (removed in T06); schema assertions that rely on legacy-only tables/columns (none expected — repository schema has the same trace tables).
  - Done when: `rg -n "AgentTraceDb::(open_at|open_for_hooks_without_migrations_at|insert_)" cli/src/services/trace` returns no matches in test modules; all trace tests pass against repository-scoped schema; `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "AgentTraceDb" cli/src/services/trace` returns no matches outside `stats.rs`/`shell.rs`/`discovery.rs` production read paths (those are T05); `nix build .#checks.x86_64-linux.cli-tests` pass.

- [ ] T05: `Switch stats/shell/discovery read paths to RepositoryAgentTraceDb` (status:todo)
  - Task ID: T05
  - Goal: Replace `AgentTraceDb` with `RepositoryAgentTraceDb` as the read-only open-without-migrations wrapper in `probe_readiness`, `collect_agent_trace_db_stats`, and `run_agent_trace_db_shell`, so `AgentTraceDb` has no remaining production caller.
  - Boundaries (in/out of scope): In - `cli/src/services/trace/stats.rs` (`collect_agent_trace_db_stats` and its `count_rows` / `query_optional_*` helpers open via `RepositoryAgentTraceDb::open_for_hooks_without_migrations_at` — add this inherent method to `repository.rs` if not present, mirroring the legacy one); `cli/src/services/trace/shell.rs` (`run_agent_trace_db_shell` opens via `RepositoryAgentTraceDb`); `cli/src/services/trace/discovery.rs` (`probe_readiness` opens discovered repository DBs via `RepositoryAgentTraceDb`). Out - the `AgentTraceDb` type definition and `AgentTraceDbSpec` (removed in T06); `agent_trace_db_path()`; lifecycle no-repo-root fallback (T06).
  - Done when: `rg -n "\\bAgentTraceDb\\b" cli/src/services/trace cli/src/services/hooks cli/src/services/agent_trace_db/lifecycle.rs` returns no matches; `RepositoryAgentTraceDb` is the only DB type referenced by trace read paths; `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "\\bAgentTraceDb\\b" cli/src/services/trace` returns no matches; `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass.

- [ ] T06: `Remove legacy AgentTraceDb adapter, spec, path, and migration chain` (status:todo)
  - Task ID: T06
  - Goal: Delete the legacy `AgentTraceDb` type, `AgentTraceDbSpec`, `agent_trace_db_path()`, `AGENT_TRACE_MIGRATIONS`, the 15 `cli/migrations/agent-trace/*.sql` files, and rework the lifecycle no-repo-root fallback to an actionable error instead of the global sentinel path.
  - Boundaries (in/out of scope): In - `cli/src/services/agent_trace_db/mod.rs` (remove `AgentTraceDbSpec`, `pub type AgentTraceDb`, the `impl AgentTraceDb` blocks and their block-level `#[allow(dead_code)]`, the legacy `open_at` / `open_for_hooks_without_migrations_at` inherent methods, and the `use ... default_paths::agent_trace_db_path` import); `cli/src/services/default_paths.rs` (remove `agent_trace_db_path()`); `cli/src/generated_migrations.rs` (remove `AGENT_TRACE_MIGRATIONS`); `cli/migrations/agent-trace/` (delete all 15 `*.sql` files); `cli/build.rs` (remove the `agent-trace` migration directory from the generation source if it is enumerated separately); `cli/src/services/agent_trace_db/lifecycle.rs` (rework `resolve_lifecycle_agent_trace_db_path` so that `repo_root == None` returns an actionable error such as "Agent Trace diagnostics require a Git repository; run 'sce doctor' inside a repository or configure agent_trace.repository_id in .sce/config.json" instead of falling back to `agent_trace_db_path()`; remove the `agent_trace_db_path` import). Out - `RepositoryAgentTraceDb` / `RepositoryAgentTraceDbSpec` / `agent_trace_db_path_for_repository*` / `AGENT_TRACE_REPOSITORY_MIGRATIONS` / `cli/migrations/agent-trace-repository/`; checkout identity infrastructure; hook attribution behavior.
  - Done when: `rg -n "\\bAgentTraceDb\\b|AgentTraceDbSpec|agent_trace_db_path\\b|AGENT_TRACE_MIGRATIONS" cli/src` returns no matches; `cli/migrations/agent-trace/` no longer exists; `sce doctor` / `sce setup` run outside a Git repository reports the actionable no-repository diagnostic instead of probing a global path; `nix flake check` passes.
  - Verification notes (commands or checks): `rg -n "AgentTraceDbSpec|\\bAgentTraceDb\\b|agent_trace_db_path\\b|AGENT_TRACE_MIGRATIONS" cli/src` returns no matches; `ls cli/migrations/agent-trace` reports not found; `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass; `nix run .#pkl-check-generated` reports up to date (regenerated migrations constant may change).

- [ ] T07: `Update context docs and record policy reversal decision` (status:todo)
  - Task ID: T07
  - Goal: Update current-state context to reflect full legacy retirement and record the decision reversing the prior retained-legacy policy.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-db.md` (remove legacy `AgentTraceDb` / `AgentTraceDbSpec` / `agent_trace_db_path()` / `agent_trace_db_path_for_checkout()` / 15-migration-chain / `--legacy` inspection wording; state `RepositoryAgentTraceDb` is the sole adapter and legacy checkout DBs are uninspectable via the CLI but never touched); `context/cli/checkout-identity.md` (remove "legacy per-checkout DB helpers retained" and `resolve_or_create_agent_trace_db_for_checkout` references); `context/cli/agent-trace-storage.md` (remove legacy checkout resolver retention note); `context/cli/default-path-catalog.md` (remove `agent_trace_db_path_for_checkout` legacy entry and `agent_trace_db_path()` if listed); `context/cli/trace-command.md` (remove `--legacy` discovery/list/status/shell wording); `context/context-map.md` and `context/glossary.md` (remove legacy/`--legacy` references in affected entries); new `context/decisions/<date>-retire-legacy-agent-trace-db.md` (record the policy reversal: legacy checkout DBs are no longer inspectable via the CLI; SCE still never migrates/renames/deletes on-disk legacy files; rationale and date). Out - code changes; the completed `repository-scoped-agent-trace-db.md` plan (historical, left as-is).
  - Done when: `rg -n "legacy|--legacy|agent_trace_db_path_for_checkout|resolve_or_create_agent_trace_db_for_checkout" context/cli context/sce/agent-trace-db.md context/context-map.md context/glossary.md` returns only the new decision record and explicit historical-plan references; a dated decision record exists under `context/decisions/`.
  - Verification notes (commands or checks): focused `rg` over `context/` excluding `context/plans/**`; decision record present and dated; `nix run .#pkl-check-generated` reports up to date (no generated assets expected to change).

- [ ] T08: `Final validation and cleanup` (status:todo)
  - Task ID: T08
  - Goal: Run full validation, confirm no temporary scaffolding remains, and verify context reflects final behavior.
  - Boundaries (in/out of scope): In - `nix flake check`, generated-output parity, focused searches for residual legacy/`--legacy`/`AgentTraceDb` references across `cli/src` and `context/` (excluding `context/plans/**`), `git diff --check`, context sync verification. Out - new feature behavior beyond fixes required by validation failures.
  - Done when: Full checks pass; generated config is up to date; no temporary files remain; no residual legacy references remain outside historical plan files and the new decision record; plan status/evidence is updated.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `git diff --check`; `rg -n "\\bAgentTraceDb\\b|AgentTraceDbSpec|agent_trace_db_path|AGENT_TRACE_MIGRATIONS|--legacy|resolve_or_create_agent_trace_db_for_checkout" cli/src context -g '!context/plans/**'` returns no matches outside the new decision record's explicit historical references.

## Open questions

None blocking. Decisions captured in this plan:

- Full legacy retirement (Tier 1-4) is in scope; the prior "retain legacy checkout DBs, inspectable via `--legacy`, byte-for-byte untouched" policy is reversed.
- SCE still never migrates, imports, renames, or deletes existing on-disk legacy checkout DB files; they become uninspectable via the CLI.
- The no-repo-root doctor/setup case produces an actionable error instead of probing a global `agent_trace.db` sentinel path.
- Checkout identity infrastructure (`<git-dir>/sce/checkout-id`) remains in scope as repository-scoped diagnostic metadata; only the legacy DB adapter surface is retired.