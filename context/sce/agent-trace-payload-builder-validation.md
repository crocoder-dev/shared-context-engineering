# Agent Trace Payload Builder And Validation

## Scope

- Plan/task: `agent-trace-attribution-no-git-wrapper` / `T03`.
- Canonical implementation file: `cli/src/services/agent_trace.rs`.
- Purpose: define one deterministic payload-builder path on top of the adapter and verify Agent Trace schema compliance.

## Current-state contract

- `build_trace_payload(input)` is the canonical builder entrypoint.
- Builder behavior is deterministic for identical inputs:
  - uses adapter output as the single source path
  - normalizes AI `model_id` values when provider/model form is inferable (`provider:model` -> `provider/model`, lowercase)
  - keeps non-normalizable values intact instead of dropping attribution data
- Record shape remains aligned with Agent Trace-required top-level fields (`version`, `id`, `timestamp`, `files`) and local invariant `vcs.type = "git"`.

## Validation suite

- Validation tests compile the published Agent Trace trace-record schema and validate builder output.
- Format validation is enabled (`date-time`, `uri`, `uuid`) via `jsonschema` draft-2020-12 options.
- Schema checks cover:
  - required fields + enum constraints
  - nested `files[].conversations[].ranges[]` structure
  - related-link preservation using schema-compatible related objects in test payload rendering
  - negative format tests for invalid URI and RFC3339 timestamp values

## Published-schema compatibility note

- The published schema pattern for `version` currently accepts two-segment versions (`x.y`) while RFC examples and this implementation emit the CLI app version from `CARGO_PKG_VERSION` (currently sourced from the repo's centralized app version).
- Test validation applies a local compatibility patch to the version regex (`x.y` or `x.y.z`) to keep compliance tests aligned with the current emitted contract.

## Verification commands

- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `cargo test --manifest-path cli/Cargo.toml`
- `cargo build --manifest-path cli/Cargo.toml`
