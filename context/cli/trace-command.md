# sce trace command

Top-level CLI command group exposing Agent Trace database visibility for operators.

Lives under `cli/src/services/trace/` with these subcommands:

- `sce trace db list` — discover repository-scoped Agent Trace DBs under `<state_root>/sce/repos/<repository-id>/agent-trace.db`.
- `sce trace db shell [repository-id-or-alias]` — open an embedded in-process SQL shell for the current repository DB by default, or a discovered repository DB by alias/repository ID.
- `sce trace status` — render counts and last-activity for the current repository-scoped DB.
- `sce trace status --all` — aggregate counts across every discovered repository DB.

`sce trace` operates only on repository-scoped DBs; there is no `--legacy` flag. The `retire-legacy-agent-trace-db` plan removed checkout-scoped discovery/status/shell access. Any pre-migration `<state_root>/sce/agent-trace-*.db` files left on disk are never touched by SCE and are no longer inspectable through the CLI.

The list/status subcommands declare `--format text|json` via `services::output_format::OutputFormat`; `db shell` is interactive and uses standard input/output directly after successful resolution. Clap surface is defined in `cli/src/cli_schema.rs` (`Commands::Trace`, `TraceSubcommand`, `TraceDbSubcommand`) and dispatched through `services::command_registry` to `services::trace::command::TraceCommand`.

## Implemented behavior

### Discovery — `services::trace::discovery`

`discover_agent_trace_dbs()` scans `<state_root>/sce/repos/*/agent-trace.db`, sorts by file mtime descending (ties broken by repository ID ascending), and assigns positional `agent_trace_{N}` aliases. Each entry carries an mtime-derived `SystemTime`, a `DiscoveredAgentTraceDbKind::Repository { repository_id }`, and a `Readiness` verdict (`Ready` or `Skipped { missing_table }`). There is no checkout-scoped discovery kind or scanner; the `retire-legacy-agent-trace-db` plan removed `DiscoveredAgentTraceDbKind::LegacyCheckout` and `discover_legacy_agent_trace_dbs*`. SCE never migrates, imports, renames, deletes, or backfills any pre-migration checkout-scoped files into repository-scoped databases.

Readiness is probed read-only via the shared Agent Trace DB open-without-migrations path and a `sqlite_master` lookup for each required table in declared order:

```
diff_traces
post_commit_patch_intersections
agent_traces
messages
parts
```

The first missing table is reported as the skip reason. Discovery returns an empty Vec when the scanned directory does not exist.

`resolve_agent_trace_db_identifier(databases, identifier)` accepts either an `agent_trace_N` alias or the discovered database's repository ID, returns a cloned ready `DiscoveredAgentTraceDb`, rejects unknown/ambiguous identifiers with guidance to run `sce trace db list`, and rejects skipped databases with the stored missing-table readiness reason.

### Embedded shell core — `services::trace::shell`

`run_agent_trace_db_shell(target, input, output)` opens the resolved repository-scoped Agent Trace DB path in-process without running migrations (via `RepositoryAgentTraceDb::open_for_hooks_without_migrations_at`), verifies schema readiness, prints alias, scope (`repository`), identifier, and database path, then runs a minimal SQL shell over caller-provided `BufRead`/`Write` streams. The core supports `.help`, `.tables`, `.exit`, and `.quit`, splits single-line input on semicolons, executes query statements through `TursoDb::query_values`, executes non-query statements through `execute`, and renders deterministic text rows.

Default `sce trace db shell` resolves the current repository-scoped DB through the same storage context used by hook runtime. `sce trace db shell <identifier>` resolves a discovered repository DB by alias or repository ID. The shell is embedded-only and never shells out to `turso`, `sqlite3`, or another external database CLI.

### `sce trace db list` rendering — `services::trace::render_list`

Text output is `services::style::heading("SCE trace db list")` followed by a padded table with `Alias`, `Scope`, `ID`, `Status`, `Updated at`, and `Path`. Empty-state output is the heading plus `no agent-trace databases discovered`.

JSON output shape:

```json
{
  "status": "ok",
  "command": "trace",
  "subcommand": "db.list",
  "databases": [
    {
      "alias": "agent_trace_0",
      "scope": "repository",
      "identifier": "<repository-id>",
      "path": "/.../repos/<repository-id>/agent-trace.db",
      "status": "ready",
      "updated_at": "2026-06-27T12:34:56+00:00"
    }
  ]
}
```

`skip_reason` is omitted when `status == "ready"`. Text `Updated at` is rendered as `YYYY-MM-DD HH:MM:SS UTC`; JSON `updated_at` is RFC3339.

### `sce trace status` resolution/rendering — `services::trace::status`, `render_status`

`resolve_current_status(repo_root)` resolves config-backed Agent Trace storage (`agent_trace.repository_id` or configured remote, default `origin`) through `agent_trace_storage`, creating/reusing checkout identity for diagnostics and selecting `<state_root>/sce/repos/<repository-id>/agent-trace.db`. It probes schema readiness and, when ready, collects row counts and last-activity via `services::trace::stats::collect_agent_trace_db_stats`.

Text output includes `Repository: <repository-id>`, then checkout ID, database path, readiness, row counts, and last activity. JSON includes `repository_id`, `checkout_id`, `database_path`, `db_status`, `stats` for ready DBs, and `skip_reason` for skipped DBs.

### `sce trace status --all` aggregation/rendering — `services::trace::status_all`, `render_status_all`

`aggregate_current_status_all()` resolves `<state_root>/sce/` and delegates to repository discovery. It runs `collect_agent_trace_db_stats` on each ready DB and accumulates totals for `diff_traces`, `messages`, `parts`, `agent_traces`, `post_commit_patch_intersections`, and max `last_activity`. Skipped DBs are excluded from totals but included in discovery summary and breakdown rows.

Text rendering shows discovery summary, totals, and a `By database` table with `Alias`, `Scope`, `ID`, `Status`, and count columns. JSON entries use `scope` (`repository`) and `identifier`.

## Related context

- [agent-trace-storage.md](agent-trace-storage.md) — repository-scoped storage resolver and active DB path contract.
- [checkout-identity.md](checkout-identity.md) — checkout identity diagnostics and never-touch on-disk handling of pre-migration DB files.
- [default-path-catalog.md](default-path-catalog.md) — Agent Trace DB path ownership.
- [styling-service.md](styling-service.md) — heading helper used by text renderers.
- [../sce/agent-trace-db.md](../sce/agent-trace-db.md) — Agent Trace DB schema and migration ownership.
