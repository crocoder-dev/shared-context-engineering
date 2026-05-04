CREATE TABLE IF NOT EXISTS post_commit_patch_intersections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_id TEXT NOT NULL,
    post_commit_time_ms INTEGER NOT NULL,
    recent_window_cutoff_ms INTEGER NOT NULL,
    recent_window_end_ms INTEGER NOT NULL,
    loaded_diff_trace_count INTEGER NOT NULL CHECK (loaded_diff_trace_count >= 0),
    skipped_diff_trace_count INTEGER NOT NULL CHECK (skipped_diff_trace_count >= 0),
    intersection_patch TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
