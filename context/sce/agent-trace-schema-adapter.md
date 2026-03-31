# Agent Trace Schema Adapter

## Scope

- Plan/task: `agent-trace-attribution-no-git-wrapper` / `T02`.
- Purpose: define a deterministic adapter contract that maps internal attribution inputs to Agent Trace record shape, without persistence or hook side effects.

## Canonical code location

- `cli/src/services/agent_trace.rs`

## Adapter contract (current state)

- Input contract is `TraceAdapterInput` with commit identity, timestamp, record id, file attribution payload, quality status, and optional rewrite/idempotency metadata.
- Output contract is `AgentTraceRecord` with:
  - required top-level fields (`version`, `id`, `timestamp`, `files`)
  - fixed local VCS block (`vcs.type = "git"`, `vcs.revision = <commit sha>`)
  - reverse-domain metadata keys under `dev.crocoder.sce.*`
- Canonical constants are centralized for trace/media/reference values:
  - `TRACE_VERSION = env!("CARGO_PKG_VERSION")` so emitted Agent Trace record versions follow the CLI app version at compile time
  - `NOTES_REF = "refs/notes/agent-trace"`
  - `TRACE_CONTENT_TYPE = "application/vnd.agent-trace.record+json"`

## Mapping guarantees in this slice

- Contributor enum mapping is explicit and constrained to `human|ai|mixed|unknown`.
- Conversation links preserve `url` and optional `related` values.
- Extension metadata placement uses reserved keys:
  - `dev.crocoder.sce.quality_status`
  - `dev.crocoder.sce.rewrite_from`
  - `dev.crocoder.sce.rewrite_method`
  - `dev.crocoder.sce.rewrite_confidence`
  - `dev.crocoder.sce.idempotency_key`
  - `dev.crocoder.sce.notes_ref`
  - `dev.crocoder.sce.content_type`

## Verification evidence

- `nix flake check` includes the current repo-level verification flow for the CLI and related checks.

## Out of scope (deferred)

- JSON schema compliance/runtime format validation and deterministic serialization checks (`T03`).
- Hook orchestration, notes/DB writes, and rewrite execution flows (`T04+`).

## Follow-on coverage

- `T03` is now implemented in `context/sce/agent-trace-payload-builder-validation.md` with builder-path and schema-validation details layered on this adapter contract.
