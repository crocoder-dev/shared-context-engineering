# Agent Trace embedded schema validation

Current internal validation seam for Agent Trace JSON in the Rust CLI.

## Scope

- Code: `cli/src/services/agent_trace.rs`
- Tests: `cli/src/services/agent_trace/tests.rs`
- Schema source: `config/schema/agent-trace.schema.json`

## Current behavior

- The CLI embeds the schema at compile time via `include_str!` from the crate-local mirror at `assets/generated/config/schema/agent-trace.schema.json`, which is prepared during Nix builds (via `flake.nix postUnpack`) and publish-prep (via `scripts/prepare-cli-generated-assets.sh`). The canonical source remains at `config/schema/agent-trace.schema.json`. Validation does not read the schema from disk at runtime.
- `agent_trace_schema_validator()` compiles the embedded schema once and caches the `jsonschema::Validator` in a `OnceLock`.
- `validate_agent_trace_value(&serde_json::Value)` validates already-parsed JSON values against the embedded schema.
- `validate_agent_trace_json(&str)` parses a JSON string and then validates it against the embedded schema.
- Top-level `version` must match strict numeric `x.y.z` (`^[0-9]+\.[0-9]+\.[0-9]+$`); two-part values like `x.y` are rejected.
- Top-level `vcs` is optional at schema level; payloads without `vcs` validate, and payloads that include `vcs` must still provide both `type` and `revision`.
- Validation failures use `AgentTraceValidationError` to distinguish:
  - invalid JSON input
  - schema-validation failures
- Schema-validation errors are sorted before rendering so the exposed error text remains deterministic for a given payload.

## Boundaries

- This seam is internal-only; no user-facing `sce` command currently exposes Agent Trace validation.
- The existing minimal `build_agent_trace(...)` output remains unchanged by this validation seam.
- File-path loading and validation entrypoints are not part of the current implemented slice.

## Verification

- Focused tests cover embedded schema parse/compile, valid schema-shaped JSON-string acceptance, direct `serde_json::Value` validation (including records without top-level `vcs`), and representative schema-invalid rejection (including two-part `version` values and `vcs` objects missing `revision`).

See also: [agent-trace-minimal-generator.md](./agent-trace-minimal-generator.md), [../context-map.md](../context-map.md)
