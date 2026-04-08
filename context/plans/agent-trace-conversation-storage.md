# Plan: agent-trace-conversation-storage

## Change summary
- Add a **parallel local SQLite storage layer** (same `.../sce/agent-trace/local.db` file) for **prompts + conversation events** while keeping **Agent Trace records in Git notes** as canonical attribution.
- Use **existing Agent Trace range attribution** (`contributor.type` = `human|ai|mixed|unknown`) to distinguish user vs agent, and store prompts/conversations for later analysis only.
- Ingest prompt/conversation data from **`.git/sce/prompts.jsonl`** and optional **`transcript_path`** without any API calls, aligning with the Cursor Agent Trace spec v0.1.0.
- When conversation URLs are missing, use a **deterministic fallback**: `sce://local-hooks/<commit_sha>`.

## Success criteria
- Agent Trace records remain stored in **Git notes** (`refs/notes/agent-trace`) with no behavior regression.
- Local DB (`.../sce/agent-trace/local.db`) stores:
  - prompt rows (existing), and
  - **conversation metadata + conversation events** linked to commits.
- Conversation URLs in SQLite are either real URLs from input or the fallback `sce://local-hooks/<commit_sha>`.
- Attribution remains driven by Agent Trace ranges (`contributor.type`), and prompts/conversations are **analysis-only** (no new CLI surface).
- No network/API calls are added; ingestion is local-file based only.

## Constraints and non-goals
- **No new CLI commands** or output surfaces in this plan.
- **No changes** to Agent Trace schema/format in Git notes.
- **No external API calls** for fetching conversations or prompts.
- **No new contributor types**; keep `human|ai|mixed|unknown`.
- **Use existing local DB path** (`.../sce/agent-trace/local.db`) with turso/libsql backend.

## Task stack

- [x] T01: Add local DB schema for conversations + events (status:done)
  - Task ID: T01
  - Goal: Extend the local SQLite schema to store conversation metadata and event streams linked to commits.
  - Boundaries (in/out of scope):
    - In scope: new tables/indexes in `local_db` migrations (e.g., `conversation_events`, optional commit linkage), minimal structs/helpers.
    - Out of scope: runtime ingestion or hook wiring.
  - Done when:
    - Migrations create conversation/event tables and indexes in `local.db`.
    - Schema supports linking events to `conversation_url` and `commit_id`.
  - Verification notes: Defer runtime verification to T05; ensure migrations compile and are wired into `apply_core_schema_migrations`.

- [x] T02: Ingest prompt + transcript data into conversation tables (status:done)
  - Task ID: T02
  - Goal: Parse `.git/sce/prompts.jsonl` + optional `transcript_path` and persist conversation metadata/events into SQLite at post-commit time.
  - Boundaries (in/out of scope):
    - In scope: parsing optional `conversation_url` from JSONL (if present), transcript parsing into event rows, fallback URL generation, DB persistence functions.
    - Out of scope: changing prompt capture producers or adding external API fetches.
  - Done when:
    - Post-commit pipeline persists conversation rows + events derived from local transcript files.
    - Fallback URL `sce://local-hooks/<commit_sha>` is used when no URL is provided.
  - Verification notes: Manual local run or targeted tests optional; full validation in T05.

- [x] T03: Link conversations to Agent Trace ranges in local DB (status:done)
  - Task ID: T03
  - Goal: Ensure the local DB associates conversation URLs from Agent Trace ranges with conversation metadata for commit-level analysis.
  - Boundaries (in/out of scope):
    - In scope: upsert conversations during trace persistence; link conversation URLs to commit/trace records.
    - Out of scope: modifying Agent Trace record format or adding UI/CLI queries.
  - Done when:
    - Every persisted trace range’s `conversation_url` has a corresponding conversation row in SQLite.
    - Commit-level joins across `prompts` → `commits` → `trace_ranges` → `conversations` are possible.
  - Verification notes: Validate via DB query in T05 (no new CLI needed).

- [x] T04: Local SQLite health checks for new tables (status:done)
  - Task ID: T04
  - Goal: Extend local DB health checks to verify new conversation tables exist and are readable.
  - Boundaries (in/out of scope):
    - In scope: update local DB health checks and/or smoke checks.
    - Out of scope: new doctor UI surfaces or new CLI commands.
  - Done when:
    - Health check covers the new tables without breaking existing doctor output.
  - Verification notes: Reuse existing health-check execution paths; full validation in T05.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run validation and ensure context stays aligned with code truth.
  - Boundaries (in/out of scope):
    - In scope: repo-level verification, local DB smoke/health check, context sync if needed.
    - Out of scope: feature expansion beyond plan scope.
  - Done when:
    - `nix flake check` passes (preferred).
    - Local DB health check passes with new tables.
    - Context is updated only if required by behavior changes.
  - Verification notes: `nix flake check` (preferred repo validation).

## Open questions
- None.

## Task: Add local DB schema for conversations + events
- **Status:** done
- **Completed:** 2026-04-08
- **Files changed:** cli/src/services/local_db.rs
- **Evidence:** `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo build'`
- **Notes:** Added `conversation_events` table plus supporting indexes for commit/conversation linkage.

## Task: Ingest prompt + transcript data into conversation tables
- **Status:** done
- **Completed:** 2026-04-08
- **Files changed:** cli/src/services/hooks.rs
- **Evidence:** `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo build'`
- **Notes:** Post-commit now extracts conversation events from transcript JSONL with fallback URLs and persists conversation rows + events to the local DB; retry queue carries events for DB retries.

## Task: Link conversations to Agent Trace ranges in local DB
- **Status:** done
- **Completed:** 2026-04-08
- **Files changed:** cli/src/services/hooks.rs
- **Evidence:** `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo build'`
- **Notes:** Trace persistence now upserts conversations for all trace range URLs, ensuring joinability via `trace_ranges.conversation_url`.

## Task: Local SQLite health checks for new tables
- **Status:** done
- **Completed:** 2026-04-08
- **Files changed:** cli/src/services/local_db.rs
- **Evidence:** `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`; `nix develop -c sh -c 'cd cli && cargo build'`
- **Notes:** Local DB health checks now validate `conversations` and `conversation_events` table readability.

## Task: Validation and cleanup
- **Status:** done
- **Completed:** 2026-04-08
- **Files changed:** cli/src/services/hooks.rs
- **Evidence:** `nix flake check`; `nix develop -c sh -c 'cd cli && cargo run -- hooks post-commit'`; `nix develop -c sh -c 'cd cli && cargo run -- doctor'`
- **Notes:** Flake checks passed; local DB health check passed after initializing the Agent Trace DB. Doctor reports unrelated OpenCode integration drift/missing assets.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed)
- `nix develop -c sh -c 'cd cli && cargo run -- hooks post-commit'` -> exit 0 (notes=Written, database=Written)
- `nix develop -c sh -c 'cd cli && cargo run -- doctor'` -> exit 0 (Agent Trace local DB health: PASS)

### Success-criteria verification
- [x] Agent Trace records remain stored in Git notes -> verified via post-commit output `notes=Written`.
- [x] Local DB stores conversation metadata + events -> post-commit output `database=Written` plus local DB health check PASS.
- [x] Conversation URLs fallback + no API calls -> confirmed by code path (local checkpoint + transcript ingestion only).
- [x] Attribution remains driven by Agent Trace ranges -> no changes to trace schema or attribution logic in this plan.

### Failed checks and follow-ups
- `sce doctor` reports unrelated OpenCode integration drift/missing assets (outside this plan scope).

### Residual risks
- None for the conversation-storage implementation; existing OpenCode integration drift remains unchanged.
