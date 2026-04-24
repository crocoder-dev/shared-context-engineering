# Agent Trace embedded schema validation

Current internal validation seam for Agent Trace JSON in the Rust CLI.

## Scope

- Code: `cli/src/services/agent_trace.rs`
- Tests: `cli/src/services/agent_trace/tests.rs`
- Schema source: `config/schema/agent-trace.schema.json`

## Current behavior

- The CLI embeds `config/schema/agent-trace.schema.json` at compile time via `include_str!`; validation does not read the schema from disk at runtime.
- `agent_trace_schema_validator()` compiles the embedded schema once and caches the `jsonschema::Validator` in a `OnceLock`.
- `validate_agent_trace_value(&serde_json::Value)` validates already-parsed JSON values against the embedded schema.
- `validate_agent_trace_json(&str)` parses a JSON string and then validates it against the embedded schema.
- Validation failures use `AgentTraceValidationError` to distinguish:
  - invalid JSON input
  - schema-validation failures
- Schema-validation errors are sorted before rendering so the exposed error text remains deterministic for a given payload.

## Boundaries

- This seam is internal-only; no user-facing `sce` command currently exposes Agent Trace validation.
- The existing minimal `build_agent_trace(...)` output remains unchanged by this validation seam.
- File-path loading and validation entrypoints are not part of the current implemented slice.

## Verification

- Focused tests cover embedded schema parse/compile, valid schema-shaped JSON-string acceptance, direct `serde_json::Value` validation, and representative schema-invalid rejection.

See also: [agent-trace-minimal-generator.md](./agent-trace-minimal-generator.md), [../context-map.md](../context-map.md)
