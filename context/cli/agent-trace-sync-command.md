# Agent Trace sync architecture

`sce trace sync` is the composition step that synchronizes a repository's local Agent Trace capture database with the control-plane Agent Trace ingestion API. It composes already-shipped infrastructure — repository/source identity, the read-only export readers, and the existing WorkOS auth/token-storage stack — into one command; it does not redesign the local database, source identity, export readers, or control-plane storage model.

## User flow

```
sce auth login          # obtain and store WorkOS credentials
cd <repository>          # any directory inside the target Git repository
sce trace sync            # synchronize this repository's Agent Trace DB
```

`sce trace sync --format json` produces the same synchronization with machine-readable output; see [trace-command.md](trace-command.md) for the exact rendering contracts.

## Composed data flow

```mermaid
flowchart LR
    A[hooks / plugins] --> B[repository Agent Trace DB]
    B --> C[AgentTraceExportReader]
    C --> D[sce trace sync]
    D -- "HTTPS + WorkOS Bearer" --> E[control plane]
```

- **hooks/plugins** write local capture rows (`messages`, `parts`, `diff_traces`, `agent_traces`) into the current repository's `RepositoryAgentTraceDb` during normal Git/editor activity — this is unchanged by sync.
- **`AgentTraceExportReader`** (PR #198) is the read-only local export boundary sync uses to read rows after a cursor; sync never queries the repository DB directly.
- **`sce trace sync`** resolves repository storage through the same `agent_trace_storage` path `sce trace status` uses (not the hook-runtime resolver), builds an `AuthenticatedControlPlaneClient` from stored WorkOS credentials and the resolved `control_plane_base_url`, and drives one authoritative `/state` call plus one bounded per-stream reconciliation loop per stream.
- **control plane** is the sole source of cursor truth: every invocation starts from `POST /agent-trace/ingestion/state`, uploads via `POST /agent-trace/ingestion/batch`, and advances a stream's cursor only from a validated batch response (`accepted == rows.len()` and `cursor == rows.last().sourceRowId`), never by inferring `cursor + rows.len()`.

The four streams (`messages`, `parts`, `diff_traces`, `agent_traces`) are independent and synchronized in that fixed order within one invocation.

## No-local-persistence invariants

Sync creates no local sync state anywhere on disk:

- No local sync cursor or cursor file.
- No `agent-trace-sync.db` or equivalent local database/table.
- No Turso Sync and no direct Turso credentials in SCE.
- No `BridgeLock` or local data-warehouse (DWH).

Because every invocation starts from the control plane's authoritative `/state` cursors instead of local progress, restarts, conflicts, and ambiguous network failures are all recoverable without any client-side persisted state, and running `sce trace sync` twice in a row is naturally incremental — the second run's `/state` reflects the first run's uploads and only unsynced rows are re-read.

## Recovery semantics

- **`401` (unexpected):** the control-plane client refreshes the WorkOS token exactly once, saves it, and retries the request exactly once. A second `401` fails the command with `sce auth login` guidance; there is no further retry.
- **`409` (cursor conflict):** the per-stream sync engine reconciles by refetching `/state`, replacing only the affected stream's cursor, and resuming from local rows after the refreshed cursor — already-accepted rows are never resent.
- **Ambiguous batch failure (`5xx`, transport failure, or an undecodable `2xx` body):** the engine reconciles via `/state` before any resend. If the refreshed cursor advanced (the batch was actually committed), sync continues from it without resending. If the cursor is unchanged (the batch was not committed), sync may resend once from the authoritative cursor.
- **Reconciliation bound:** both the `409` and ambiguous-failure reconciliation paths share one bounded attempt counter per stream; exhausting it fails that stream with a "did not converge" error instead of looping unboundedly.
- **Invalid response:** a syntactically successful (`2xx`) but semantically inconsistent batch response (`accepted`/`cursor` not matching the sent rows, or an undecodable body) yields `ControlPlaneError::InvalidResponse` and is treated as ambiguous — it still reconciles via `/state` above, rather than failing the command outright.
- **`403` (ownership rejection):** fails the command immediately with a clear message. It never generates a new `source_instance_id`, mutates local repository metadata, or attempts ownership transfer — a `403` is treated as terminal, not reconciled.
- **Terminal protocol/API mismatch (`404`/`405`/`415`/`422` and other unrecognized `4xx` statuses during `/batch`):** classified as `ControlPlaneError::Protocol` and treated as terminal exactly like `400`/`403` — the stream fails immediately with no `/state` refetch and no resend, since a route/format mismatch is not something reconciling cursors can resolve.
- **Sanitized error messages:** every non-2xx control-plane error surfaces at most a narrow, length-bounded `message`/`error` string extracted from the response body (or a generic per-status fallback when the body is malformed, HTML, oversized, or missing that field) — the raw server response body is never included in a command-visible error.

## Related context

- [trace-command.md](trace-command.md) — the full `sce trace` command group, including `sync` request/response rendering (text and JSON shapes).
- [agent-trace-storage.md](agent-trace-storage.md) — the repository-scoped storage resolver sync reuses from `sce trace status`.
- [agent-trace-export-readers.md](../sce/agent-trace-export-readers.md) — the read-only local export boundary sync reads through.
- [auth-db.md](../sce/auth-db.md) — encrypted WorkOS credential storage sync authenticates through.
