BEGIN IMMEDIATE;

DROP TABLE IF EXISTS mutation_trace_worktrees_v006;

CREATE TABLE mutation_trace_worktrees_v006 (
    worktree_id TEXT PRIMARY KEY,
    cursor_tree TEXT NOT NULL,
    revision BLOB NOT NULL
        CHECK (typeof(revision) = 'blob' AND length(revision) = 8),
    tainted INTEGER NOT NULL CHECK (tainted IN (0, 1)),
    failure_kind TEXT NOT NULL CHECK (failure_kind IN ('healthy', 'snapshot_failure')),
    needs_rebaseline INTEGER NOT NULL CHECK (needs_rebaseline IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)
);

INSERT INTO mutation_trace_worktrees_v006 (
    worktree_id,
    cursor_tree,
    revision,
    tainted,
    failure_kind,
    needs_rebaseline,
    created_at,
    updated_at
)
SELECT
    worktree_id,
    cursor_tree,
    revision,
    CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END,
    failure_kind,
    needs_rebaseline,
    created_at,
    updated_at
FROM mutation_trace_worktrees;

DROP TABLE mutation_trace_worktrees;

ALTER TABLE mutation_trace_worktrees_v006 RENAME TO mutation_trace_worktrees;

DROP TABLE IF EXISTS mutation_trace_events_v006;

CREATE TABLE mutation_trace_events_v006 (
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
    ),
    CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)
);

INSERT INTO mutation_trace_events_v006 (
    worktree_id,
    revision,
    before_tree,
    after_tree,
    tainted,
    failure_kind,
    attribution_kind,
    attribution_scope_id,
    boundary_kind,
    boundary_scope_id,
    boundary_event_id,
    created_at
)
SELECT
    worktree_id,
    revision,
    before_tree,
    after_tree,
    CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END,
    failure_kind,
    attribution_kind,
    attribution_scope_id,
    boundary_kind,
    boundary_scope_id,
    boundary_event_id,
    created_at
FROM mutation_trace_events;

DROP TABLE mutation_trace_events;

ALTER TABLE mutation_trace_events_v006 RENAME TO mutation_trace_events;

COMMIT;
