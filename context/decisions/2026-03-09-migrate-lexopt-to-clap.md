# Decision: Migrate from lexopt to clap for CLI Argument Parsing

Date: 2026-03-09
Plan: `context/plans/migrate-lexopt-to-clap.md`

## Decision

- Replace `lexopt` with `clap` derive macros for all CLI argument parsing.
- Replace manual shell completion scripts with `clap_complete` for auto-generated completions.
- Preserve the existing error-code taxonomy, exit-code contract, and stdout/stderr stream contract.

## Why this path

- **Maintainability**: `clap` derive macros reduce boilerplate and centralize command/option definitions in one schema module (`cli/src/cli_schema.rs`).
- **Auto-generated help**: Clap automatically generates help text from derive attributes, eliminating manual `*_usage_text()` functions in service modules.
- **Auto-generated completions**: `clap_complete` generates shell completions for bash/zsh/fish from the same schema, removing ~175 lines of manual completion script code.
- **Ecosystem alignment**: `clap` is the de-facto standard for Rust CLI applications with active maintenance and extensive documentation.
- **Type safety**: Derive macros provide compile-time validation of command structure and option types.

## Compatibility and risk analysis

- **Backward compatibility**: Command-line interface remains unchanged (same flags, same behavior).
- **Exit codes preserved**: The stable exit-code taxonomy (`2` parse, `3` validation, `4` runtime, `5` dependency) is preserved through explicit error mapping.
- **Error codes preserved**: The `SCE-ERR-*` taxonomy is preserved through custom error formatting.
- **Stream contract preserved**: stdout for payloads, stderr for diagnostics remains unchanged.
- **Dependency footprint**: `clap` adds slightly more compile-time dependencies than `lexopt`, but the trade-off is acceptable given the maintainability benefits.

## Implementation approach

1. Add `clap` (derive feature) and `clap_complete` to dependencies.
2. Create `cli/src/cli_schema.rs` with clap derive structs/enums for all commands.
3. Migrate `cli/src/app.rs` to use clap parsing while preserving error mapping.
4. Remove lexopt-based parsers from service modules.
5. Replace manual completion scripts with `clap_complete` generation.
6. Remove `lexopt` from dependencies.

## Consequences for follow-up tasks

- Context files (`context/overview.md`, `context/glossary.md`, `context/architecture.md`, `context/patterns.md`, `context/cli/cli-command-surface.md`) updated to reference clap instead of lexopt.
- Future CLI commands should be added to `cli/src/cli_schema.rs` using derive macros.
- Completion scripts will automatically include new commands when added to the schema.
