# sce trace command

Top-level CLI command group exposing Agent Trace database visibility for operators.

Lives under `cli/src/services/trace/` with these subcommands:

- `sce trace db list [--legacy]` — discover repository-scoped Agent Trace DBs under `<state_root>/sce/repos/<repository-id>/agent-trace.db` by default, or old checkout-scoped DBs with `--legacy`.
- `sce trace db shell [repository-id-or-alias] [--legacy]` — open an embedded in-process SQL shell for the current repository DB by default, a discovered repository DB by alias/repository ID, or an explicit legacy checkout DB when `--legacy` is supplied.
- `sce trace status [--legacy]` — render counts and last-activity for the current repository-scoped DB by default, or the current checkout's legacy DB with `--legacy`.
- `sce trace status --all [--legacy]` — aggregate counts across every discovered repository DB by default, or legacy checkout DBs with `--legacy`.

The list/status subcommands declare `--format text|json` via `services::output_format::OutputFormat`; `db shell` is interactive and uses standard input/output directly after successful resolution. Clap surface is defined in `cli/src/cli_schema.rs` (`Commands::Trace`, `TraceSubcommand`, `TraceDbSubcommand`) and dispatched through `services::command_registry` to `services::trace::command::TraceCommand`.

## Implemented behavior

### Discovery — `services::trace::discovery`

`discover_agent_trace_dbs()` scans `<state_root>/sce/repos/*/agent-trace.db`, sorts by file mtime descending (ties broken by repository ID ascending), and assigns positional `agent_trace_{N}` aliases. Each entry carries an mtime-derived `SystemTime`, a `DiscoveredAgentTraceDbKind::Repository { repository_id }`, and a `Readiness` verdict (`Ready` or `Skipped { missing_table }`).

`discover_legacy_agent_trace_dbs()` is the explicit legacy scanner for `<state_root>/sce/agent-trace-{checkout_id}.db`; default trace commands do not use it unless `--legacy` is supplied.

Readiness is probed read-only via the shared Agent Trace DB open-without-migrations path and a `sqlite_master` lookup for each required table in declared order:

```
diff_traces
post_commit_patch_intersections
agent_traces
messages
parts
```

The first missing table is reported as the skip reason. Discovery returns an empty Vec when the scanned directory does not exist.

`resolve_agent_trace_db_identifier(databases, identifier)` accepts either an `agent_trace_N` alias or the discovered database kind identifier (repository ID by default; checkout ID in legacy mode), returns a cloned ready `DiscoveredAgentTraceDb`, rejects unknown/ambiguous identifiers with guidance to run `sce trace db list`, and rejects skipped databases with the stored missing-table readiness reason.

### Embedded shell core — `services::trace::shell`

`run_agent_trace_db_shell(target, input, output)` opens the resolved Agent Trace DB path in-process without running migrations, verifies schema readiness, prints alias, scope (`repository` or `legacy checkout`), identifier, and database path, then runs a minimal SQL shell over caller-provided `BufRead`/`Write` streams. The core supports `.help`, `.tables`, `.exit`, and `.quit`, splits single-line input on semicolons, executes query statements through `TursoDb::query_values`, executes non-query statements through `execute`, and renders deterministic text rows.

Default `sce trace db shell` resolves the current repository-scoped DB through the same storage context used by hook runtime. `sce trace db shell <identifier>` resolves a discovered repository DB by alias or repository ID. `sce trace db shell --legacy <identifier>` is required for old checkout-scoped DBs. The shell is embedded-only and never shells out to `turso`, `sqlite3`, or another external database CLI.

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

`resolve_current_legacy_status_in(repo_root, sce_dir)` keeps the old checkout-scoped behavior for `sce trace status --legacy`: it reads `<git-dir>/sce/checkout-id` and inspects `<state_root>/sce/agent-trace-{checkout_id}.db` without creating or selecting it as active storage.

Text output includes `Repository: <repository-id>` when repository-scoped, then checkout ID, database path, readiness, row counts, and last activity. JSON includes `repository_id` (null for legacy), `checkout_id`, `database_path`, `db_status`, `stats` for ready DBs, and `skip_reason` for skipped DBs.

### `sce trace status --all` aggregation/rendering — `services::trace::status_all`, `render_status_all`

`aggregate_current_status_all(legacy)` resolves `<state_root>/sce/` and delegates to repository discovery by default or legacy discovery when `legacy == true`. It runs `collect_agent_trace_db_stats` on each ready DB and accumulates totals for `diff_traces`, `messages`, `parts`, `agent_traces`, `post_commit_patch_intersections`, and max `last_activity`. Skipped DBs are excluded from totals but included in discovery summary and breakdown rows.

Text rendering shows discovery summary, totals, and a `By database` table with `Alias`, `Scope`, `ID`, `Status`, and count columns. JSON entries use `scope` and `identifier` for both repository and legacy rows.

## Related context

- [agent-trace-storage.md](agent-trace-storage.md) — repository-scoped storage resolver and active DB path contract.
- [checkout-identity.md](checkout-identity.md) — checkout identity diagnostics and legacy DB handling.
- [default-path-catalog.md](default-path-catalog.md) — Agent Trace DB path ownership.
- [styling-service.md](styling-service.md) — heading helper used by text renderers.
- [../sce/agent-trace-db.md](../sce/agent-trace-db.md) — Agent Trace DB schema and migration ownership.
