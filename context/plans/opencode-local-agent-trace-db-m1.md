# Plan: opencode local Agent Trace DB milestone 1

## Change summary

Implement the first milestone of local Agent Trace persistence for opencode prompt submissions by expanding `cli/src/services/local_db.rs` into the canonical SQLite persistence service and wiring the first real submit-time integration.

This milestone establishes lazy, automatic persistence on prompt submit with no manual pre-init workflow:

1. ensure local SQLite DB exists (repo-local `.sce/local.db`)
2. initialize schema when missing
3. ensure active session exists
4. ensure active conversation exists
5. persist submitted prompt immediately

User-confirmed decisions:

- Canonical DB location for this milestone: repo-local `.sce/local.db`
- Update `cli/src/services/local_db.rs` in place as the canonical persistence module
- Submit-time persistence scope for now: prompt-only (assistant-message write path can be wired later)

## Success criteria

1. `cli/src/services/local_db.rs` owns SQLite initialization and schema creation for the milestone tables: `sessions`, `conversations`, `prompts`, `assistant_messages`, `file_observations`, `file_ranges`, `trace_exports`.
2. IDs are UUIDs and timestamps are RFC3339 UTC strings; SQL and schema/migration logic remain isolated in `local_db.rs`.
3. The module exposes the required typed API surface (or exact-name equivalent): `init_db`, `create_session`, `end_session`, `create_conversation`, `append_prompt`, `append_assistant_message`, `record_file_observation`, `record_file_range`, `get_conversation_prompts`, `get_conversation_ranges`, `get_minimal_trace_inputs`, plus submit-time orchestration helpers for lazy init/session/conversation.
4. Prompt submit in opencode automatically performs lazy DB init + active session/conversation ensure + prompt persistence with no manual setup command.
5. Persistence trigger is submit-time only (no keypress/on-change logging).
6. Integration tests cover DB init + core writes/reads + minimal trace inputs + submit-time auto-create/persist behavior from the real prompt submit path (or closest existing path if no dedicated hook exists).
7. Internal session/conversation IDs are not surfaced in user-facing UI output.

## Constraints and non-goals

- In scope:
  - local SQLite persistence layer for future Agent Trace generation inputs
  - opencode submit-time prompt persistence integration
  - modular API design to support future hooks/git-notes/export integration
- Out of scope for this milestone:
  - full Agent Trace export payload/command implementation
  - git notes integration
  - pre-commit/post-commit hook persistence flows
  - full file diff/range inference
  - keypress-level prompt logging
- Keep implementation simple and reliable; prefer `rusqlite` sync path unless an existing clean abstraction makes a different choice clearly superior.

## Task stack

- [x] T01: `Expand local_db.rs into canonical SQLite schema + typed persistence service` (status:done)
  - Task ID: T01
  - Goal: Replace the current neutral local-DB baseline with a concrete SQLite-backed schema bootstrap and typed CRUD/query helpers in `cli/src/services/local_db.rs` for the milestone entities.
  - Boundaries (in/out of scope): In - SQLite connection/bootstrap, table/index creation, UUID/timestamp/hash handling, transactional write helpers, typed return models for prompt/range/minimal-trace reads, and robust error mapping. Out - submit-path wiring (T02), export/hook/git-note behavior, non-opencode persistence flows.
  - Done when: `local_db.rs` can initialize a repo-local `.sce/local.db` and perform typed insert/read operations for sessions, conversations, prompts, assistant messages, file observations, file ranges, and trace export payload rows.
  - Verification notes (commands or checks): Add focused service/integration tests using temp repo fixtures and SQLite files validating schema creation, indexes, and CRUD round-trips; verify UUID + RFC3339 UTC timestamp shape in persisted rows.
  - Completed: 2026-04-16
  - Evidence: `nix flake check` (pass)
  - Files changed: `cli/src/services/local_db.rs`, `cli/src/services/mod.rs`, `cli/Cargo.toml`

- [x] T02: `Add submit-time orchestration helpers for lazy init + active session/conversation` (status:done)
  - Task ID: T02
  - Goal: Implement orchestration helpers (e.g., `ensure_db_initialized`, `ensure_active_session`, `ensure_active_conversation`, `append_prompt_with_auto_init`) that make submit-time prompt persistence one-call and idempotent for runtime callers.
  - Boundaries (in/out of scope): In - helper APIs and state/lookup behavior required to reuse active session/conversation when present and lazily create when absent. Out - UI exposure of internal IDs, assistant-response capture wiring, command-surface additions.
  - Done when: A caller can invoke one submit-oriented helper path and reliably get lazy DB bootstrap + active session/conversation + persisted prompt in one flow without pre-running setup commands.
  - Verification notes (commands or checks): Add helper-focused tests covering missing DB path auto-create, first-submit creation path, and subsequent-submit reuse path for active session/conversation.
  - Completed: 2026-04-16
  - Evidence: `nix flake check` (pass); `nix run .#pkl-check-generated` (pass)
  - Files changed: `cli/src/services/local_db.rs`

- [x] T03: `Wire the opencode prompt submit path to local DB auto-persistence` (status:done)
  - Task ID: T03
  - Goal: Integrate the orchestration helper into the real opencode prompt submit lifecycle point (or closest existing submission path) so prompt submit is the trigger for automatic persistence.
  - Boundaries (in/out of scope): In - minimal submit-path wiring and error-handling strategy consistent with existing runtime behavior; optional model/provider propagation when already available in submit context. Out - keystroke logging, full conversation replay, new manual setup/init commands.
  - Done when: Normal prompt submit execution auto-creates `.sce/local.db` if missing, ensures active session/conversation, and writes prompt text/hash/timestamp/sequence without user pre-initialization steps.
  - Verification notes (commands or checks): Add/extend integration test that simulates submit path end-to-end and asserts DB file creation + inserted prompt row after submit.
  - Completed: 2026-04-16
  - Evidence: `nix flake check` (pass); `nix run .#pkl-check-generated` (pass)
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/command_surface.rs`, `cli/src/services/mod.rs`, `cli/src/services/trace.rs`, `config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts`, `config/.opencode/plugins/sce-bash-policy.ts`, `config/automated/.opencode/plugins/sce-bash-policy.ts`, `.opencode/plugins/sce-bash-policy.ts`

- [x] T04: `Add milestone integration coverage for minimal trace inputs` (status:done)
  - Task ID: T04
  - Goal: Ensure persistence contracts needed for future minimal Agent Trace generation are test-verified, including minimal trace input retrieval and file observation/range records.
  - Boundaries (in/out of scope): In - integration tests for `append_assistant_message` API, `record_file_observation`, `record_file_range`, and `get_minimal_trace_inputs`/range-query helpers using temporary SQLite files. Out - full trace JSON export and git-linked provenance.
  - Done when: Tests demonstrate that milestone data model can retrieve conversation prompts/ranges and minimal trace-ready aggregates deterministically.
  - Verification notes (commands or checks): Integration tests assert stored/retrieved sequence ordering, indexed lookups by `conversation_id` and `path`, and stable minimal-trace input shape.
  - Completed: 2026-04-16
  - Evidence: `nix flake check -L` (pass)
  - Files changed: `cli/src/services/local_db.rs`

- [x] T05: `Validation, cleanup, and context sync` (status:done)
  - Task ID: T05
  - Goal: Run full validation for the completed milestone, remove temporary scaffolding, and sync SCE context to the new local DB + submit-time persistence truth.
  - Boundaries (in/out of scope): In - full repo validation checks, plan status/evidence updates, and context updates for local DB + submit behavior contracts. Out - additional feature expansion beyond accepted milestone scope.
  - Done when: Validation passes, no temporary scaffolding remains, and relevant context files reflect current runtime behavior (including replacement of the old empty-file local DB baseline).
  - Verification notes (commands or checks): Run repo-standard validation suite, then update/verify `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/sce/agent-trace-core-schema-migrations.md` (plus any submit-path contract docs touched by final code truth).
  - Completed: 2026-04-16
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix flake check` (pass)
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/cli/cli-command-surface.md`, `context/sce/agent-trace-core-schema-migrations.md`, `context/plans/opencode-local-agent-trace-db-m1.md`

## Validation Report (final task)

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)

### Lint/format/test coverage

- `nix flake check` includes the repository-standard validation surface for this project, including Rust tests, clippy, fmt, and scoped JS/Bun/Biome checks documented in `AGENTS.md`.

### Temporary scaffolding cleanup

- No temporary scaffolding for this milestone remained in the implemented runtime paths; no cleanup deletions were required in this final task.

### Success-criteria verification summary

- [x] (1) `local_db.rs` owns milestone schema/tables -> confirmed in `cli/src/services/local_db.rs` and reflected in `context/sce/agent-trace-core-schema-migrations.md`.
- [x] (2) UUID + RFC3339 UTC + schema logic isolation -> confirmed by `local_db.rs` API/tests and milestone context updates (`overview/architecture/glossary`).
- [x] (3) Required typed API surface exists -> confirmed in `cli/src/services/local_db.rs` (T01/T02 outputs).
- [x] (4) Submit-time lazy init + prompt persistence is wired -> confirmed by `cli/src/services/trace.rs` and OpenCode plugin bridge (`config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts`).
- [x] (5) Persistence trigger remains submit-time only -> confirmed by current `trace append-prompt` bridge contract and no keypress logging paths.
- [x] (6) Integration coverage for init/writes/reads/minimal trace inputs/submit path exists -> covered by `local_db.rs` tests and validated in `nix flake check` evidence.
- [x] (7) Internal session/conversation IDs not exposed in user-facing UI -> current command/context contract documents no ID surfacing.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for milestone-1 scope.

## Open questions

None.
