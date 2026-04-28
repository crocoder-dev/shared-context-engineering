-- Migration: 001_create_agent_traces
-- Description: Creates normalized local Agent Trace persistence tables
-- Created: 2026-04-24

CREATE TABLE IF NOT EXISTS agent_traces (
    trace_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    trace_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS agent_trace_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id TEXT NOT NULL,
    file_index INTEGER NOT NULL,
    path TEXT NOT NULL,
    FOREIGN KEY (trace_id) REFERENCES agent_traces(trace_id) ON DELETE CASCADE,
    UNIQUE (trace_id, file_index)
);

CREATE TABLE IF NOT EXISTS agent_trace_conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    conversation_index INTEGER NOT NULL,
    contributor_type TEXT NOT NULL,
    FOREIGN KEY (file_id) REFERENCES agent_trace_files(id) ON DELETE CASCADE,
    UNIQUE (file_id, conversation_index)
);

CREATE TABLE IF NOT EXISTS agent_trace_ranges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL,
    range_index INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES agent_trace_conversations(id) ON DELETE CASCADE,
    UNIQUE (conversation_id, range_index)
);

CREATE INDEX IF NOT EXISTS idx_agent_traces_timestamp
    ON agent_traces (timestamp);

CREATE INDEX IF NOT EXISTS idx_agent_trace_files_path
    ON agent_trace_files (path);
