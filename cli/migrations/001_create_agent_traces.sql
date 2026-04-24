-- Migration: 001_create_agent_traces
-- Description: Creates the agent_traces table for storing agent trace JSON blobs
-- Created: 2026-04-24

CREATE TABLE IF NOT EXISTS agent_traces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
