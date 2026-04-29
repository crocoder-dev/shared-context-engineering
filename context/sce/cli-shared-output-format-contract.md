# CLI Shared Output-Format Contract

## Scope

`T13` introduces one canonical output-format contract for CLI commands that support dual text/JSON rendering.

- Canonical type lives at `cli/src/services/output_format.rs` as `OutputFormat`.
- Allowed values are `text` and `json`.
- Parsing is command-context aware via `OutputFormat::parse(raw, help_command)` so invalid values include command-specific help guidance.

## Current command integration

- `cli/src/services/config/mod.rs` uses the shared type through `ReportFormat = OutputFormat`.
- `cli/src/services/version.rs` uses the shared type through `VersionFormat = OutputFormat`.
- Both commands keep deterministic default `text` when `--format` is omitted.
- Both commands keep stable `--format <text|json>` usage in help text and parser behavior.

## Validation/error contract

- Invalid values fail deterministically as validation errors with this canonical structure:
  - `Invalid --format value '<value>'. Valid values: text, json. Run '<command> --help' to see valid usage.`
- Missing `--format` value behavior remains command parser-owned (`Option '--format' requires a value`).

## Determinism notes

- The shared parser only accepts lowercase canonical values (`text`, `json`).
- Existing command business logic and payload shape remain unchanged; only format parsing contract ownership is centralized.
