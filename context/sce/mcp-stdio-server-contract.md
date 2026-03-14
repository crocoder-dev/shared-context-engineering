# MCP Stdio Server Contract

## Scope

- This file documents the implemented `sce-mcp-smart-cache-engine` T05 MCP stdio server in `cli/src/services/mcp.rs`.
- The MCP server exposes Smart Cache Engine tools over stdio using the Model Context Protocol.

## Implemented contract

- `sce mcp` starts a stdio MCP server that implements the MCP protocol using the `rmcp` crate.
- The server registers four tools: `read_file`, `read_files`, `cache_status`, and `cache_clear`.
- Each tool handler resolves the repository root from the current working directory, then routes to the corresponding cache service function.

## Tool definitions

### read_file

- **Description**: Read a file from the repository with smart caching. Returns full content on first read, unchanged markers for unchanged files, or unified diffs for changed files. Supports partial reads with offset/limit.
- **Parameters**:
  - `path` (string, required): Repository-relative path to the file to read.
  - `session_id` (string, required): Unique session identifier for cache tracking.
  - `offset` (integer, optional): 1-based line offset for partial reads.
  - `limit` (integer, optional): Line limit for partial reads.
  - `force` (boolean, optional): Force full content read, bypassing cache compression.
- **Response**: JSON object with `response_type`, `content`/`unified_diff`/`marker`, `content_hash`, `line_count`, `byte_count`, `estimated_tokens`, `saved_tokens`, `session_saved_tokens`, `cumulative_saved_tokens`, `cache_hit`, `first_read_in_session`, and `force` fields.

### read_files

- **Description**: Read multiple files from the repository in a single batch request with smart caching. Returns per-file sections with unchanged markers or diffs, plus session token savings summary.
- **Parameters**:
  - `paths` (array of strings, required): List of repository-relative file paths to read.
  - `session_id` (string, required): Unique session identifier for cache tracking.
  - `force` (boolean, optional): Force full content reads, bypassing cache compression.
- **Response**: JSON object with `repository_root`, `outputs` (array of per-file responses), `rendered_response` (text summary), `session_saved_tokens`, and `cumulative_saved_tokens`.

### cache_status

- **Description**: Report cache status for the current repository: database path, tracked file count, session token savings, and cumulative token savings.
- **Parameters**:
  - `session_id` (string, required): Unique session identifier for session-specific metrics.
- **Response**: JSON object with `repository_root`, `repository_db_path`, `tracked_file_count`, `session_saved_tokens`, `cumulative_saved_tokens`, and `last_cleared_at`.

### cache_clear

- **Description**: Clear cached state for the current repository. Resets file versions, session reads, and token savings while preserving the cache database scaffold.
- **Parameters**: None.
- **Response**: JSON object with `repository_root`, `repository_db_path`, `cleared_file_versions`, `cleared_session_reads`, `tracked_file_count`, `session_saved_tokens`, `cumulative_saved_tokens`, and `last_cleared_at`.

## Server implementation

- The MCP server uses `rmcp::handler::server::ServerHandler` trait with `#[tool_router]` macro for tool registration.
- Tool parameter structs derive `Serialize`, `Deserialize`, and `JsonSchema` for automatic schema generation.
- Error handling maps service errors to `rmcp::ErrorData` with `internal_error` and `invalid_params` variants.
- The server runs on a tokio current-thread runtime created by `run_mcp_server_blocking()`.

## Dependencies

- `rmcp` crate version 1.x with `server` and `transport-io` features enabled.
- `schemars` crate version 1.x for JSON schema generation.
- `tokio` with `rt` and `io-util` features for async runtime.

## Current limitations

- Only stdio transport is supported; no HTTP or WebSocket transport.
- No editor-specific configuration installers; clients must configure MCP server connection manually.
- Session management is client-driven; the server does not persist sessions across restarts.