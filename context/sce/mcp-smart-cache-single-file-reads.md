# MCP Smart Cache Single-File Reads

## Scope

- This file documents the implemented `sce-mcp-smart-cache-engine` T02-T03 single-file cache read path in `cli/src/services/mcp.rs`.

## Implemented contract

- `read_cached_single_file` bootstraps the per-repository cache store before reading, so repository config and `cache.db` schema are present before session tracking updates.
- Read requests require a non-empty `session_id`, a repository-relative file path, optional `offset` / `limit` line slicing, and optional `force` bypass behavior.
- Repository-relative paths are canonicalized against the active repository root and rejected when they are absolute, outside the repository, or not readable files.
- First reads in a session return full file content for whole-file requests and return the requested slice for partial requests, then persist the current file fingerprint into `file_versions` plus the session hash and last-read content into `session_reads`.
- Repeated unchanged whole-file reads return the deterministic unchanged marker `File unchanged since the last read in this session; cached content omitted.` when the stored session hash still matches the current file hash.
- Repeated changed whole-file reads return a deterministic unified diff (`--- a/...`, `+++ b/...`, stable hunk headers, and added/removed lines) plus tracked changed line numbers for the current file version.
- Partial reads respect 1-based `offset` / `limit`; if a reread changed only lines outside the requested slice, the response returns `Requested line range unchanged since the last read in this session; cached content omitted.` instead of the slice content.
- Partial rereads whose changed lines overlap the requested slice return the current requested content slice rather than a diff.
- `force=true` bypasses unchanged/diff compression and returns direct content for the requested whole-file or partial read while updating the session row with `was_forced = 1` without adding token savings.

## Persisted read metadata

- File fingerprints use SHA-256 content hashing plus stored line count and byte count.
- Session rows now also persist the last-read file content so whole-file diffs and partial-range overlap checks can compare the current snapshot against the prior session snapshot.
- Estimated token size uses deterministic byte-based accounting: `max(byte_count, 1) / 4`, rounded up.
- Session savings accumulate per `session_id + repository_root + relative_path` row in `session_reads.token_savings`.
- Whole-file diff rereads save `full_file_estimated_tokens - unified_diff_estimated_tokens` with floor-at-zero behavior; unchanged partial-range markers save the estimated tokens for the omitted requested slice.
- Repository aggregates in `cache_stats` are refreshed from persisted rows after each read: tracked file count, current-session savings for the active session, and cumulative savings across all sessions for that repository.

## Current limitations

- The current diff path is deterministic and line-based; multi-file aggregation, cache status reporting, and cache-clear behavior are documented separately in `context/sce/mcp-smart-cache-batch-status-clear.md`.
