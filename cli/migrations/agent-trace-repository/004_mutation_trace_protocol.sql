-- Durable persistence for the verified mutation-cursor protocol
-- (`cli/src/services/mutation_trace/`).
--
-- This migration is additive: it introduces five new tables and leaves every
-- table from 001/002 untouched. `revision` is stored as an 8-byte
-- big-endian BLOB on every column that carries one (never a SQLite INTEGER),
-- enforced by `CHECK (typeof(revision) = 'blob' AND length(revision) = 8)`,
-- so a TEXT value of matching length is still rejected. Enum-shaped columns
-- use TEXT with an explicit CHECK allow-list, matching the `role`/
-- `payload_type` convention already used in 001. `AttemptState` (transient)
-- and `external_taint` (not DB-authoritative) are deliberately not
-- represented by any table here.

CREATE TABLE IF NOT EXISTS mutation_trace_worktrees (
    worktree_id TEXT PRIMARY KEY,
    cursor_tree TEXT NOT NULL,
    revision BLOB NOT NULL
        CHECK (typeof(revision) = 'blob' AND length(revision) = 8),
    tainted INTEGER NOT NULL CHECK (tainted IN (0, 1)),
    failure_kind TEXT NOT NULL CHECK (failure_kind IN ('healthy', 'snapshot_failure')),
    needs_rebaseline INTEGER NOT NULL CHECK (needs_rebaseline IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS mutation_trace_scopes (
    scope_id TEXT PRIMARY KEY,
    worktree_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('claude_code', 'codex', 'opencode', 'pi')),
    status TEXT NOT NULL CHECK (status IN ('never_seen', 'active', 'closed', 'abandoned')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_mutation_trace_scopes_worktree
ON mutation_trace_scopes (worktree_id);

CREATE INDEX IF NOT EXISTS idx_mutation_trace_scopes_worktree_status
ON mutation_trace_scopes (worktree_id, status);

-- `worktree_id` is deliberately not duplicated here: a processed event's
-- identity is exactly `(scope_id, event_id)` (the domain `EventKey`), and
-- `scope_id`'s worktree is already a permanent fact owned by
-- `mutation_trace_scopes` (`ScopeId -> WorktreeId`, never reassigned). A
-- second `worktree_id` column would create two sources of truth for the same
-- fact and could disagree with `mutation_trace_scopes` for the same
-- `scope_id`; the schema does not represent that inconsistency.
CREATE TABLE IF NOT EXISTS mutation_trace_processed_events (
    scope_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (scope_id, event_id)
);

CREATE TABLE IF NOT EXISTS mutation_trace_events (
    worktree_id TEXT NOT NULL,
    revision BLOB NOT NULL
        CHECK (typeof(revision) = 'blob' AND length(revision) = 8),
    before_tree TEXT NOT NULL,
    after_tree TEXT NOT NULL,
    tainted INTEGER NOT NULL CHECK (tainted IN (0, 1)),
    failure_kind TEXT NOT NULL CHECK (failure_kind IN ('healthy', 'snapshot_failure')),
    attribution_kind TEXT NOT NULL
        CHECK (attribution_kind IN ('ineligible_unscoped', 'ai_exclusive', 'ai_contended')),
    attribution_scope_id TEXT,
    boundary_kind TEXT NOT NULL CHECK (boundary_kind IN ('start', 'advance', 'close', 'flush')),
    boundary_scope_id TEXT,
    boundary_event_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (worktree_id, revision),
    CHECK (
        (attribution_kind = 'ai_exclusive' AND attribution_scope_id IS NOT NULL)
        OR (attribution_kind != 'ai_exclusive' AND attribution_scope_id IS NULL)
    ),
    CHECK (
        (boundary_kind IN ('start', 'advance', 'close')
            AND boundary_scope_id IS NOT NULL AND boundary_event_id IS NOT NULL)
        OR (boundary_kind = 'flush'
            AND boundary_scope_id IS NULL AND boundary_event_id IS NULL)
    )
);

CREATE TABLE IF NOT EXISTS mutation_trace_event_active_scopes (
    worktree_id TEXT NOT NULL,
    revision BLOB NOT NULL
        CHECK (typeof(revision) = 'blob' AND length(revision) = 8),
    scope_id TEXT NOT NULL,
    PRIMARY KEY (worktree_id, revision, scope_id)
);
