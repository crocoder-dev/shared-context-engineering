# MCP Smart Cache Batch Status Clear

## Scope

- This file documents the implemented `sce-mcp-smart-cache-engine` T04 service-layer cache operations in `cli/src/services/mcp.rs`.

## Implemented contract

- `read_cached_batch_files` requires a non-empty `session_id`, at least one repository-relative path, and optional `force=true` bypass behavior.
- Batch reads bootstrap the same per-repository cache storage used by single-file reads, then execute each file read in request order through the same snapshot/session persistence path as `read_cached_single_file`.
- Batch output preserves request ordering in both the structured `outputs` list and the rendered text response.
- Rendered batch sections use deterministic `==> <relative_path> <==` headers, then render the same underlying whole-file, diff, or unchanged body returned by the single-file cache service.
- The rendered batch response ends with `Session token savings: <n> estimated tokens saved.` using the repository/session aggregate after the final file read in the batch.
- `read_smart_cache_status` reports the canonical repository root, cache DB path, tracked-file count, current-session token savings, cumulative token savings, and optional `last_cleared_at` timestamp.
- `clear_smart_cache` clears only the current repository's cached file/version rows, zeroes repository aggregate counters in `cache_stats`, and records a deterministic UTC `last_cleared_at` timestamp.
- Cache clear preserves the repository cache DB/config scaffold so later reads can reuse the same storage path without a fresh config-map entry.

## Persisted state semantics

- Batch reads do not introduce a separate cache model; they reuse `file_versions`, `session_reads`, and `cache_stats` in the same per-repository `cache.db`.
- Status reads refresh `cache_stats` from persisted rows before reporting values so tracked-file and token-savings totals stay derived from current DB truth.
- Cache clear deletes only rows scoped to the active repository root and leaves other repository cache databases untouched.

## Current limitations

- DB-backed coverage for batch/status/clear behavior is intentionally deferred from unit tests; follow-up integration coverage can exercise these flows end-to-end.
