# Plan: Diff traces model_id

## Change summary

Add optional `model_id` capture to the local diff-trace pipeline so newly captured `session.diff` records can preserve which OpenCode model produced the session changes.

Research conclusion: OpenCode `session.diff` events currently expose only `sessionID` and `diff`; model metadata is not present on that event. The easiest typed plugin source is the `chat.message` hook, which receives `sessionID` and optional `model: { providerID, modelID }`. `chat.params` also receives model/provider data before LLM execution and can be used as a fallback cache source. Lower-level `message.updated` events also include model data, but they are noisier and less direct for this plugin. Do not rely on `session.updated` or `client.session.get(sessionID)` as the primary source unless implementation verifies the installed OpenCode SDK shape exposes model metadata there; the current typed `Session` shape observed during planning is session metadata only, while model data is message/chat-hook scoped.

The implementation should send one optional `model_id` string in the diff-trace payload, formatted as `<providerID>/<modelID>` (for example `openai/gpt-5.5`), and persist it to a nullable `diff_traces.model_id` column.

## Success criteria

- `diff_traces` has a nullable `model_id TEXT` column; existing rows remain valid with `NULL` model IDs.
- `sce hooks diff-trace` accepts existing payloads without `model_id` unchanged.
- `sce hooks diff-trace` accepts and persists a non-empty optional `model_id` string when provided.
- OpenCode agent-trace plugin correlates model metadata by `sessionID` before `session.diff` handling and forwards `model_id` when known.
- `session.diff` records still persist to both `context/tmp/*-diff-trace.json` and AgentTraceDb.
- Generated OpenCode plugin outputs remain in sync with canonical sources.
- Validation passes through repo-standard checks.

## Constraints and non-goals

- Use a nullable DB column; do not backfill historical `diff_traces` rows.
- Keep payload compatibility: `{ sessionID, diff, time }` remains valid.
- Do not change post-commit patch-intersection semantics beyond carrying the new DB field through relevant read/query structs where needed.
- Do not introduce a new external service or durable model cache.
- Do not add model tracking to Claude or non-OpenCode paths in this plan.
- Treat `session.diff` as diff-only; derive model metadata from other OpenCode plugin hooks keyed by `sessionID`.

## Task stack

- [x] T01: `Add nullable model_id to Agent Trace DB` (status:done)
  - Task ID: T01
  - Goal: Add an Agent Trace DB migration and adapter support for nullable `diff_traces.model_id`.
  - Boundaries (in/out of scope): In - new embedded migration, migration list wiring, `DiffTraceInsert` model field, insert SQL, recent diff-trace row structs/query projection, and focused DB/adapter tests. Out - OpenCode plugin payload changes and hook STDIN validation.
  - Done when: New databases create `diff_traces.model_id`; existing databases migrate forward without rewriting rows; inserts can store `NULL` or a model ID; existing recent-patch query behavior remains ordered and unchanged except carrying the optional field.
  - Verification notes (commands or checks): Targeted Rust tests covering migrations/insert/query if available; otherwise repo-level `nix flake check` in final validation.
  - Completed: 2026-05-13
  - Files changed: `cli/migrations/agent-trace/004_add_diff_traces_model_id.sql`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/hooks/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy` passed; `nix build .#checks.x86_64-linux.cli-fmt` passed; `nix build .#default` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Added nullable `model_id` storage/projection while preserving legacy payload insertion as `NULL`; hook intake behavior remains unchanged for T02.

- [x] T02: `Accept optional model_id in diff-trace intake` (status:done)
  - Task ID: T02
  - Goal: Extend `sce hooks diff-trace` STDIN parsing and persistence to accept optional `model_id` while preserving old payload compatibility.
  - Boundaries (in/out of scope): In - payload type, optional string validation, artifact JSON serialization, AgentTraceDb insert mapping, and hook-runtime tests. Out - plugin extraction/correlation logic.
  - Done when: Payloads without `model_id` behave exactly as before; payloads with a non-empty string persist that value to file artifacts and DB; invalid non-string or empty `model_id` values fail with deterministic validation errors.
  - Verification notes (commands or checks): Focused hooks tests for missing, valid, and invalid `model_id`; final `nix flake check`.
  - Completed: 2026-05-13
  - Files changed: `cli/src/services/hooks/mod.rs`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/cli/cli-command-surface.md`, root context summaries/map
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy` passed; `nix build .#checks.x86_64-linux.cli-fmt` passed; `nix build .#default` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Added optional non-empty `model_id` validation for diff-trace STDIN, preserved legacy payload artifact shape by omitting absent `model_id`, and mapped accepted values into `DiffTraceInsert.model_id` for AgentTraceDb persistence.

- [x] T03: `Capture model_id in OpenCode agent-trace plugin` (status:done)
  - Task ID: T03
  - Goal: Correlate OpenCode model metadata by `sessionID` and include `model_id` in diff-trace payloads when known.
  - Boundaries (in/out of scope): In - canonical plugin source under `config/lib/agent-trace-plugin/`, in-memory `sessionID -> model_id` cache, primary extraction from `chat.message`, fallback from `chat.params` if practical in the existing plugin type surface, generated OpenCode plugin output updates, and focused plugin tests if a test harness exists or is added locally to that package. Out - durable cache, DB access from plugin, and changing `session.diff` extraction semantics for patches.
  - Done when: The plugin records `<providerID>/<modelID>` for a session before diff capture; `session.diff` payloads include `model_id` when the cache has a value; sessions without known model still emit valid legacy-compatible payloads; generated plugin files match canonical source.
  - Verification notes (commands or checks): Plugin/runtime tests for model cache and payload shape; `nix run .#pkl-check-generated` to verify generated outputs.
  - Completed: 2026-05-13
  - Files changed: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`, `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.test.ts`, `config/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-agent-trace.ts`, `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/context-map.md`, `context/glossary.md`
  - Evidence: `bun test` in `config/lib/agent-trace-plugin` passed (4 tests, 7 assertions); `nix develop -c tsc -p config/lib/agent-trace-plugin/tsconfig.json` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Added an in-memory `sessionID -> model_id` cache populated from `chat.message` and `chat.params`; `session.diff` payloads include `model_id` only when known and otherwise preserve the legacy payload shape.

- [ ] T04: `Sync current-state context for model_id diff traces` (status:todo)
  - Task ID: T04
  - Goal: Update current-state SCE context to document the new optional model metadata flow.
  - Boundaries (in/out of scope): In - `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, context map/glossary entries if needed. Out - broad historical rewrites of inactive Agent Trace docs.
  - Done when: Context says `session.diff` itself is diff-only, model metadata is correlated from OpenCode chat hooks, diff-trace payload may include `model_id`, and `diff_traces.model_id` is nullable.
  - Verification notes (commands or checks): Manual context consistency check plus final generated-output parity and flake validation.

- [ ] T05: `Validate and cleanup model_id diff-trace rollout` (status:todo)
  - Task ID: T05
  - Goal: Run full validation and remove temporary scaffolding after the implementation tasks land.
  - Boundaries (in/out of scope): In - full repo checks, generated-output parity, temporary test fixture cleanup, plan evidence updates. Out - additional feature work beyond `model_id` capture/persistence.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no temporary debugging artifacts remain; plan status/evidence is updated; context sync is verified.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions

- None. Chosen assumptions: `model_id` is nullable, payload field is a single optional `model_id` string, and OpenCode model metadata should be researched/derived outside `session.diff` rather than assumed to exist on that event.
