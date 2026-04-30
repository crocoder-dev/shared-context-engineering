# CLI Shell Completion Contract

## Scope

Defines the implemented `sce completion` contract for deterministic shell completion script generation.

## Command surface

- Command: `sce completion --shell <bash|zsh|fish>`
- Help: `sce completion --help`
- Required option: `--shell`
- Supported shells: `bash`, `zsh`, `fish`

## Parsing and validation

- Requires exactly one shell via `--shell <bash|zsh|fish>`.
- Rejects missing shell with deterministic guidance:
  - `Missing required option '--shell <bash|zsh|fish>'. Run 'sce completion --help' to see valid usage.`
- Rejects unknown flags with deterministic guidance:
  - `Unknown completion option '--<name>'. Run 'sce completion --help' to see valid usage.`
- Rejects unexpected positional args with deterministic guidance:
  - `Unexpected completion argument '<value>'. Run 'sce completion --help' to see valid usage.`
- Rejects unsupported shell values with deterministic guidance:
  - `Unsupported shell '<value>'. Valid values: bash, zsh, fish.`

## Output contract

- Output is a shell script payload emitted on stdout for redirection/eval.
- Scripts are deterministic for identical binary + input shell.
- Generated scripts encode current parser-valid command/flag/subcommand surfaces for:
- top-level commands: `help`, `config`, `setup`, `doctor`, `auth`, `hooks`, `trace`, `sync`, `version`, `completion`
  - completion-specific flags and values: `--shell` with `bash|zsh|fish`

## Implementation ownership

- Command parse/dispatch wiring: `cli/src/app.rs`
- Top-level command catalog/help row: `cli/src/command_surface.rs`
- Completion parser/rendering and shell scripts: `cli/src/services/completion/mod.rs`
- Operator docs/install examples: `cli/README.md`

## Verification coverage

- `app::tests` lock completion command routing and help behavior.
- `services::completion::tests` lock parse validation and deterministic script rendering.
- `command_surface::tests` lock top-level help discoverability for `completion`.
