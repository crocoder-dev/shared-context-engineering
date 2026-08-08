-- Agent Trace DWH baseline schema.
--
-- This is the first versioned destination contract for the append-oriented
-- Agent Trace data warehouse. It is deliberately separate from the
-- repository-scoped `agent-trace.db` source schema
-- (cli/migrations/agent-trace-repository/): the DWH is a distinct database
-- boundary that a future ETL consumer ingests into, not the live capture
-- path.
--
-- Every fact table carries `repository_id` and `source_instance_id` as plain
-- TEXT lineage columns instead of foreign keys, so independently created
-- source databases for the same repository, and out-of-order or partial
-- batch ingestion across fact tables, are never blocked by referential
-- constraints. A future ETL consumer keys row provenance on the tuple
-- (repository_id, source_instance_id, source_table, source_row_id).
--
-- Two kinds of logical identity are distinguished by their uniqueness scope:
--   * Deterministic source identities (message session_id/message_id, an
--     Agent Trace's agent_trace_id) are expected to be reproduced identically
--     if the same logical event is re-ingested from an independently created
--     source database for the same repository, so their uniqueness excludes
--     source_instance_id: re-ingestion stays idempotent across repositories
--     and independently created source databases.
--   * Raw local autoincrement source row IDs (a source `parts.id`, a source
--     `diff_traces.id`) are NOT stable across independently created source
--     databases, so their uniqueness is scoped by
--     (repository_id, source_instance_id, <local id>): the same local integer
--     ID is expected to coexist across different source instances and
--     repositories.

CREATE TABLE IF NOT EXISTS repositories (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_repositories_repository_id
ON repositories (repository_id);

CREATE TABLE IF NOT EXISTS source_instances (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_source_instances_repository_source
ON source_instances (repository_id, source_instance_id);

-- Extraction watermarks are independently keyed per repository, per source
-- instance, and per extensible source-table text (not a database enum), so
-- ETL progress for one repository/source/table triple never affects another.
CREATE TABLE IF NOT EXISTS etl_watermarks (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_table TEXT NOT NULL,
    last_extracted_source_row_id INTEGER,
    last_extracted_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_etl_watermarks_repository_source_table
ON etl_watermarks (repository_id, source_instance_id, source_table);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0),
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Logical message identity excludes source_instance_id: re-ingesting the same
-- session/message from an independently created source database for the same
-- repository must not create a duplicate row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_messages_logical_identity
ON messages (repository_id, session_id, message_id);

CREATE TABLE IF NOT EXISTS message_parts (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    source_part_id INTEGER NOT NULL,
    part_type TEXT NOT NULL,
    text TEXT NOT NULL,
    text_sha256 TEXT NOT NULL,
    generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0),
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- source_part_id is a raw local `parts.id` value from the repository source
-- schema; it is only unique within one (repository, source instance), so the
-- same local integer coexists across different source instances/repositories.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_message_parts_source_identity
ON message_parts (repository_id, source_instance_id, source_part_id);

-- Deterministic message-part reconstruction ordering: repository, session,
-- message, source timestamp, then source_part_id as the tie-break for parts
-- sharing the same generated_at_unix_ms value.
CREATE INDEX IF NOT EXISTS idx_dwh_message_parts_order
ON message_parts (repository_id, session_id, message_id, generated_at_unix_ms, source_part_id);

CREATE TABLE IF NOT EXISTS agent_traces (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    agent_trace_id TEXT NOT NULL,
    commit_id TEXT NOT NULL,
    commit_time_ms INTEGER NOT NULL CHECK (commit_time_ms >= 0),
    trace_json TEXT NOT NULL,
    trace_json_sha256 TEXT NOT NULL,
    url TEXT NOT NULL,
    remote_url TEXT,
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Logical Agent Trace identity excludes source_instance_id: agent_trace_id is
-- expected to be deterministically derived, so re-deriving the same trace from
-- an independently created source database for the same repository must not
-- create a duplicate row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_agent_traces_logical_identity
ON agent_traces (repository_id, agent_trace_id);

CREATE TABLE IF NOT EXISTS code_changes (
    id INTEGER PRIMARY KEY,
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_diff_trace_id INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    time_ms INTEGER NOT NULL CHECK (time_ms >= 0),
    model_id TEXT,
    tool_name TEXT NOT NULL,
    tool_version TEXT,
    payload_type TEXT NOT NULL,
    files_changed INTEGER NOT NULL CHECK (files_changed >= 0),
    lines_added INTEGER NOT NULL CHECK (lines_added >= 0),
    lines_removed INTEGER NOT NULL CHECK (lines_removed >= 0),
    patch_sha256 TEXT NOT NULL,
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- source_diff_trace_id is a raw local `diff_traces.id` value from the
-- repository source schema; scoped the same way as source_part_id above so
-- the same local integer coexists across different source instances/repositories.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dwh_code_changes_source_identity
ON code_changes (repository_id, source_instance_id, source_diff_trace_id);
