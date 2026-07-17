# 2026-07-17 — Retire the legacy checkout-scoped Agent Trace DB surface

## Status

Accepted.

## Context

The `repository-scoped-agent-trace-db` migration moved active Agent Trace
persistence to one database per logical Git repository at
`<state_root>/sce/repos/<repository-id>/agent-trace.db`, resolved through
`agent_trace_storage` and `RepositoryAgentTraceDb`. That migration intentionally
retained a "legacy surface" for a transition period:

- a dead per-checkout DB opener (`resolve_or_create_agent_trace_db_for_checkout`)
  and its `agent_trace_db_path_for_checkout()` path helper,
- the legacy `AgentTraceDb` / `AgentTraceDbSpec` adapter and its 15-file
  incremental migration chain under `cli/migrations/agent-trace/`,
- the global sentinel path helper `agent_trace_db_path()` and a lifecycle
  fallback to it outside repository context,
- the `sce trace --legacy` CLI surface for inspecting old checkout-scoped
  `agent-trace-<checkout-id>.db` files.

The prior policy was: retain legacy checkout DBs, keep them inspectable via
`--legacy`, and leave them byte-for-byte untouched.

## Decision

Fully retire the legacy checkout-scoped Agent Trace DB surface and reverse the
retained-legacy policy. After the `retire-legacy-agent-trace-db` plan:

- `RepositoryAgentTraceDb` is the sole Agent Trace DB adapter.
- `sce trace` has no `--legacy` flag; only repository-scoped DBs are
  discoverable, listable, statusable, and shell-inspectable.
- The legacy `AgentTraceDb` type, `AgentTraceDbSpec`, `agent_trace_db_path()`,
  `agent_trace_db_path_for_checkout()`, `AGENT_TRACE_MIGRATIONS`, and all 15
  `cli/migrations/agent-trace/*.sql` files are deleted.
- Outside a Git repository, Agent Trace diagnostics (`sce doctor` / `sce setup`)
  no longer probe a global `agent-trace.db` sentinel. They report an actionable
  "requires a Git repository" diagnostic instead of falling back to a sentinel
  path.

Existing on-disk legacy checkout DB files (`agent-trace-<checkout-id>.db`) and
the legacy global `agent-trace.db` are still never migrated, imported, copied,
renamed, archived, deleted, or backfilled by any SCE code path. They simply
become uninspectable through the CLI.

Checkout identity infrastructure (`<git-dir>/sce/checkout-id`) is out of scope
for this retirement; it remains as repository-scoped diagnostic metadata and is
not stored on Agent Trace rows.

## Consequences

- One adapter, one schema file, one migration directory
  (`cli/migrations/agent-trace-repository/`); no dual-adapter maintenance and no
  stale `#[allow(dead_code)]` masking genuinely-used repository code.
- Operators lose CLI visibility into pre-migration checkout-scoped databases.
  Because SCE never touched those files, the data is still on disk and could be
  inspected with an external SQLite tool if ever needed; SCE itself no longer
  offers a path to it.
- The no-repository doctor/setup case now fails loudly with guidance instead of
  silently pointing at a global sentinel that is never a write target.

See `context/sce/agent-trace-db.md`, `context/cli/trace-command.md`, and
`context/cli/agent-trace-storage.md` for the resulting current-state contracts.
