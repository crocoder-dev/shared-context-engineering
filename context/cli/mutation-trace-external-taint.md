# Mutation-cursor external-taint boundary (`runtime::external_taint`)

The worktree-local durability boundary for the mutation cursor: a filesystem
marker that a later hook invocation reads as the external signal that the
previous invocation never proved a trustworthy durable completion. This is the
concrete runtime refinement of the abstract `ProtocolState.external_taint` /
`databaseFailure` / `recover` semantics in
[`mutation-trace-protocol.md`](mutation-trace-protocol.md) — it changes no
protocol semantics, adds no database state, and needs no migration.

Built by the `mutation-cursor-external-taint` plan
(`context/plans/mutation-cursor-external-taint.md`), which extends the
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md)
work.

## `ExternalTaintMarker` primitive

`cli/src/services/mutation_trace/runtime/external_taint.rs` —
`ExternalTaintMarker::new(git_dir: &Path)` builds a handle rooted at
`<git-dir>/sce/mutation-cursor-tainted`, the worktree-specific Git directory as
resolved by `checkout::resolve_git_dir`. Two linked worktrees resolve to two
different Git directories and therefore two independent markers.

The marker file is empty; **its existence is its entire state**:

- `exists() -> Result<bool>` — `symlink_metadata`; a `NotFound` is `Ok(false)`,
  any other inspection failure is `Err`.
- `persist() -> Result<()>` — `create_dir_all` the `sce/` directory,
  `create`-open the marker without truncation (`write(true).create(true).truncate(false)`,
  since the contents carry no meaning), `sync_data()` it, then a best-effort
  `#[cfg(unix)]` parent-directory `sync_all` whose error is not propagated.
  Idempotent: an existing marker is re-opened and re-synced, never rewritten.
- `clear() -> Result<()>` — `remove_file`, treating `NotFound` as success, then
  the same best-effort parent-directory sync. Idempotent.

Durability mirrors `checkout::persist_checkout_id_inner`: it protects against
process error, non-graceful process exit, `SIGKILL`, and normal runtime
restart — not host power loss or a filesystem-level crash (the parent-directory
sync is best-effort). The marker is never removed via `Drop`; only an explicit
`clear()` removes it. It is never authoritative for normal cursor state.

The module carries `#![allow(dead_code)]` (the `services/capabilities.rs`
precedent): nothing arms or reads the marker yet. Wiring it into a reshaped
`coordinate()` entrypoint — armed write-ahead after the `WorktreeLock` is held
and before Agent Trace DB acquisition, cleared only on a successful
`CoordinateOutcome`, and promoted to protocol external taint only when inherited
by a later invocation — is later work in the same plan.

Inline `#[cfg(test)] mod tests` follows the unique-`std::env::temp_dir()`-path
precedent (see [`../patterns.md`](../patterns.md)): marker path is worktree
scoped, the marker survives reconstruction of the handle until an explicit
`clear()`, and `persist`/`clear` are idempotent.

## On-disk layout addition

```text
<worktree-git-dir>/sce/
└── mutation-cursor-tainted    (runtime::external_taint, empty; existence = armed)
```

See also: [`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`checkout-identity.md`](checkout-identity.md).
