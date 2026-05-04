CREATE TABLE IF NOT EXISTS patch_intersections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_sha TEXT NOT NULL,
    source_diff_trace_ids TEXT NOT NULL,
    intersection_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
