# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic log level/format controls and event emission boundaries in `cli/src/services/observability.rs` and `cli/src/app.rs`.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- Defaults are deterministic: `SCE_LOG_LEVEL=info` and `SCE_LOG_FORMAT=text` when env keys are unset.
- Invalid observability env values fail invocation validation with actionable error text.

## Emission contract

- Log output is emitted to `stderr` only; command result payloads remain on `stdout`.
- Each emitted record includes a stable `event_id`.
- Current app-level event identifiers:
  - `sce.app.start`
  - `sce.command.parsed`
  - `sce.command.completed`
  - `sce.command.parse_failed`
  - `sce.command.failed`
- Event records include deterministic metadata keys used by automation (`command`, `failure_class`, `component` when applicable).

## Format contract

- `text` format emits single-line key/value records with fixed key ordering: `log_format`, `level`, `event_id`, `message`, then optional fields.
- `json` format emits a single-line object with fixed top-level keys: `log_format`, `level`, `event_id`, `message`, `fields`.
- Logger threshold behavior is deterministic and severity-based (`error < warn < info < debug`).

## Ownership and verification

- `cli/src/services/observability.rs` owns env parsing, level filtering, and record rendering.
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths.
- Contract behavior is covered by `services::observability::tests` and exercised in end-to-end app command tests.
