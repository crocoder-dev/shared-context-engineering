# sce trace command

Top-level CLI command group exposing Agent Trace database visibility for operators.

Lives under `cli/src/services/trace/` with three subcommands:

- `sce trace db list` — discover per-checkout Agent Trace DBs under `<state_root>/sce/agent-trace-*.db` and render an alias / status / path table.
- `sce trace status` — render counts and last-activity for the cwd's checkout DB.
- `sce trace status --all` — aggregate counts across every discovered DB.

All three subcommands declare `--format text|json` via `services::output_format::OutputFormat`. Clap surface is defined in `cli/src/cli_schema.rs` (`Commands::Trace`, `TraceSubcommand`, `TraceDbSubcommand`) and dispatched through `services::command_registry` to `services::trace::command::TraceCommand`.

## Implemented behavior

### Discovery — `services::trace::discovery`

`discover_agent_trace_dbs()` scans the resolved `<state_root>/sce/` directory for `agent-trace-{checkout_id}.db` files, sorts by file mtime descending (ties broken by `checkout_id` ascending), and assigns positional `agent_trace_{N}` aliases. Each entry carries an mtime-derived `SystemTime`, the parsed `checkout_id`, and a `Readiness` verdict (`Ready` or `Skipped { missing_table }`).

Readiness is probed read-only via `AgentTraceDb::open_for_hooks_without_migrations_at` and a `sqlite_master` lookup for each required table in declared order:

```
diff_traces
post_commit_patch_intersections
agent_traces
messages
parts
session_models
```

The first missing table is reported as the skip reason. The discovery module returns an empty Vec when the `sce` directory does not exist; callers do not need to special-case that.

### `sce trace db list` rendering — `services::trace::render_list`

`render(databases, format)` dispatches to the text or JSON renderer.

**Text** — `services::style::heading("SCE trace db list")` followed by a 3-column padded table:

```
Alias          Status                                              Path
agent_trace_0  ready                                               /path/to/agent-trace-aaaa.db
agent_trace_1  skipped: missing table 'post_commit_patch_intersections'  /path/to/agent-trace-bbbb.db
```

Empty-state output is the heading plus `no agent-trace databases discovered`.

**JSON** — stable shape:

```json
{
  "status": "ok",
  "command": "trace",
  "subcommand": "db.list",
  "databases": [
    {
      "alias": "agent_trace_0",
      "checkout_id": "aaaa",
      "path": "/path/to/agent-trace-aaaa.db",
      "status": "ready",
      "mtime": "2026-06-27T12:34:56+00:00"
    },
    {
      "alias": "agent_trace_1",
      "checkout_id": "bbbb",
      "path": "/path/to/agent-trace-bbbb.db",
      "status": "skipped",
      "skip_reason": "missing table: post_commit_patch_intersections",
      "mtime": "2026-06-27T12:34:51+00:00"
    }
  ]
}
```

`skip_reason` is omitted when `status == "ready"`. `mtime` is RFC3339 derived from the discovery `SystemTime`.

### `sce trace status` resolution — `services::trace::status`

`resolve_current_status(repo_root)` resolves the cwd's git directory via `services::checkout::resolve_git_dir`, reads the stored checkout id via `read_checkout_id`, computes the canonical `<state_root>/sce/agent-trace-{id}.db` path, and probes schema readiness (reusing the discovery-layer probe). When ready it also collects row counts and last-activity via `services::trace::stats::collect_agent_trace_db_stats`. Returns either a `StatusReport { checkout_id, database_path, db_status: DbStatus::{Ready { stats, last_activity }, Skipped { missing_table }} }` or a `StatusErrorOrRuntime`.

Three user-actionable error variants (`StatusError::{NotInGitRepo, NoCheckoutId, DbMissing}`) are mapped at the command boundary to `ClassifiedError::validation` (exit code 3) with stable messages directing the user to cd into a git repo, run `sce setup`, or wait for traces to be recorded. Sqlite/IO failures stay runtime-class (exit 4).

A `resolve_current_status_in(repo_root, sce_dir)` variant takes the `sce` directory explicitly for unit-test fixtures.

### `sce trace status` rendering — `services::trace::render_status`

**Text** — `services::style::heading("SCE trace status")` followed by:

```
Checkout: <uuid>
Database: <absolute path>
Status: ready
Diff traces: N
Messages: N
Parts: N
Session models: N
Agent traces: N
Post-commit intersections: N
Last activity: 2026-06-27T22:39:03.926+00:00
```

When `last_activity` is `None` the value is rendered as `never`. When the DB exists but a required table is missing, the per-checkout block ends after `Status: skipped: missing table '<name>'` with no stats lines (exit 0).

**JSON** — stable shape:

```json
{
  "status": "ok",
  "command": "trace",
  "subcommand": "status",
  "checkout_id": "01900000-...",
  "database_path": "/.../agent-trace-{id}.db",
  "db_status": "ready",
  "stats": {
    "diff_traces": N,
    "messages": N,
    "parts": N,
    "session_models": N,
    "agent_traces": N,
    "post_commit_patch_intersections": N
  },
  "last_activity": "2026-06-27T22:39:03.926+00:00"
}
```

For `db_status: "skipped"`, `stats` and `last_activity` are omitted and a `skip_reason: "missing table: <name>"` field is added.

### `sce trace status --all` aggregation — `services::trace::status_all`

`aggregate_current_status_all()` resolves `<state_root>/sce/` and delegates to `aggregate_status_all_in(sce_dir)`, which walks `discover_agent_trace_dbs_in`, runs `collect_agent_trace_db_stats` on each `Readiness::Ready` DB, and accumulates `Totals` (sum of the six row counts plus the max of per-DB `last_activity`). `Readiness::Skipped` DBs are excluded from totals but counted in the discovery summary and surfaced as breakdown rows with a `missing_table` reason. Returns `StatusAllReport { discovery: DiscoverySummary { discovered, ready, skipped }, totals: Totals, databases: Vec<DatabaseRow> }`.

### `sce trace status --all` rendering — `services::trace::render_status_all`

**Text** — heading `SCE trace status (all)` followed by three blocks: discovery summary line, `Totals` block (same six counts plus `Last activity`), and an optional `By database` block omitted when no databases are discovered:

```
SCE trace status (all)
Databases: 3 discovered, 2 ready, 1 skipped

Totals
Diff traces: N
Messages: N
Parts: N
Session models: N
Agent traces: N
Post-commit intersections: N
Last activity: 2026-06-28T...

By database
Alias          Status                                              Diffs  Messages  Parts  Models  Traces  Intersections
agent_trace_0  ready                                               3      2         4      0       0       0
agent_trace_1  ready                                               1      1         1      0       0       0
agent_trace_2  skipped: missing 'post_commit_patch_intersections'  -      -         -      -       -       -
```

`Last activity` renders `never` when no DB carries activity. Skipped rows fill count cells with `-`.

**JSON** — stable shape:

```json
{
  "status": "ok",
  "command": "trace",
  "subcommand": "status.all",
  "discovery": { "discovered": N, "ready": N, "skipped": N },
  "totals": {
    "diff_traces": N,
    "messages": N,
    "parts": N,
    "session_models": N,
    "agent_traces": N,
    "post_commit_patch_intersections": N,
    "last_activity": "2026-06-28T..."
  },
  "databases": [
    {
      "alias": "agent_trace_0",
      "checkout_id": "aaaa",
      "path": "/.../agent-trace-aaaa.db",
      "status": "ready",
      "diff_traces": N,
      "messages": N,
      "parts": N,
      "session_models": N,
      "agent_traces": N,
      "post_commit_patch_intersections": N,
      "last_activity": "2026-06-28T..."
    },
    {
      "alias": "agent_trace_2",
      "checkout_id": "cccc",
      "path": "/.../agent-trace-cccc.db",
      "status": "skipped",
      "skip_reason": "missing table: post_commit_patch_intersections"
    }
  ]
}
```

`totals.last_activity` is `null` when no ready DB has activity. Skipped DB entries omit per-database counts and `last_activity` and add `skip_reason`.

## Related context

- [cli-command-surface.md](cli-command-surface.md) — full CLI command surface and dispatch contract.
- [checkout-identity.md](checkout-identity.md) — per-checkout Agent Trace DB path resolution and `sce trace db list` discovery surface.
- [default-path-catalog.md](default-path-catalog.md) — `<state_root>/sce/agent-trace-*.db` path ownership.
- [styling-service.md](styling-service.md) — heading helper used by the text renderer.
- [../sce/agent-trace-db.md](../sce/agent-trace-db.md) — Agent Trace DB schema and migration ownership.
