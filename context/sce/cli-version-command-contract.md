# CLI Version Command Contract

## Scope

Defines the implemented `sce version` runtime contract for deterministic human and machine-readable runtime identification.

## Command surface

- Command: `sce version`
- Help: `sce version --help`
- Format option: `--format <text|json>`
- Default format: `text`

## Parsing and validation

- Accepts only `--format <text|json>` (plus `--help`/`-h` when used alone).
- Rejects unknown flags with deterministic guidance:
  - `Unknown version option '--<name>'. Run 'sce version --help' to see valid usage.`
- Rejects unexpected positional args with deterministic guidance:
  - `Unexpected version argument '<value>'. Run 'sce version --help' to see valid usage.`
- Rejects unsupported formats with deterministic guidance:
  - `Unsupported --format value '<value>'. Valid values: text, json.`

## Output contract

Text output (`sce version`):

- Single deterministic line:
  - `<binary> <version> (<build_profile>)`

JSON output (`sce version --format json`):

- Stable object fields:
  - `status`: always `"ok"`
  - `command`: always `"version"`
  - `binary`: compile-time package/binary name
  - `version`: compile-time package version
  - `build_profile`: compile-time build profile (`"debug"` or `"release"`)

## Implementation ownership

- Command parse/dispatch wiring: `cli/src/app.rs`
- Top-level command catalog/help row: `cli/src/command_surface.rs`
- Version parser/rendering: `cli/src/services/version/mod.rs`

## Verification coverage

- `app::tests` lock `version` command routing and help behavior.
- `services::version::tests` lock default/JSON parsing and stable JSON field presence.
- `command_surface::tests` lock top-level help discoverability for `version`.
