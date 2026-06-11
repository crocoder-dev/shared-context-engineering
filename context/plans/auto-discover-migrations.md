# Auto-Discover Embedded Migrations

## Change summary

Replace hardcoded per-database migration lists with a `build.rs`-generated module that scans `cli/migrations/` directories at compile time, embeds SQL files via `include_str!`, and produces sorted `(id, sql)` arrays consumed by `DbSpec::migrations()`. This removes the need to manually update Rust code when adding new `.sql` migration files.

## Success criteria

- [ ] No hardcoded migration IDs or `include_str!` calls exist in `agent_trace_db/mod.rs` or `auth_db/mod.rs`.
- [ ] `build.rs` generates a `src/generated_migrations.rs` module with `&[(&str, &str)]` arrays for each migration directory.
- [ ] Migration ordering is deterministic: files are sorted by the numeric prefix before the first `_`.
- [ ] Tests assert dynamically (count, sortedness, prefix pattern) instead of hardcoding full expected ID lists.
- [ ] `nix flake check` passes after all changes.

## Constraints and non-goals

- **In scope**: `build.rs` generation, `DbSpec` trait migration source, test assertions, removal of hardcoded constants.
- **Out of scope**: Changing the `__sce_migrations` table schema, runtime migration discovery, or altering migration SQL files.
- **Non-goal**: Supporting non-numeric prefixes or sub-directory nesting beyond `migrations/<db-name>/`.
- **Assumption**: Migration filenames follow the pattern `NNN_description.sql` where `NNN` is a zero-padded sortable prefix.

## Task stack

- [x] T01: Extend `build.rs` to scan and generate migration arrays (status: done)
- [x] T02: Update `DbSpec` consumers to use generated migration arrays (status: done)
- [ ] T03: Convert tests to dynamic migration assertions (status: todo)
- [ ] T04: Validation, cleanup, and context sync (status: todo)

---

### T01: Extend `build.rs` to scan and generate migration arrays

- **Status:** done
- **Completed:** 2026-06-11
- **Files changed:** `cli/build.rs`, `cli/src/generated_migrations.rs`
- **Evidence:** `nix develop -c sh -c 'cd cli && cargo build'` passed; `nix flake check` passed.
- **Notes:** `build.rs` now scans immediate migration database directories, sorts SQL files by numeric filename prefix, writes deterministic generated constants with `include_str!` references, and emits rerun directives for the migration root, directories, and files.

- **Task ID**: T01
- **Goal**: Add a `build.rs` step that discovers `cli/migrations/*/*.sql`, parses the numeric prefix, sorts files, and writes a generated Rust module (`src/generated_migrations.rs`) containing `&[(&str, &str)]` constants for each database.
- **Boundaries (in/out of scope)**:
  - **In**: `build.rs` file-walking logic, sorting by numeric prefix, generating `include_str!` references, emitting `cargo:rerun-if-changed` for each `.sql` file and the `migrations/` directory.
  - **Out**: Modifying any `src/` files other than adding `mod generated_migrations;` to `main.rs` or `lib.rs`.
- **Done when**:
  - `build.rs` produces `cli/src/generated_migrations.rs` with constants like `AGENT_TRACE_MIGRATIONS`, `AUTH_MIGRATIONS`.
  - Each constant is a `&[(&'static str, &'static str)]` where the `id` is the filename without `.sql` extension and the `sql` is an `include_str!` to the original file.
  - Files are sorted by the numeric prefix before the first `_`.
  - `cargo build` succeeds and `nix flake check` passes.
- **Verification notes (commands or checks)**:
  - `nix develop -c sh -c 'cd cli && cargo build'`
  - Inspect `cli/src/generated_migrations.rs` for correct structure and ordering.
  - `nix flake check`

---

### T02: Update `DbSpec` consumers to use generated migration arrays

- **Status:** done
- **Completed:** 2026-06-11
- **Files changed:** `cli/src/main.rs`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/auth_db/mod.rs`, `cli/src/generated_migrations.rs`
- **Evidence:** `grep -n "include_str.*migrations" cli/src/services/agent_trace_db/mod.rs` and `grep -n "include_str.*migrations" cli/src/services/auth_db/mod.rs` had no matches via equivalent repository search; `nix develop -c sh -c 'cd cli && cargo build'` passed; `nix flake check` passed.
- **Notes:** The generated migration module is now wired into the binary crate, and Agent Trace/Auth `DbSpec` implementations and local test specs return `generated_migrations::AGENT_TRACE_MIGRATIONS` / `generated_migrations::AUTH_MIGRATIONS` instead of local hardcoded arrays. Existing migration-ID tests now derive expected IDs from the generated arrays only to keep T02 checks green; broader dynamic property assertions remain T03.

- **Task ID**: T02
- **Goal**: Remove all hardcoded `include_str!` and `&[(&str, &str)]` migration constants from `agent_trace_db/mod.rs` and `auth_db/mod.rs`, and wire them to the generated arrays.
- **Boundaries (in/out of scope)**:
  - **In**: Deleting hardcoded migration constants and `AGENT_TRACE_MIGRATIONS` / `AUTH_MIGRATIONS` arrays; updating `AgentTraceDbSpec::migrations()` and `AuthDbSpec::migrations()` to return generated constants.
  - **Out**: Changing `LocalDbSpec` (it has zero migrations and can continue returning `&[]` or use an empty generated constant).
- **Done when**:
  - `agent_trace_db/mod.rs` contains no `include_str!("../../../migrations/...")` lines.
  - `auth_db/mod.rs` contains no `include_str!("../../../migrations/...")` lines.
  - `DbSpec::migrations()` implementations reference `generated_migrations::AGENT_TRACE_MIGRATIONS` and `generated_migrations::AUTH_MIGRATIONS`.
  - `nix flake check` passes.
- **Verification notes (commands or checks)**:
  - `grep -n "include_str.*migrations" cli/src/services/agent_trace_db/mod.rs` → no matches
  - `grep -n "include_str.*migrations" cli/src/services/auth_db/mod.rs` → no matches
  - `nix flake check`

---

### T03: Convert tests to dynamic migration assertions

- **Task ID**: T03
- **Goal**: Replace hardcoded `vec!["001_...", "002_..."]` assertions in `agent_trace_db/mod.rs` and `auth_db/mod.rs` tests with dynamic checks that validate properties instead of exact lists.
- **Boundaries (in/out of scope)**:
  - **In**: Test assertions in `agent_trace_db/mod.rs` and `auth_db/mod.rs`.
  - **Out**: Changing production migration logic or the `__sce_migrations` schema.
- **Done when**:
  - No test contains a hardcoded list of migration IDs.
  - Tests verify: applied count equals expected count, IDs are sorted lexicographically, every ID matches `^\d{3}_.+`.
  - `nix flake check` passes.
- **Verification notes (commands or checks)**:
  - `grep -n "001_create" cli/src/services/agent_trace_db/mod.rs` → only in generated file
  - `grep -n "001_create" cli/src/services/auth_db/mod.rs` → only in generated file
  - `nix flake check`

---

### T04: Validation, cleanup, and context sync

- **Task ID**: T04
- **Goal**: Run full validation, remove any temporary scaffolding, and update `context/` files to reflect the new build-time migration generation pattern.
- **Boundaries (in/out of scope)**:
  - **In**: `nix flake check`, `nix run .#pkl-check-generated`, deleting any leftover commented-out code, updating `context/sce/shared-turso-db.md` or `context/sce/agent-trace-db.md` to mention generated migrations.
  - **Out**: Changing Pkl sources or generated config assets.
- **Done when**:
  - `nix flake check` passes cleanly.
  - No stale comments or dead code remain in modified files.
  - Relevant context files mention the `build.rs`-generated migration pattern.
- **Verification notes (commands or checks)**:
  - `nix flake check`
  - `nix run .#pkl-check-generated`
  - Review `git diff --stat` for expected file set

## Open questions

None — all clarifications resolved.
