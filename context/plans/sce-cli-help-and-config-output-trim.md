# Plan: sce-cli-help-and-config-output-trim

## Change summary

Trim the current `sce` CLI help and config command surfaces so operator-facing help no longer advertises implementation-status copy or the temporarily hidden `auth`, `hooks`, `trace`, and `sync` commands, `sce config` behaves as help-first routing, and `sce config validate` reports only pass/fail plus validation errors or warnings.

The target operating model for this plan is:

- `sce`, `sce help`, and `sce --help` show a slimmer command list without implemented/placeholder wording.
- `auth`, `hooks`, `trace`, and `sync` remain implemented in code but are hidden from top-level/local help surfaces for now.
- `sce config` routes to the same help payload as `sce config --help` instead of defaulting to resolved-config inspection.
- `sce config validate` stops emitting resolved-value/provenance dumps and instead reports only validation status plus any errors/warnings in text and JSON modes.

## Success criteria

- Top-level help surfaces no longer show implemented/placeholder copy.
- `auth`, `hooks`, `trace`, and `sync` are hidden from `sce`, `sce help`, and `sce --help` output for this phase.
- The removed setup/config explanatory paragraphs and command-status summary no longer appear in top-level help text.
- Running `sce config` produces the same help payload as `sce config --help`.
- `sce config show` no longer behaves as the default entrypoint for bare `sce config`.
- `sce config validate` text and JSON outputs report only pass/fail plus validation errors/warnings, without resolved values, precedence prose, or provenance/source metadata dumps.
- Help/config tests and durable context are updated to match the new contract.

## Constraints and non-goals

- In scope: top-level help rendering/routing, command-visibility filtering for help surfaces, config command default routing, config validate output-shape reduction, and any directly affected tests/context.
- In scope: preserving existing command implementations while hiding selected commands from help only.
- In scope: keeping `sce config --help` as the canonical bare `sce config` behavior.
- Out of scope: removing or disabling the hidden commands themselves.
- Out of scope: changing `sce config show` semantics when explicitly invoked, unless required only to keep shared help/rendering code coherent.
- Out of scope: redesigning unrelated command-local help text beyond the targeted top-level/help-family cleanup.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Slim top-level help and hide selected commands` (status:done)
  - Task ID: T01
  - Goal: Update the top-level help/help-adjacent rendering path so `sce`, `sce help`, and `sce --help` omit implemented/placeholder labeling, remove the specified setup/config explanatory copy, and hide `auth`, `hooks`, `trace`, and `sync` from those help surfaces.
  - Boundaries (in/out of scope): In - top-level help text generation, command catalog/help filtering, and parser/app tests that assert on top-level help output. Out - removing command implementations, changing explicit command invocation behavior, or broad rewrites of unrelated subcommand help text.
  - Done when: Top-level help no longer contains the removed paragraphs/status summary, the hidden commands are absent from the surfaced command list/examples, and existing direct invocations for those commands remain routable outside help.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test app'`; manual parity review of `sce`, `sce help`, and `sce --help` expected output fixtures/assertions.
  - Completed: 2026-03-31
  - Files changed: `cli/src/command_surface.rs`, `cli/src/services/style.rs`
  - Evidence: `nix flake check` passed.
  - Context sync classification: important change (top-level CLI help contract changed).

- [x] T02: `Make bare sce config route to help` (status:done)
  - Task ID: T02
  - Goal: Change config command routing so bare `sce config` returns the same help/usage payload as `sce config --help`.
  - Boundaries (in/out of scope): In - config command parser/app routing, shared help renderer reuse, and targeted tests for bare/help config behavior. Out - changing explicit `sce config show` or `sce config validate` invocation contracts beyond routing bare `sce config` to help.
  - Done when: `sce config` and `sce config --help` produce the same help payload and targeted tests assert that bare config no longer defaults to `show`.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test config'`; `nix develop -c sh -c 'cd cli && cargo test app'`.
  - Completed: 2026-03-31
  - Files changed: `cli/src/app.rs`
  - Evidence: `nix flake check` passed.
  - Context sync classification: important change (config command routing contract changed).

- [x] T03: `Reduce sce config validate output to pass/fail with errors or warnings` (status:done)
  - Task ID: T03
  - Goal: Simplify `sce config validate` rendering so text/JSON outputs include only validation status plus validation errors/warnings, removing resolved-value, precedence, and provenance reporting from validate mode.
  - Boundaries (in/out of scope): In - validate-mode response model/rendering, JSON schema/shape expectations for tests, and any config-specific docs/context that describe validate output. Out - changing explicit `sce config show` output beyond any shared internal refactor needed to keep validate isolated.
  - Done when: Successful validate output is minimal pass/warn reporting, failing validate output contains only actionable validation issues, JSON output no longer includes resolved/resolved_auth/resolved_observability/resolved_policies payloads, and tests cover both valid and invalid cases.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test config'`; targeted manual review of text and JSON validate expectations in `cli/src/services/config.rs` tests.
  - Completed: 2026-03-31
  - Files changed: `cli/src/services/config.rs`, `cli/src/cli_schema.rs`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Context sync classification: important change (config validate output contract changed).

- [x] T04: `Run validation and sync context for help/config contract changes` (status:done)
  - Task ID: T04
  - Goal: Validate the help/config contract changes end to end, remove any stale context references to implemented/placeholder help copy or old config validate behavior, and confirm durable context matches code truth.
  - Boundaries (in/out of scope): In - repo validation for touched CLI surfaces, context updates required by the new help/config contract, and final cleanup of stale wording. Out - new CLI feature work beyond this help/config trim scope.
  - Done when: Required validation passes, affected durable context reflects the slimmer help/config contract, and no in-scope stale wording remains about top-level implemented/placeholder help labels, bare `sce config` defaulting to show, or `config validate` resolved/provenance dumps.
  - Verification notes (commands or checks): `nix flake check`; manual parity review across `context/overview.md`, `context/cli/placeholder-foundation.md`, `context/cli/config-precedence-contract.md`, `context/sce/cli-observability-contract.md`, and touched CLI tests/help snapshots.
  - Completed: 2026-03-31
  - Files changed: `context/cli/placeholder-foundation.md`, `context/plans/sce-cli-help-and-config-output-trim.md`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed; durable context parity reviewed across `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/placeholder-foundation.md`, `context/cli/config-precedence-contract.md`, and `context/sce/cli-observability-contract.md`.
  - Context sync classification: important change already documented by prior tasks; final T04 sync pass verified root shared files against code truth and updated the remaining stale domain-context wording in `context/cli/placeholder-foundation.md`.

## Open questions

- None. The user confirmed that hiding applies to `sce`, `sce help`, and `sce --help`; bare `sce config` should behave the same as `sce config --help`; and `sce config validate` should report only pass/fail plus validation errors/warnings.

## Validation Report

### Commands run

- `nix flake check` -> exit 0
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)

### Temporary scaffolding

- None introduced for T04.

### Success-criteria verification

- [x] Top-level help surfaces no longer show implemented/placeholder copy -> verified in existing code/context contract (`cli/src/command_surface.rs`, `context/overview.md`, `context/cli/placeholder-foundation.md`)
- [x] `auth`, `hooks`, `trace`, and `sync` are hidden from `sce`, `sce help`, and `sce --help` output for this phase -> verified in `cli/src/command_surface.rs` and matching durable context
- [x] Removed setup/config explanatory paragraphs and command-status summary no longer appear in top-level help text -> covered by `cli/src/command_surface.rs` tests and durable context review
- [x] Running `sce config` produces the same help payload as `sce config --help` -> verified in `cli/src/app.rs` and matching context
- [x] `sce config show` no longer behaves as the default entrypoint for bare `sce config` -> verified in `cli/src/app.rs` and matching context
- [x] `sce config validate` text and JSON outputs report only pass/fail plus validation errors/warnings -> verified in `cli/src/services/config.rs`, `cli/src/cli_schema.rs`, and matching context
- [x] Help/config tests and durable context are updated to match the new contract -> confirmed by `nix flake check`, `nix run .#pkl-check-generated`, and final context review/update in `context/cli/placeholder-foundation.md`

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for the scoped help/config trim contract.
