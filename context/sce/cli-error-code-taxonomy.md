# CLI Error-Code Taxonomy

## Scope

This document defines the stable user-facing error-code contract rendered by `sce` runtime diagnostics in `cli/src/app.rs`.
It complements the numeric process exit-code classes documented in `context/sce/cli-exit-code-contract.md`.

## Stable diagnostic code classes

- `SCE-ERR-PARSE`: top-level parse failures before command invocation.
- `SCE-ERR-VALIDATION`: invocation/argument validation failures after parsing.
- `SCE-ERR-RUNTIME`: runtime execution failures after successful parse + validation.
- `SCE-ERR-DEPENDENCY`: startup dependency failures before parsing/dispatch.

## Rendering contract

- User-facing diagnostics are emitted on `stderr` as: `Error [<code>]: <message>`.
- If a diagnostic message does not already include `Try:`, runtime appends class-default remediation guidance.
- If the message already contains `Try:`, runtime preserves the original remediation text and does not append a second one.
- Diagnostic text is still redaction-filtered through `services::security::redact_sensitive_text` before emission.

## Ownership

- `FailureClass` in `cli/src/app.rs` owns class selection.
- `ClassifiedError` in `cli/src/app.rs` owns stable code assignment.
- `write_error_diagnostic` in `cli/src/app.rs` owns final code-bearing stderr rendering.

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output and remediation presence are locked by `app::tests`.
