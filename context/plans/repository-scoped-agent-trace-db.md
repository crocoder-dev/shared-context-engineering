# Repository-scoped Agent Trace database

## Change summary

Change Agent Trace persistence from checkout-scoped database files to repository-scoped database files. The target invariant is:

```text
one logical Git repository = one Agent Trace database
```

The active Agent Trace database is selected by a stable `repository_id` and stored at:

```text
<sce-state-root>/sce/repos/<repository-id>/agent-trace.db
```

The existing checkout identity remains intact for identifying clones/worktrees and for diagnostics, but this plan intentionally does **not** add `checkout_id` columns to Agent Trace rows. Agent Trace rows and operational attribution are repository-level within the shared repository database. Existing checkout-scoped databases such as `<sce-state-root>/sce/agent-trace-<checkout-id>.db` are legacy data and must not be migrated, renamed, deleted, modified, imported, or selected for new writes.

Initial code inspection found the current checkout-scoped behavior in these areas:

- Checkout identity and active DB opening: `cli/src/services/checkout/mod.rs`, especially `resolve_or_create_agent_trace_db_for_checkout()`.
- Agent Trace path construction: `cli/src/services/default_paths.rs`, especially `agent_trace_db_path_for_checkout()` and legacy `agent_trace_db_path()`.
- Database lifecycle and migrations: `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/agent_trace_db/mod.rs`, and `cli/migrations/agent-trace/*.sql`.
- Hook-time opening and attribution: `cli/src/services/hooks/mod.rs`, including diff-trace insert, post-commit intersection, Agent Trace persistence, and commit-msg staged-overlap preflight.
- Recent trace selection: `AgentTraceDb::recent_diff_trace_patches()` in `cli/src/services/agent_trace_db/mod.rs`.
- Trace status, discovery, listing, and shell: `cli/src/services/trace/{status,discovery,render_list,render_status,status_all,render_status_all,shell,stats}.rs`.
- Doctor/setup diagnostics: `cli/src/services/agent_trace_db/lifecycle.rs` and `cli/src/services/doctor/inspect.rs`.
- Configuration resolution: `cli/src/services/config/{schema,resolver,types}.rs`, generated schema assets, and Pkl source `config/pkl/base/sce-config-schema.pkl`.

## Success criteria

- Each logical Git repository resolves to exactly one active repository-scoped Agent Trace database.
- Different logical repositories resolve to different repository IDs and different database paths.
- Multiple clones and linked worktrees of the same logical repository resolve to the same repository-scoped database path.
- Checkout IDs remain distinct per clone/worktree and are visible in storage context/diagnostics, but are not persisted on Agent Trace rows.
- Operational commit attribution, post-commit intersections, Agent Trace persistence, and co-author decisions consume repository-level trace data from the shared repository database.
- Active database path is `<sce-state-root>/sce/repos/<repository-id>/agent-trace.db`.
- No global active Agent Trace database is introduced or used for new writes.
- Existing checkout-scoped databases remain byte-for-byte untouched and no migration/import/archive/delete/rename flow is added.
- Repository identity precedence is implemented: explicit config identity, then selected remote URL, with default remote `origin`.
- Equivalent SSH/SCP/HTTPS remote URLs canonicalize to the same safe canonical identity and hash to the same repository ID.
- Raw credential-bearing remote URLs never appear in repository IDs, paths, doctor/status diagnostics, or metadata.
- Missing explicit identity and missing usable remote produce an actionable error explaining `.sce/config.json` configuration.
- New repository-scoped DBs are initialized from one schema SQL file, contain repository metadata, and validate stored `repository_id` on reopen.
- Concurrent first-time initialization is safe and idempotent.
- Relevant CLI output uses repository-scoped terminology and shows repository identity source, repository ID, canonical identity when safe, configured remote, checkout ID, DB path, and schema status.
- Legacy checkout-scoped DB inspection is available only behind an explicit `--legacy` flag.
- Documentation explains repository-scoped storage, checkout identity without row-level provenance, repository-level attribution within a shared repository DB, legacy database handling, and no daemon/background process.
- Validation passes with `nix flake check`; generated config parity passes with `nix run .#pkl-check-generated` when generated assets change.

## Constraints and non-goals

- Do not migrate, import, copy, rename, delete, archive, clean up, or modify existing checkout-scoped databases.
- Do not add legacy migration markers to old databases.
- Do not fall back to local paths, Git directories, checkout IDs, random UUIDs, or one global database when repository identity cannot be resolved.
- Do not log or render credential-bearing remote URLs.
- Do not introduce a daemon, resident process, scheduler, file watcher, external lock server, registry service, or central mutable JSON registry.
- Keep database creation command-driven through existing command/hook flows.
- Preserve existing checkout identity storage under the checkout-specific Git directory.
- Use repo config keys `agent_trace.repository_id` and `agent_trace.repository_remote`; default `repository_remote` to `origin`.
- Treat old checkout-scoped DB listing/shell behavior as legacy inspection only behind an explicit `--legacy` flag; never show legacy files as active DBs or choose them as active write targets.
- Because repository-scoped DB files are new, define the fresh schema in one schema SQL file without adding `checkout_id` columns.

## Task stack

- [x] T01: `Add Agent Trace repository config resolution` (status:done)
  - Task ID: T01
  - Goal: Add typed config support for `agent_trace.repository_id` and `agent_trace.repository_remote` with default remote `origin`.
  - Boundaries (in/out of scope): In - Rust config types/resolver/schema validation, Pkl schema source, generated schema assets, tests for explicit ID and remote default/override. Out - Git remote canonicalization, DB path changes, hook query changes.
  - Done when: Config files can specify optional explicit repository identity and selected remote name; invalid shapes are rejected; config rendering/validation remains deterministic; generated outputs are in sync.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; targeted Rust config tests via `nix develop -c sh -c 'cd cli && cargo test config'` if needed.
  - Completed: 2026-07-17
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json` (generated), `cli/assets/generated/config/schema/sce-config.schema.json` (generated sync), `cli/src/services/config/schema.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/config/render.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` pass; `cli-clippy` + `cli-fmt` checks pass; `nix run .#pkl-check-generated` reports "Generated outputs are up to date."
  - Notes: Top-level `agent_trace` object added to the config schema; resolver exposes `agent_trace_repository_id` (`ResolvedOptionalValue`) and `agent_trace_repository_remote` (`ResolvedValue`, default `origin` via `DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE`); `sce config show` renders both keys in text and JSON. No env-var layer was added (config-file + default only, per plan). T03 will add the runtime accessor/consumer.

- [x] T02: `Implement repository identity canonicalization and hashing` (status:done)
  - Task ID: T02
  - Goal: Add a pure repository identity module that canonicalizes explicit identities and Git remote URLs, then derives `sha256("sce-repository-id-v1\0" + canonical_identity)` hex IDs.
  - Boundaries (in/out of scope): In - canonicalization for SCP-style SSH, `ssh://`, HTTPS, hostname lowercasing, credential stripping, default port removal, query/fragment/trailing slash/trailing `.git` cleanup, safe diagnostics, tests. Out - opening databases or reading Git config from real repos.
  - Done when: Equivalent GitHub SSH/SCP/HTTPS URLs hash to the same ID; distinct identities hash differently; credential-bearing inputs do not leak credentials into returned canonical identity, ID, or errors.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test repository_identity'` or the matching module test name.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/repository_identity.rs` (new), `cli/src/services/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` pass (153 tests, 12 new `repository_identity` module tests); `cli-clippy` + `cli-fmt` checks pass.
  - Notes: Pure module, no new dependencies (hand-rolled URL normalization; `sha2` reused). Canonical identity is scheme-neutral `host[:port]/path` so SSH/SCP/HTTPS equivalents converge; supported schemes `ssh`/`git+ssh`/`ssh+git`/`http`/`https`/`git` with default ports 22/80/443/9418 removed; IPv6 bracketed hosts supported. Explicit config identities are trimmed and used verbatim (opaque, not URL-parsed). `RepositoryIdentityError` variants carry no input fragments so credentials cannot leak via errors. Module is `#[allow(dead_code)]` until T03 wires the runtime resolver.

- [x] T03: `Resolve repository identity from config and Git remotes` (status:done)
  - Task ID: T03
  - Goal: Add runtime resolution that applies precedence: explicit config identity, selected Git remote URL, default selected remote `origin`, otherwise actionable error.
  - Boundaries (in/out of scope): In - Git remote lookup helper, config-driven remote name, missing-identity error text, tests with temp Git repos/remotes. Out - DB creation, schema changes, trace CLI rendering.
  - Done when: Explicit identity overrides remotes; configured remote name is honored; missing explicit identity and missing usable remote errors with `.sce/config.json` guidance; local paths are not used implicitly.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test repository_identity'` or exact resolver tests.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/repository_identity.rs` → `cli/src/services/repository_identity/mod.rs` (moved, doc header updated), `cli/src/services/repository_identity/resolve.rs` (new)
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` pass (163 tests, 10 new `repository_identity::resolve` tests including temp-git-repo remote cases); `cli-clippy` + `cli-fmt` checks pass.
  - Notes: `resolve::resolve_repository_identity(repo_root, explicit, remote_name)` shells out to `git config --get remote.<name>.url`; `resolve_repository_identity_with_lookup` is the injectable-lookup precedence core. `ResolvedRepositoryIdentity` carries a `RepositoryIdentitySource` (`ExplicitConfig` vs `RemoteUrl { remote_name }`) for T10 diagnostics. `RepositoryIdentityResolutionError` (`InvalidExplicitIdentity`, `InvalidRemoteUrl`, `MissingIdentity`) never echoes URLs, and all Display messages include `.sce/config.json` guidance. Local-path remotes fail as `InvalidRemoteUrl` (no implicit fallback); git-unavailable/non-repo lookups map to `MissingIdentity`. Still `#[allow(dead_code)]` at the module level until T04 consumes it.

- [x] T04: `Add repository-scoped Agent Trace storage resolver` (status:done)
  - Task ID: T04
  - Goal: Replace checkout-path-oriented active DB resolution with `AgentTraceStorageContext` and `ResolvedAgentTraceStorage` that return repository ID, checkout ID, and `<state-root>/sce/repos/<repository-id>/agent-trace.db`.
  - Boundaries (in/out of scope): In - new path helper, directory creation, checkout ID reuse, DB open/create, safe concurrent/idempotent initialization path, tests for path separation and clone/worktree consolidation. Out - changing schema or hook query semantics.
  - Done when: Active resolver creates repository directories and opens repository-scoped DBs; different repository IDs produce different paths; equivalent clones/worktrees share the path while retaining distinct checkout IDs; no active global/checkout DB path is created.
  - Verification notes (commands or checks): targeted checkout/storage resolver tests; inspect that old `agent-trace-<checkout-id>.db` paths are not used by the active resolver.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/agent_trace_storage/mod.rs` (new), `cli/src/services/default_paths.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` pass (171 tests, 8 new `agent_trace_storage` tests covering repo separation, SSH/HTTPS clone consolidation, linked-worktree consolidation, explicit-ID override, idempotent re-resolution, missing-identity guidance, and path-segment validation); `cli-clippy` + `cli-fmt` checks pass.
  - Notes: `resolve_agent_trace_storage(context)` is the production entrypoint; `resolve_agent_trace_storage_at_state_root(context, state_root)` is the injectable core used by tests. Context takes already-resolved config values (explicit ID + remote name); T08 wires config/hooks/lifecycle call sites. `default_paths::agent_trace_db_path_for_repository{,_at}` rejects empty/path-unsafe repository IDs. Directory creation rides on `TursoDb` parent-dir `create_dir_all` (idempotent, safe for concurrent first open); DB open reuses the fast-path-then-migrate pattern from the checkout resolver. Failed identity resolution creates no state directories. Module is `#[allow(dead_code)]` until T08 consumes it; the legacy checkout resolver remains untouched and active until then.

- [x] T05: `Define one-file repository-scoped Agent Trace schema` (status:done)
  - Task ID: T05
  - Goal: Replace the checkout-scoped Agent Trace schema baseline with one repository-scoped schema SQL file that includes repository metadata and keeps trace tables repository-level.
  - Boundaries (in/out of scope): In - one fresh schema SQL file covering `repository_metadata`, existing Agent Trace tables, repository-level indexes/constraints, and metadata validation on open. Out - old database alteration/migration/import, `checkout_id` columns on trace tables, checkout-scoped attribution queries, and a new chain of incremental SQL files for the repository-scoped DB baseline.
  - Done when: New DBs are initialized from one fresh schema SQL file and have metadata matching the resolved repository ID; opening a DB with mismatched metadata errors; no `checkout_id` columns are added to Agent Trace row tables.
  - Verification notes (commands or checks): targeted AgentTraceDb schema tests; assert no code path opens legacy checkout DBs for migration.
  - Completed: 2026-07-17
  - Files changed: `cli/migrations/agent-trace-repository/001_repository_schema.sql`, `cli/src/generated_migrations.rs`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/agent_trace_db/repository.rs`, `cli/src/services/agent_trace_storage/mod.rs`, `cli/src/services/db/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` pass; `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass; `nix run .#pkl-check-generated` reports "Generated outputs are up to date."
  - Notes: Added the `agent-trace-repository` one-file fresh schema baseline with `repository_metadata`, existing repository-level Agent Trace tables/indexes/triggers, and no `checkout_id` trace columns. Added `RepositoryAgentTraceDb` over the shared Turso adapter, metadata seed/validation helpers, schema readiness tests, and storage resolver metadata validation. Shared migration execution now uses batch execution so the one-file multi-statement baseline runs as one recorded migration.

- [x] T06: `Keep Agent Trace writes repository-level` (status:done)
  - Task ID: T06
  - Goal: Update Agent Trace write paths to use the repository-scoped database while preserving current row shapes without `checkout_id` fields.
  - Boundaries (in/out of scope): In - Rust insert structs/constants/SQL/tests updated only as needed for the fresh one-file schema and repository DB opening. Out - adding checkout provenance columns or checkout-scoped write APIs.
  - Done when: Diff traces, messages, parts, Agent Trace rows, and post-commit intersections write successfully into the repository-scoped DB using repository-level row schemas.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test agent_trace_db'` or exact insert tests.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/agent_trace_db/repository.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass.
  - Notes: `RepositoryAgentTraceDb` now exposes repository-level write methods for diff traces, post-commit patch intersections, Agent Trace rows, messages, and parts by delegating to the existing generic insert helpers and SQL. Added repository-scoped insert coverage for all row families against the fresh one-file schema, with no `checkout_id` columns or checkout-provenance write API added.

- [x] T07: `Use repository-level attribution queries` (status:done)
  - Task ID: T07
  - Goal: Ensure recent diff trace reads and commit attribution decisions operate against the current repository-scoped database without checkout filtering.
  - Boundaries (in/out of scope): In - preserve `recent_diff_trace_patches(cutoff, end)` repository-level semantics, post-commit intersection, tool/model selection, Agent Trace commit association, commit-msg staged-overlap preflight, tests for repository-level behavior. Out - adding `checkout_id` parameters or cross-checkout isolation guarantees.
  - Done when: Commit attribution consumes recent traces from the current repository DB; different repositories remain isolated by repository ID and database path.
  - Verification notes (commands or checks): targeted hooks/agent_trace_db tests covering same-repository shared DB behavior and different-repository DB separation.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/agent_trace_db/repository.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass.
  - Notes: Added `RepositoryAgentTraceDb::recent_diff_trace_patches(cutoff, end)` delegating to the shared recent diff-trace parser/query helper, preserving repository-level semantics with no checkout filter. Added repository DB tests for same-repository row loading across checkout-like session IDs and separate repository DB path isolation. Active hook/lifecycle opening remains deferred to T08 by plan scope.

- [x] T08: `Wire hooks and lifecycle to repository storage context` (status:done)
  - Task ID: T08
  - Goal: Update setup, doctor/lifecycle, and hook runtime opening to use repository-scoped storage context while keeping Agent Trace writes and queries repository-level.
  - Boundaries (in/out of scope): In - `open_agent_trace_db_for_hook_runtime`, setup messages, lifecycle health, hook diagnostics, no-migration fast-path behavior against repository DBs. Out - trace list/status/shell UX changes beyond compilation needs.
  - Done when: Setup initializes the repository-scoped database; hooks lazily resolve repository storage; errors mention repository identity guidance where relevant; checkout-scoped active DB opening is removed from runtime write paths.
  - Verification notes (commands or checks): targeted setup/hooks/lifecycle tests; inspect `resolve_or_create_agent_trace_db_for_checkout` removal or legacy-only status.
  - Completed: 2026-07-17
  - Files changed: `cli/src/services/hooks/mod.rs`, `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/config/{mod.rs,resolver.rs,types.rs}`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/checkout/mod.rs`, `context/{overview.md,architecture.md,glossary.md,context-map.md}`, `context/cli/{agent-trace-storage.md,checkout-identity.md,cli-command-surface.md,default-path-catalog.md,repository-identity.md,service-lifecycle.md}`, `context/sce/{agent-trace-db.md,agent-trace-hooks-command-routing.md}`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` pass; `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass; `nix run .#pkl-check-generated` pass; `git diff --check` pass; `nix flake check` pass. Direct targeted `cargo test` was blocked by repo bash policy, so the Crane-backed CLI test derivation was used instead.
  - Notes: Hook runtime now resolves `agent_trace.repository_id` / `agent_trace.repository_remote` config and opens `RepositoryAgentTraceDb` through `resolve_agent_trace_storage(...)`, so diff-trace, conversation-trace, post-commit, and commit-msg attribution read/write repository-level rows. Agent Trace lifecycle setup initializes repository-scoped storage and reports repository + checkout identity in setup messages; lifecycle health resolves the repository DB path from repository identity and uses no-migration schema checks against `RepositoryAgentTraceDb`. Legacy checkout DB opening remains present but `#[allow(dead_code)]` and no longer participates in active hook/lifecycle write paths; trace UX migration is deferred to T09.

- [x] T09: `Update trace status/list/shell discovery with --legacy` (status:done)
  - Task ID: T09
  - Goal: Make `sce trace status`, `trace db list`, status-all, and DB shell understand repository-scoped active databases while keeping legacy checkout DBs inspectable only through `--legacy`.
  - Boundaries (in/out of scope): In - repository DB discovery under `repos/<repo-id>/agent-trace.db`, active current-repo status, aliases/identifiers including repository ID, explicit `--legacy` discovery/listing/shell path for old checkout-scoped DBs, shell target metadata. Out - changing hook persistence behavior.
  - Done when: Current status shows the current repository-scoped DB and checkout ID; list/status-all preserve separation between repositories; legacy `agent-trace-<checkout-id>.db` files are hidden by default and available only through `--legacy`; shell opens the resolved repository DB or explicit legacy target only.
  - Verification notes (commands or checks): targeted trace discovery/render/shell tests; assert multiple repository DBs remain separate.
  - Completed: 2026-07-17
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/command_registry.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/trace/{command.rs,discovery.rs,mod.rs,render_list.rs,render_status.rs,render_status_all.rs,shell.rs,status.rs,status_all.rs}`, `context/{overview.md,context-map.md}`, `context/cli/{agent-trace-storage.md,checkout-identity.md,trace-command.md}`, `context/sce/agent-trace-db.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` pass; `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` pass.
  - Notes: Trace discovery now defaults to repository-scoped DBs under `<state_root>/sce/repos/<repository-id>/agent-trace.db`; old checkout-scoped `agent-trace-<checkout-id>.db` discovery is explicit via `--legacy`. `sce trace status` resolves active repository storage and reports repository ID + checkout ID diagnostics; `sce trace db shell` opens the current repository DB by default or resolves repository aliases/IDs, while `--legacy` requires an explicit legacy alias/checkout ID. List/status-all render scope + identifier fields for repository and legacy rows.

- [ ] T10: `Harden diagnostics and credential-safe output` (status:todo)
  - Task ID: T10
  - Goal: Update setup, doctor, trace status/list/shell, and related JSON/text renderers to use repository-scoped terminology and safe identity metadata.
  - Boundaries (in/out of scope): In - render repository identity source, repository ID, safe canonical identity, configured remote, checkout ID, repository-scoped path, schema status; redact/avoid raw URLs. Out - new storage/query behavior.
  - Done when: User-facing diagnostics never display credentials; active database is described as repository-scoped; actionable missing-identity guidance is visible.
  - Verification notes (commands or checks): targeted render tests with credential-bearing remotes; `rg` for old active checkout-scoped wording in non-historical docs/code.

- [ ] T11: `Add end-to-end repository storage behavior tests` (status:todo)
  - Task ID: T11
  - Goal: Add integration-style tests covering repository separation, clone/worktree consolidation, repository-level attribution behavior, existing-data non-migration, and concurrent initialization.
  - Boundaries (in/out of scope): In - temp Git repositories/clones/worktrees, multiple remotes, existing legacy DB byte-for-byte assertions, concurrent first-open test, empty new DB assertion. Out - production code changes except small testability seams discovered while writing tests.
  - Done when: Repository separation, clone/worktree consolidation, security, repository-level DB behavior, existing-data, and concurrency cases are covered by automated tests where practical.
  - Verification notes (commands or checks): targeted exact tests, then `nix develop -c sh -c 'cd cli && cargo test repository_scoped_agent_trace'` or matching module filters.

- [ ] T12: `Document repository-scoped Agent Trace storage` (status:todo)
  - Task ID: T12
  - Goal: Update repository docs and SCE context to explain repository-scoped Agent Trace DBs and legacy DB handling.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-db.md`, hook routing/status command docs, context map, README or CLI docs as relevant, example directory tree. Out - code behavior changes.
  - Done when: Docs state that SCE creates one Agent Trace DB per logical Git repository; clones/worktrees share it; checkout ID remains a checkout identity but is not stored on trace rows; commit attribution is repository-level within the shared repository DB; old checkout DBs remain untouched and historical data is not migrated; no daemon/background service exists.
  - Verification notes (commands or checks): docs review; `nix run .#pkl-check-generated` if generated docs/config are touched.

- [ ] T13: `Final validation and cleanup` (status:todo)
  - Task ID: T13
  - Goal: Run full validation, remove temporary scaffolding, and sync context after implementation.
  - Boundaries (in/out of scope): In - `nix flake check`, generated-output parity, focused searches for stale checkout-scoped active DB assumptions, context sync verification. Out - new feature behavior beyond fixes required by validation failures.
  - Done when: Full checks pass; generated config is up to date; no temporary files remain; context docs reflect final behavior; plan status/evidence is updated.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `git diff --check`; targeted `rg` for stale active `agent-trace-{checkout_id}.db` terminology excluding historical/legacy references.

## Open questions

None blocking. Decisions captured in this plan:

- Config keys: `agent_trace.repository_id` and `agent_trace.repository_remote`.
- Legacy checkout-scoped DBs are hidden by default and inspectable through `--legacy`.
- Repository-scoped Agent Trace DBs use one fresh schema SQL file.
- No `checkout_id` columns are added to Agent Trace row tables.
- Attribution is repository-level within each repository-scoped DB, not checkout-scoped.
