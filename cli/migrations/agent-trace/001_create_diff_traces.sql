-- Migration: 001_create_diff_traces
-- Description: Creates the diff_traces table for storing accepted diff-trace payloads
-- Created: 2026-04-29

CREATE TABLE IF NOT EXISTS diff_traces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    time_ms INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    patch TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
