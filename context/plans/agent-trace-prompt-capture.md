# Agent Trace Prompt Capture

## Change Summary

Add multi-prompt capture to Agent Trace for debugging, analytics, and audit trails. Capture the user's last input(s) per commit, including context (cwd, branch) and metrics (tool calls, duration), stored separately from the trace payload in local DB.

### Scope
- Prompts table in local DB with full context and metrics
- Claude Code `UserPromptSubmit` hook for capturing prompts
- Checkpoint artifact extension for prompts array (with `has_sce_attribution` field)
- Post-commit persistence with deduplication
- CLI query command `sce trace prompts <commit-sha>`

### Out of Scope
- Session tracking (simplified design)
- Full conversation history (only last inputs per turn)
- Other AI harnesses (Claude Code first, extensible design)
- Storage in git notes (prompts stay in local DB only)
- Automatic cleanup/pruning (append-only with dedupe)

### Recent Code Changes (as of 2026-03-18)
- **Commit 3b266f0**: Added `has_sce_attribution` field to checkpoint structs (`PendingFileCheckpoint`, `FinalizedFileCheckpoint`). T03 checkpoint format must include this field to remain compatible with the updated checkpoint schema.

## Success Criteria

1. Schema migration adds `prompts` table with all fields
2. Claude Code hook captures prompts to `.git/sce/prompts.jsonl`
3. Pre-commit hook reads prompts and writes to checkpoint artifact
4. Post-commit hook persists prompts to local DB
5. `sce trace prompts <sha>` displays prompts with context and metrics
6. JSON output option works: `sce trace prompts <sha> --json`
7. Duplicate prompts on recommit are handled gracefully
8. Truncation (10KB) works with `is_truncated` flag

## Constraints and Non-Goals

- **No session tracking**: Sessions table intentionally omitted for simplicity
- **Claude Code only**: First harness, schema supports extensibility
- **10KB limit**: Prompts truncated beyond this, flagged in output
- **Append-only cleanup**: File grows, dedupe on commit
- **Local DB only**: Prompts not mirrored to git notes or backend
- **No automatic pruning**: Manual cleanup if needed later

## Task Stack

- [x] T01: Add prompts table schema migration (status:done)
  - Task ID: T01
  - Goal: Create `prompts` table in local DB with all required fields and indexes
  - Boundaries (in/out of scope):
    - In scope: Migration file, table schema, indexes, idempotent `CREATE TABLE IF NOT EXISTS`
    - Out of scope: Hook integration, data population, CLI queries
  - Done when:
    - `prompts` table exists with all columns
    - Indexes created
    - Migration passes `cargo test core_schema_migrations`
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml core_schema_migrations`
    - `cargo build --manifest-path cli/Cargo.toml`
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/services/local_db.rs`
  - **Evidence:** Build succeeded (9.15s), local_db smoke test passed, fmt clean, clippy clean

- [x] T02: Create Claude Code hook for prompt capture (status:done)
  - Task ID: T02
  - Goal: Implement `UserPromptSubmit` hook that appends prompts to `.git/sce/prompts.jsonl`
  - Boundaries (in/out of scope):
    - In scope: Hook config file, JSONL format, session_id/cwd/timestamp capture
    - Out of scope: Model detection, tool counting, duration tracking
  - Done when:
    - Hook file `.claude/hooks/sce-capture-prompt.json` exists
    - Prompts append to `.git/sce/prompts.jsonl` on each submit
    - JSON format: `{"session_id": "...", "prompt": "...", "cwd": "...", "timestamp": "..."}`
  - Verification notes (commands or checks):
    - Manual test: Submit prompt in Claude Code, verify file created and appended
  - **Completed:** 2026-03-18
  - **Files changed:** `config/pkl/generate.pkl`, `config/pkl/lib/claude-settings.json`, `config/pkl/lib/claude-capture-prompt.js`, `config/pkl/lib/claude-prompt-capture-hook.json`, `config/.claude/settings.json`, `config/.claude/hooks/sce-capture-prompt.js`, `config/.claude/hooks/sce-capture-prompt.json`
  - **Evidence:** Prompt-capture smoke test wrote JSONL to the resolved git-dir `sce/prompts.jsonl`; `nix run .#pkl-check-generated` passed; `nix flake check` reached full evaluation after temporarily staging new generated-source files for validation, but the run could not complete because the Nix daemon disconnected after repeated cache/DNS fetch failures

- [x] T03: Extend checkpoint artifact with prompts array (status:done)
  - Task ID: T03
  - Goal: Pre-commit hook reads `prompts.jsonl` and writes prompts to checkpoint
  - Boundaries (in/out of scope):
    - In scope: Parse prompts.jsonl, add `prompts` array to checkpoint, dedupe logic, include `has_sce_attribution` field
    - Out of scope: Model/tool metadata (captured at commit time in T04)
  - Done when:
    - Checkpoint includes `harness_type`, `prompts[]` array, and `has_sce_attribution` in files
    - Each prompt has `turn_number`, `prompt_text`, `captured_at`
    - Deduplication works (same prompt not added twice)
    - `has_sce_attribution` field populated per recent checkpoint schema changes
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml pre_commit_checkpoint`
    - Inspect checkpoint file after commit
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/services/hooks.rs`, `cli/src/services/hooks/tests.rs`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo test pre_commit_finalization'` passed (4 tests); `nix develop -c sh -c 'cd cli && cargo test load_pending_prompts'` passed; `nix develop -c sh -c 'cd cli && cargo build'` succeeded; `nix run .#pkl-check-generated` passed; `nix flake check` passed

- [x] T04: Capture commit-time metadata (status:done)
  - Task ID: T04
  - Goal: Enrich prompt records with model, branch, context at commit time
  - Boundaries (in/out of scope):
    - In scope: Detect git branch, model_id from env/context, cwd from last prompt
    - Out of scope: Tool counting (T05), duration calculation (T05)
  - Done when:
    - `model_id` captured (from env or hook context)
    - `git_branch` captured from git
    - `cwd` inherited from last prompt
  - Verification notes (commands or checks):
    - Checkpoint includes branch and model info
    - Tests verify branch detection
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/services/hooks.rs`, `cli/src/services/hooks/tests.rs`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo test pre_commit_finalization'` passed (4 tests); `nix develop -c sh -c 'cd cli && cargo test load_pending_prompts'` passed (2 tests); `nix develop -c sh -c 'cd cli && cargo test resolve_pre_commit_git_branch'` passed; `nix develop -c sh -c 'cd cli && cargo build'` succeeded; `nix run .#pkl-check-generated` passed; `nix flake check` currently fails in unrelated setup coverage (`services::setup::tests::setup_install_copies_bash_policy_assets_for_opencode_and_claude`)

- [x] T05: Post-commit persistence and metrics (status:done)
  - Task ID: T05
  - Goal: Persist prompts to DB with tool count and duration metrics
  - Boundaries (in/out of scope):
    - In scope: Insert prompts to DB, calculate `duration_ms`, count tools, truncate if >10KB
    - Out of scope: CLI query display
  - Done when:
    - Post-commit hook inserts all prompts to `prompts` table
    - `tool_call_count` populated
    - `duration_ms` calculated between prompts
    - `is_truncated` flag set if >10KB
    - Truncation applied
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml post_commit_prompts`
    - Query DB: `SELECT * FROM prompts WHERE commit_id = ?`
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/services/hooks.rs`, `cli/src/services/hooks/tests.rs`, `config/pkl/lib/claude-capture-prompt.js`, `config/.claude/hooks/sce-capture-prompt.js`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo test post_commit_finalization'` passed; `nix develop -c sh -c 'cd cli && cargo test load_post_commit_prompt_records'` passed; `nix develop -c sh -c 'cd cli && cargo test load_pending_prompts'` passed; `nix develop -c sh -c 'cd cli && cargo test pre_commit_finalization'` passed; `nix develop -c sh -c 'cd cli && cargo build'` succeeded; `nix run .#pkl-check-generated` passed; `nix flake check` passed

- [x] T06: CLI query command (status:done)
  - Task ID: T06
  - Goal: Implement `sce trace prompts <commit-sha>` with human and JSON output
  - Boundaries (in/out of scope):
    - In scope: Human-readable table output, `--json` flag, error handling for missing commits
    - Out of scope: Filtering, stats view (stretch)
  - Done when:
    - `sce trace prompts abc1234` shows formatted output
    - `sce trace prompts abc1234 --json` returns JSON
    - Handles missing commits gracefully
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml trace_prompts_command`
    - Manual: Run command on commits with/without prompts
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/app.rs`, `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/mod.rs`, `cli/src/services/trace.rs`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo test trace'` passed; `nix develop -c sh -c 'cd cli && cargo build'` succeeded; `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'` passed; `nix develop -c sh -c 'cd cli && cargo fmt --check'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` could not complete from the dirty worktree because Nix evaluated the tracked git tree before the new untracked `cli/src/services/trace.rs` file was included

- [x] T07: Validation and cleanup (status:done)
  - Task ID: T07
  - Goal: Full integration test, verify all tasks work together
  - Boundaries (in/out of scope):
    - In scope: End-to-end test, context sync, documentation update
    - Out of scope: Additional features, optimization
  - Done when:
    - Full flow works: hook -> checkpoint -> DB -> query
    - All tests pass
    - Context docs updated
    - No regressions in existing tests
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml`
    - `cargo build --manifest-path cli/Cargo.toml`
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - Update `context/sce/agent-trace-*` docs as needed
  - **Completed:** 2026-03-18
  - **Files changed:** `cli/src/services/hooks/tests.rs`, `cli/src/services/trace.rs`, `context/plans/agent-trace-prompt-capture.md`, `context/sce/agent-trace-prompt-persistence-metrics.md`, `context/sce/agent-trace-prompt-query-command.md`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo test prompt_capture_flow_persists_and_queries_end_to_end'` passed; `nix develop -c sh -c 'cd cli && cargo test'` passed; `nix develop -c sh -c 'cd cli && cargo build'` succeeded; `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'` passed; `nix develop -c sh -c 'cd cli && cargo fmt --check'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` remained blocked by the dirty worktree because Nix evaluated the tracked git tree before untracked sources such as `cli/src/services/trace.rs` were included

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test prompt_capture_flow_persists_and_queries_end_to_end'` -> exit 0 (`services::hooks::tests::prompt_capture_flow_persists_and_queries_end_to_end` passed)
- `nix develop -c sh -c 'cd cli && cargo test trace'` -> exit 0 (22 trace/prompt-related tests passed)
- `nix develop -c sh -c 'cd cli && cargo test hooks'` -> exit 0 (48 hooks-related tests passed)
- `nix develop -c sh -c 'cd cli && cargo test'` -> exit 0 (full CLI suite passed)
- `nix develop -c sh -c 'cd cli && cargo build'` -> exit 0
- `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'` -> exit 0
- `nix develop -c sh -c 'cd cli && cargo fmt --check'` -> exit 0
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 1 because Nix evaluated the tracked git tree without untracked sources in the dirty worktree (`cli/src/services/trace.rs` was excluded from the flake build snapshot)

### Success-criteria verification

- [x] Schema migration adds `prompts` table with all fields -> covered by earlier completed T01 migration work and preserved by passing full `cargo test`
- [x] Claude Code hook captures prompts to `.git/sce/prompts.jsonl` -> covered by existing prompt-capture hook implementation and retained by the end-to-end validation path
- [x] Pre-commit hook reads prompts and writes to checkpoint artifact -> confirmed by `prompt_capture_flow_persists_and_queries_end_to_end`
- [x] Post-commit hook persists prompts to local DB -> confirmed by `prompt_capture_flow_persists_and_queries_end_to_end` and `cargo test hooks`
- [x] `sce trace prompts <sha>` displays prompts with context and metrics -> confirmed by `prompt_capture_flow_persists_and_queries_end_to_end` and `cargo test trace`
- [x] JSON output option works: `sce trace prompts <sha> --json` -> confirmed by `prompt_capture_flow_persists_and_queries_end_to_end` and `cargo test trace`
- [x] Duplicate prompts on recommit are handled gracefully -> confirmed by `cargo test load_pending_prompts`
- [x] Truncation (10KB) works with `is_truncated` flag -> confirmed by `cargo test load_post_commit_prompt_records`

### Context sync

- Classified as verify-only for root context: no repository-wide behavior, architecture, or terminology changed during T07 validation.
- Verified existing root coverage in `context/overview.md`, `context/architecture.md`, and `context/glossary.md`; no root edits required.
- Updated feature-local context in `context/sce/agent-trace-prompt-persistence-metrics.md` and `context/sce/agent-trace-prompt-query-command.md` to record end-to-end verification coverage.

### Failed checks and follow-ups

- `nix flake check` is still blocked in the current dirty worktree because Nix built from the tracked git snapshot and excluded untracked files already present in this branch. Re-run after those files are tracked/staged in git.

### Residual risks

- Final repo-wide flake validation still depends on re-running `nix flake check` from a worktree where the relevant new files are part of the tracked git snapshot.

## Implementation Details

### Database Schema

Add to `cli/src/services/local_db.rs` in `CORE_SCHEMA_STATEMENTS`:

```sql
-- Prompts table for multi-prompt capture per commit
CREATE TABLE IF NOT EXISTS prompts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_id INTEGER NOT NULL,
    
    -- Prompt content
    prompt_text TEXT NOT NULL,
    prompt_length INTEGER,          -- Original length before truncation
    is_truncated BOOLEAN DEFAULT 0,
    turn_number INTEGER,
    
    -- Harness info
    harness_type TEXT NOT NULL,     -- 'claude_code', 'opencode', 'manual'
    model_id TEXT,                  -- claude-sonnet-4-20250514, etc.
    
    -- Context
    cwd TEXT,                       -- Working directory
    git_branch TEXT,                -- Branch at prompt time
    
    -- Operational metrics
    tool_call_count INTEGER,        -- Tools invoked in response
    duration_ms INTEGER,            -- Time until next prompt/commit
    captured_at TEXT NOT NULL,
    
    FOREIGN KEY (commit_id) REFERENCES commits(id) ON DELETE CASCADE
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_prompts_commit ON prompts(commit_id);
CREATE INDEX IF NOT EXISTS idx_prompts_harness ON prompts(harness_type);
CREATE INDEX IF NOT EXISTS idx_prompts_captured ON prompts(captured_at);
CREATE INDEX IF NOT EXISTS idx_prompts_commit_turn ON prompts(commit_id, turn_number);
```

### Claude Code Hook Format

**File:** `.claude/hooks/sce-capture-prompt.json`

```json
{
  "UserPromptSubmit": [
    {
      "matcher": "*",
      "hooks": [
        {
          "type": "command",
          "command": "mkdir -p .git/sce && echo '{\"session_id\": \"$SESSION_ID\", \"prompt\": \"$USER_PROMPT\", \"cwd\": \"$CWD\", \"timestamp\": \"$(date -Iseconds)\"}' >> .git/sce/prompts.jsonl"
        }
      ]
    }
  ]
}
```

**Output format** (`.git/sce/prompts.jsonl`):
```json
{"session_id": "abc123", "prompt": "add error handling", "cwd": "/home/user/project/src", "timestamp": "2026-03-13T10:30:00+00:00"}
{"session_id": "abc123", "prompt": "also handle null token", "cwd": "/home/user/project/src", "timestamp": "2026-03-13T10:32:00+00:00"}
```

### Checkpoint Artifact Extension

**Location:** `.git/sce/pre-commit-checkpoint.json`

```json
{
  "files": [
    {
      "path": "src/auth.js",
      "has_sce_attribution": true,
      "ranges": [...]
    }
  ],
  "harness_type": "claude_code",
  "git_branch": "feature/auth-refactor",
  "model_id": "claude-sonnet-4-20250514",
  "prompts": [
    {
      "turn_number": 1,
      "prompt_text": "add error handling to auth module",
      "prompt_length": 35,
      "is_truncated": false,
      "cwd": "/home/user/project/src",
      "captured_at": "2026-03-13T10:30:00Z"
    },
    {
      "turn_number": 2,
      "prompt_text": "also handle the edge case where token is null",
      "prompt_length": 46,
      "is_truncated": false,
      "cwd": "/home/user/project/src",
      "captured_at": "2026-03-13T10:32:00Z"
    }
  ]
}
```

**Note:** The `has_sce_attribution` field in each file entry is required per recent checkpoint schema changes (commit 3b266f0). This prevents co-author false positives on human-only commits.

### CLI Output Formats

**Human-readable (default):**
```
$ sce trace prompts abc1234

Commit: abc1234
Harness: claude_code
Model: claude-sonnet-4-20250514
Branch: feature/auth-refactor
Total prompts: 3

Turn 1  10:30:00Z  cwd: src/auth  duration: 2m  tools: 5
  add error handling to auth module

Turn 2  10:32:00Z  cwd: src/auth  duration: 3m  tools: 3
  also handle null token edge case

Turn 3  10:35:00Z  cwd: tests/  duration: 1.5m  tools: 8  [truncated]
  refactor the entire authentication flow to support OAuth2, SAML, and custom...
```

**JSON output (`--json`):**
```json
{
  "commit": "abc1234",
  "harness": "claude_code",
  "model": "claude-sonnet-4-20250514",
  "branch": "feature/auth-refactor",
  "prompt_count": 3,
  "prompts": [
    {
      "turn_number": 1,
      "text": "add error handling to auth module",
      "length": 35,
      "cwd": "src/auth",
      "duration_ms": 120000,
      "tool_call_count": 5,
      "captured_at": "2026-03-13T10:30:00Z",
      "is_truncated": false
    },
    {
      "turn_number": 2,
      "text": "also handle null token edge case",
      "length": 46,
      "cwd": "src/auth",
      "duration_ms": 180000,
      "tool_call_count": 3,
      "captured_at": "2026-03-13T10:32:00Z",
      "is_truncated": false
    },
    {
      "turn_number": 3,
      "text": "refactor the entire authentication flow...",
      "length": 10240,
      "cwd": "tests/",
      "duration_ms": 90000,
      "tool_call_count": 8,
      "captured_at": "2026-03-13T10:35:00Z",
      "is_truncated": true,
      "original_length": 20480
    }
  ]
}
```

### Duration Calculation

- **Between prompts:** `duration_ms = next_prompt.captured_at - current_prompt.captured_at`
- **Last prompt:** `duration_ms = commit_timestamp - last_prompt.captured_at`
- Stored in the prompt record for the *current* turn (how long was spent on it)

### Truncation Logic

```rust
const MAX_PROMPT_BYTES: usize = 10 * 1024; // 10KB

fn truncate_prompt(text: &str) -> (String, bool, usize) {
    let original_len = text.len();
    if original_len <= MAX_PROMPT_BYTES {
        return (text.to_string(), false, original_len);
    }
    
    // Truncate to 10KB, preserving valid UTF-8
    let truncated = match text.char_indices().take_while(|(i, _)| *i < MAX_PROMPT_BYTES).last() {
        Some((idx, _)) => &text[..idx],
        None => &text[..MAX_PROMPT_BYTES],
    };
    
    (truncated.to_string(), true, original_len)
}
```

### Deduplication Strategy

When reading `prompts.jsonl` in pre-commit:
1. Parse all entries
2. Hash `(prompt_text, captured_at)` for each
3. Skip if hash already exists in checkpoint (handles recommits)
4. Assign `turn_number` sequentially (1, 2, 3...)

### Error Handling

- **Missing prompts.jsonl:** No prompts captured (empty prompts array in checkpoint)
- **Corrupted JSONL:** Log warning, skip corrupted lines, continue with valid ones
- **DB insert failure:** Enqueue in retry queue (reuse existing `TraceRetryQueue` pattern)
- **Commit not found:** CLI returns error with helpful message

## Open Questions

None - all clarified during design discussion.

## Related Context

- `context/sce/agent-trace-implementation-contract.md`
- `context/sce/agent-trace-core-schema-migrations.md`
- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/sce/agent-trace-pre-commit-staged-checkpoint.md`

---

**Plan created:** 2026-03-13  
**Plan updated:** 2026-03-18  
**Status:** Ready for implementation  
**Next:** `/next-task agent-trace-prompt-capture T07`
