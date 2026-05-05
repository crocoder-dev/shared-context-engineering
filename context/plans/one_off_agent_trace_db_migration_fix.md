# One-off: Agent Trace DB migration fix

## Approved task

- **Status:** done
- **Approved by:** user selected one-off execution after plan review found no unchecked plan tasks
- **Completed:** 2026-05-05

## Goal

Ensure `sce setup` / `AgentTraceDb::new()` applies Agent Trace DB migrations added after `~/.local/state/sce/agent-trace.db` already exists.

## Boundaries

- In scope: shared Turso migration runner, Agent Trace DB migration regression coverage, current-state context sync.
- Out of scope: new Agent Trace schema, payload-shape changes, hosted/cloud sync, rollback tooling, setup UX redesign.

## Files changed

- `cli/src/services/db/mod.rs`
- `cli/src/services/agent_trace_db/mod.rs`
- `context/sce/shared-turso-db.md`
- `context/sce/agent-trace-db.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`

## Evidence

- `nix develop -c sh -c 'cd cli && cargo check'` passed.
- `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed after formatting.
- `nix flake check` passed.

## Notes

`TursoDb<M>::run_migrations()` now creates `__sce_migrations`, skips only recorded migration IDs, executes unrecorded migration SQL, and records each ID after success. Existing metadata-less DBs are brought forward by re-applying the current idempotent migration set. Regression coverage creates a legacy Agent Trace DB with only migration `001`, then reopens it with the full migration list and verifies migrations `002` and `003` are applied and recorded.
