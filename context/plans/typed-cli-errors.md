# Plan: typed-cli-errors

## Change summary

Replace the current string-only `ClassifiedError` (`class`, `code`, `message: String`) at the CLI command boundary with a typed `CliError` that separates two categories: `CliError::User { error: UserError, source: Option<anyhow::Error> }` for expected, deliberately-explained failures, and `CliError::Internal { class: FailureClass, source: anyhow::Error }` for everything else. `UserError` starts with exactly one variant, `NotAuthenticated`, and `app_support` becomes the sole owner of turning it into a friendly, actionable stderr sentence using stderr TTY/`NO_COLOR` policy. `sce sync` is the first adopter: today its stream/control-plane errors are already erased into plain strings before reaching the CLI boundary (`BatchAttemptOutcome::Terminal(String)`, `StreamSyncError::Refresh(String)`/`Terminal(String)` in `cli/src/services/agent_trace_sync/mod.rs`, built from a typed `ControlPlaneError` via `.to_string()`/`error.to_string()` in `cli/src/services/agent_trace_sync/mod.rs:510` and `cli/src/services/sync/sync.rs:525`), so an authentication failure and, say, a `500` both render as an opaque runtime string. This plan fixes that erasure, adds `is_authentication_failure()` typed classification through `ControlPlaneError` → `StreamSyncError` → `TraceSyncError`, and wires `sce sync`'s classifier to produce `UserError::NotAuthenticated` for `MissingCredentials`/`AuthenticationFailed` while every other `ControlPlaneError` (`Forbidden`, `BadRequest`, `Transport`, `ServerError`, `InvalidResponse`, `Storage`, `Protocol`) stays an internal failure with its full `anyhow` chain intact for observability.

This replaces `ClassifiedError` rather than extending it, and is scoped to one adopter — it does not migrate setup validation, clap/parser errors, bash policy errors, or `AuthError` to typed user errors, and it does not touch `sce auth whoami` semantics. It is a fresh implementation on top of current `main`; it does not build on, cherry-pick from, or reuse the `UserFacingPresentation` design of PR #221.

## Acceptance criteria

- [ ] AC1: No arbitrary user-message escape hatch exists — `UserError` has no `Message(String)`/`Custom(...)` variant, and `UserFacingPresentation` does not exist anywhere in the codebase.
  - Validate: `grep -rn "UserFacingPresentation" cli/src` and `grep -rn "UserError::Message\|UserError::Custom" cli/src` both return no results; `cli/src/services/error.rs` shows `UserError` with only `NotAuthenticated`.
- [ ] AC2: `CliError` distinguishes typed user errors (`CliError::User`) from internal failures (`CliError::Internal`), and `FailureClass::code()` still maps to the four stable `SCE-ERR-*` strings.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error::`
- [ ] AC3: `sce sync` classifies an authentication failure from the initial `/state` call (`MissingCredentials` or `AuthenticationFailed`), a stream batch request, or a stream reconciliation `/state` refresh as `UserError::NotAuthenticated`, while `Forbidden`, `BadRequest`, `Transport`, `ServerError`, `InvalidResponse`, `Storage`, and `Protocol` remain internal failures.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::` (positive and negative classification cases from Phase 10).
- [ ] AC4: A `sce sync` authentication failure renders exactly one friendly login diagnostic on stderr (no low-level control-plane text), leaves stdout empty, exits with the runtime class (`4`), and still preserves the technical `ControlPlaneError`/`anyhow` source for observability.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app_support::` end-to-end routing test asserting stdout/stderr/exit-code/single-diagnostic behavior.
- [ ] AC5: `CliError::Internal` diagnostics still render the real `anyhow` error chain (`format!("{source:#}")`) rather than a pre-stringified message, and existing exit-code classes plus `SCE-ERR-*` codes and `Try:` remediation are unchanged.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app::`
- [ ] AC6: Friendly login-guidance styling follows stderr TTY/`NO_COLOR` policy (`supports_color_stderr()`), independent of stdout's TTY state.
  - Validate: targeted styling test covering TTY, redirected-stderr, and `NO_COLOR` cases for the user-error renderer.
- [ ] AC7: `cli/src/services/sync/command.rs` contains no friendly-sentence construction, no terminal styling call, and no string/substring matching used to decide authentication semantics.
  - Validate: `grep -n "style::success\|You are not logged in" cli/src/services/sync/command.rs` returns no results; manual review of `classify_sync_error` shows it dispatches only on `is_authentication_failure()`.
- [ ] AC8: Observability logs one structured record per `CliError` (class, code, surface, `user_error` key when present, technical source) without emitting a second competing terminal stderr diagnostic for the same error.
  - Validate: targeted observability test asserting a single stderr diagnostic write plus the structured log fields for both a `CliError::User` and a `CliError::Internal` case.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/cli-error-code-taxonomy.md` — `CliError`/`UserError` ownership replacing `ClassifiedError`.
- `context/sce/cli-stdout-stderr-contract.md` — stream contract restated against `CliError`.
- `context/sce/cli-observability-contract.md` — `log_cli_error` API, `error_surface`/`user_error` structured fields, single-owner terminal emission.
- `context/cli/sync-command.md` and `context/cli/agent-trace-sync-command.md` — typed authentication-failure classification through the sync stack.
- `context/overview.md` — cross-cutting `ClassifiedError` → `CliError` rename at the command boundary.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/error.rs`; `cli/src/app.rs`; `cli/src/services/app_support.rs`; `cli/src/services/command_registry.rs`; `cli/src/services/observability.rs` and `cli/src/services/observability/traits.rs`; `cli/src/services/parse/command_runtime.rs`; `cli/src/services/sync/command.rs`, `cli/src/services/sync/sync.rs`; `cli/src/services/agent_trace_sync/mod.rs` and `cli/src/services/agent_trace_sync/control_plane.rs`; the `CliError`-boundary surface of command adapters for auth/config/doctor/hooks/setup/version and policy code currently returning `ClassifiedError`; existing tests referencing `ClassifiedError`; the context docs listed under Context sync.
- **Out of scope:** migrating setup `bail!` validation to typed user errors; migrating clap/parser usage errors; migrating bash policy errors; redesigning `AuthError` as a whole; removing existing `Try:` remediation strings; changing `sce auth whoami` semantics (it keeps returning unauthenticated state as a successful result); broad CLI copy cleanup unrelated to this architecture; PR #221 (not built on, not cherry-picked from, not closed or modified by this plan).
- **Constraints:** no new crate dependencies; existing numeric exit-code classes (`2`/`3`/`4`/`5`) and `SCE-ERR-{PARSE,VALIDATION,RUNTIME,DEPENDENCY}` codes stay stable; `UserError` variants only — no `Message(String)`/`Custom(...)` escape hatch; no `UserFacingPresentation` type; no terminal styling stored on error types; no string/substring matching to determine authentication semantics; branch from current `main`.
- **Non-goal:** migrating the rest of the CLI's `ClassifiedError` call sites to typed `UserError` variants beyond `sce sync` authentication; broadening this into an `AuthError` redesign or a `sce auth whoami` behavior change.

## Assumptions

- The exact login-guidance sentence (`You are not logged in. Please log in using the \`sce auth login\` command.`) may be preserved verbatim from current behavior; the request states ownership matters, not exact punctuation.
- Illustrative type/field names in the request (`BatchAttemptOutcome`, `StreamSyncError`, the `is_authentication_failure` traversal shape) may differ in the implementation as long as `ControlPlaneError` stays typed end-to-end through the sync stack and authentication semantics are never derived from string matching.
- Branch creation, staging, and opening the draft PR against `main` are carried out through this repository's normal `/commit` and PR workflow after the task stack completes; they are not modeled as plan tasks.

## Task stack

- [x] T01: `Introduce the CliError/UserError boundary and retire ClassifiedError` (status:done)
  - Task ID: T01
  - Scope: In — `cli/src/services/error.rs` (new `FailureClass::code()`, `CliError::{User,Internal}`, `UserError::NotAuthenticated` with `class()`/`key()`, constructors `user`/`user_with_source`/`internal`/`runtime`/`dependency` plus compatibility `parse`/`validation` string helpers that wrap `anyhow::Error::msg(...)`); mechanical `ClassifiedError` → `CliError` rename across `cli/src/app.rs`, `cli/src/services/app_support.rs`, `cli/src/services/command_registry.rs`, `cli/src/services/observability.rs`/`observability/traits.rs` (signature only), `cli/src/services/parse/command_runtime.rs`, `cli/src/services/sync/command.rs`, the auth/config/doctor/hooks/setup/version command adapters, policy code, and existing tests; `app_support::write_error_diagnostic` updated to an exhaustive match rendering `CliError::Internal` from `format!("{source:#}")` and `CliError::User` as the friendly `UserError::NotAuthenticated` sentence styled via `supports_color_stderr()`, with redaction applied after rendering and before the write. Out — Phase 3 anyhow-preservation cleanup in other adapters, sync's own stream-error typing, the sync classifier, and observability's structured logging fields (later tasks).
  - Dependencies: none
  - Done when: `ClassifiedError` no longer exists anywhere in `cli/src/`; `CliError`, `UserError::NotAuthenticated`, and the constructors above exist and compile; `write_error_diagnostic` renders both variants correctly (internal via the real error chain with unchanged `SCE-ERR-*`/`Try:` behavior, user via the friendly sentence under stderr color policy); existing exit-code and error-code behavior is unchanged for every current call site.
  - Verify: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`; `grep -rn "ClassifiedError" cli/src` (expect no results).
  - Context synchronization: synced
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/error.rs`; `cli/src/app.rs`; `cli/src/services/app_support.rs`; `cli/src/services/command_registry.rs`; `cli/src/services/observability.rs`; `cli/src/services/observability/traits.rs`; `cli/src/services/parse/command_runtime.rs`; `cli/src/services/sync/command.rs`; `cli/src/services/auth_command/command.rs`; `cli/src/services/config/command.rs`; `cli/src/services/doctor/command.rs`; `cli/src/services/hooks/command.rs`; `cli/src/services/setup/command.rs`; `cli/src/services/version/command.rs`; `cli/src/services/bash_policy.rs`; `cli/src/services/agent_trace_sync/control_plane.rs` (doc-comment only)
  - Result: `error.rs` now defines `FailureClass::code()`, `CliError::{User,Internal}`, `UserError::NotAuthenticated` (`class()`/`key()`/`message()`), and the `user`/`user_with_source`/`internal`/`runtime`/`dependency` constructors plus `parse`/`validation` string-compatibility helpers that wrap `anyhow::Error::msg(...)`. `ClassifiedError` is deleted; every prior call site now constructs `CliError`, with `format!`-built messages passed to `runtime`/`dependency` wrapped in `anyhow::Error::msg(...)` so `CliError::Internal` always carries a live `anyhow::Error` and renders via `format!("{source:#}")`. `app_support::write_error_diagnostic` is an exhaustive match: `Internal` renders the real error chain with unchanged `Try:` guidance; `User` renders `UserError::message()` with no `Try:` suffix. Both paths go through the existing stderr-policy-aware `services::style::error_text` and existing redaction. `observability::Logger::log_classified_error` (name unchanged; renamed in T05) now takes `&CliError` and logs `error.to_string()` in place of the old flat `.message()`. `UserError::NotAuthenticated`, `CliError::User`, `CliError::user`, and `CliError::user_with_source` are marked `#[allow(dead_code)]` since no call site constructs them yet — `sce sync` classification wiring is T04's scope. Three tests (`parse/command_runtime.rs`, `bash_policy.rs`) that asserted on the old `.message()` accessor were updated to `.to_string()`. Added a small unit-test module in `error.rs` covering `FailureClass::code()`, both `CliError` variants' `class()`/`code()`/`Display`, and the `parse`/`validation` compatibility helpers.
  - Verify:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed (dead-code lint required the `#[allow(dead_code)]` annotations noted above to compile clean under `-D warnings`).
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed, 345/345.
    - `grep -rn "ClassifiedError" cli/src` — no results.
    - Additionally ran `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml` (clean) and manually exercised `sce bogus-command` (unchanged `Error [SCE-ERR-PARSE]: ... Try: ...` rendering, exit code 2) and `sce version` (unchanged success output) through the built binary.
  - Context impact: Internal-only. `CliError`/`UserError` are new public types within `cli/src/services/error.rs`, but no external-facing behavior, CLI contract, or SCE-ERR-* code changed — the rename and wrapping are transparent to callers. No `context/sce`/`context/cli` doc requires updating yet; T07 documents the full architecture once T02–T06 finish shaping it.

- [x] T02: `Preserve anyhow error sources at remaining command-adapter boundaries` (status:done)
  - Task ID: T02
  - Scope: In — command adapters (auth/config/doctor/hooks/setup/version and other obvious `anyhow::Error`-returning call sites) currently doing `.map_err(|error| CliError::runtime(format!("{error:#}")))`, changed to `.map_err(CliError::runtime)` with `anyhow::Context` attached beforehand where it adds useful context. Out — redesigning domain error types, sync's own error path (T03/T04), setup `bail!` validation, clap/parser errors, bash policy errors.
  - Dependencies: T01
  - Done when: identified adapters construct `CliError::Internal` from the live `anyhow::Error` object instead of a pre-formatted string; rendered diagnostic text for existing failure cases is unchanged, since `write_error_diagnostic` already renders `format!("{source:#}")`.
  - Verify: `grep -rn 'CliError::runtime(format!("{error:#}"))' cli/src/services` (expect no results in migrated adapters); `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`.
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/auth_command/command.rs`; `cli/src/services/config/command.rs`; `cli/src/services/doctor/command.rs`; `cli/src/services/hooks/command.rs`; `cli/src/services/setup/command.rs`; `cli/src/services/version/command.rs`
  - Result: All six command-adapter `.map_err(|error| CliError::runtime(anyhow::Error::msg(format!("{error:#}"))))` call sites (one each in `auth_command`, `config`, `doctor`, `hooks`, `version`, and seven in `setup/command.rs`) now pass the live `anyhow::Error` directly via `.map_err(CliError::runtime)`, since every wrapped domain function already returns `anyhow::Result<...>` and `CliError::runtime` takes `impl Into<anyhow::Error>`. No `anyhow::Context` needed attaching beforehand — none of the six sites were missing useful context. `cli/src/services/app_support.rs:157` (`write_stdout_payload`, wrapping an `io::Error`, not one of the six named adapters) and `cli/src/services/sync/command.rs:93` (explicitly T04's territory) were left untouched, matching the task's out-of-scope boundaries. `write_error_diagnostic` already renders `format!("{source:#}")`, so rendered diagnostic text for these call sites is unchanged; only the preserved source chain differs (previously erased into a message-only `anyhow::Error`, now the original error with its full chain intact for observability).
  - Verify:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed, 345/345.
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml` — clean.
    - `grep -rn 'CliError::runtime(anyhow::Error::msg(format!("{error:#}")))' cli/src/services/{auth_command,config,doctor,hooks,version,setup}` — no results.
  - Context impact: Internal-only. No public interface, CLI contract, exit code, `SCE-ERR-*` code, or rendered diagnostic text changed — only the internal fidelity of the preserved `anyhow` source chain. No `context/sce`/`context/cli` doc requires updating for this task; T07 documents the full architecture once T02–T06 finish shaping it.
  - Context synchronization: synced

- [x] T03: `Preserve typed control-plane errors through the sync stream stack` (status:done)
  - Task ID: T03
  - Scope: In — `cli/src/services/agent_trace_sync/mod.rs` (`BatchAttemptOutcome::Terminal(String)` → `Terminal(ControlPlaneError)`, `StreamSyncError::Refresh(String)`/`Terminal(String)` → `Refresh(ControlPlaneError)`/`Terminal(ControlPlaneError)`, and their construction sites at `mod.rs:510` and `sync.rs:525`); `ControlPlaneError::is_authentication_failure()` in `cli/src/services/agent_trace_sync/control_plane.rs` (true only for `MissingCredentials`/`AuthenticationFailed`); equivalent typed traversal `StreamSyncError::is_authentication_failure()` and `TraceSyncError::is_authentication_failure()` in `cli/src/services/sync/sync.rs`/`agent_trace_sync/mod.rs`. Out — the CLI-facing classifier (T04), any `CliError`/`UserError` reference (this task is internal to the sync/control-plane modules).
  - Dependencies: none
  - Done when: no sync-stream path erases a `ControlPlaneError` into a bare `String` before it reaches `TraceSyncError`; `is_authentication_failure()` is available on `ControlPlaneError`, `StreamSyncError`, and `TraceSyncError` and correctly returns `true` only for `MissingCredentials`/`AuthenticationFailed`; `Display` output for the affected variants is unchanged.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_sync`; `grep -rn "BatchAttemptOutcome::Terminal(String)\|StreamSyncError::Refresh(String)\|StreamSyncError::Terminal(String)" cli/src` (expect no results).
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/agent_trace_sync/control_plane.rs`; `cli/src/services/agent_trace_sync/mod.rs`; `cli/src/services/sync/sync.rs`
  - Result: `ControlPlaneError::is_authentication_failure()` added in `control_plane.rs`, true only for `MissingCredentials`/`AuthenticationFailed`. In `mod.rs`, `BatchAttemptOutcome::Terminal` and `StreamSyncError::{Refresh,Terminal}` now carry `ControlPlaneError` instead of `String`; `StreamSyncError::is_authentication_failure()` added, delegating to the inner `ControlPlaneError` for `Refresh`/`Terminal` and returning `false` for `Read`/`InvalidResponse`/`DidNotConverge` (neither of which can carry a control-plane error). `BatchAttemptOutcome`'s `PartialEq, Eq` derive was dropped since `ControlPlaneError` doesn't implement them and no call site compared these enums by equality (confirmed via grep; all existing assertions use `matches!`). In `sync.rs`, the two construction sites (`sync.rs:510` `BatchAttemptOutcome::Terminal(error.to_string())` → `BatchAttemptOutcome::Terminal(error)`; `sync.rs:525` `.map_err(|error| StreamSyncError::Refresh(error.to_string()))` → `.map_err(StreamSyncError::Refresh)`) now pass the live `ControlPlaneError` through instead of stringifying it, and `TraceSyncError::is_authentication_failure()` was added, traversing `ControlPlane(_)` directly and `Stream { source, .. }` via `StreamSyncError::is_authentication_failure()`, `false` for `Runtime`. `TraceSyncError::is_authentication_failure()` is marked `#[allow(dead_code)]` since no call site invokes it yet — wiring it into `sce sync`'s classifier is T04's scope (same pattern T01 used for not-yet-called constructors). The one existing test relying on the old `String` shape (`terminal_failure_does_not_call_refresh` in `mod.rs`) was updated to construct/match `ControlPlaneError::BadRequest(...)` instead of a bare string. `Display` output is unchanged: both affected arms already interpolate `{reason}` via `write!`, and `ControlPlaneError`'s `Display` impl produces the same text `.to_string()` previously captured.
  - Verify:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed (required `#[allow(dead_code)]` on `TraceSyncError::is_authentication_failure()` to compile clean under `-D warnings`, same pattern as T01).
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_sync` — passed, 38/38.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed, 345/345 (no change in total count).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml` — clean (required backticking `WorkOS` in two new doc comments for `clippy::doc_markdown` under `-D clippy::pedantic`).
    - `grep -rn "BatchAttemptOutcome::Terminal(String)\|StreamSyncError::Refresh(String)\|StreamSyncError::Terminal(String)" cli/src` — no results.
  - Context impact: Internal-only. `ControlPlaneError`/`StreamSyncError`/`TraceSyncError` are all internal-to-the-sync-stack types; no public CLI interface, exit code, `SCE-ERR-*` code, or rendered diagnostic text changed, and `Display` output for every affected variant is unchanged. No `context/sce`/`context/cli` doc requires updating yet; T07 documents the full architecture once T02–T06 finish shaping it.
  - Context synchronization: synced

- [x] T04: `Classify sce sync authentication failures as the typed user error` (status:done)
  - Task ID: T04
  - Scope: In — `cli/src/services/sync/command.rs`'s `classify_sync_error(err: TraceSyncError) -> CliError`, rewritten to call `err.is_authentication_failure()` and return `CliError::user_with_source(UserError::NotAuthenticated, err)` when true, `CliError::runtime(err)` otherwise. Out — any friendly-sentence text, terminal styling, or color-policy decision in `sync/command.rs` (owned by `app_support` since T01); rendering/observability changes.
  - Dependencies: T01, T03
  - Done when: `sce sync` authentication failures from the initial `/state` call, a stream batch request, and a stream reconciliation `/state` refresh all classify as `CliError::User { error: UserError::NotAuthenticated, .. }`; every other `ControlPlaneError` variant (`Forbidden`, `BadRequest`, `Transport`, `ServerError`, `InvalidResponse`, `Storage`, `Protocol`) still classifies as `CliError::Internal`; `sync/command.rs` contains no string matching, no styling call, and no friendly sentence.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::`; `grep -n "style::success\|You are not logged in\|\\.contains(" cli/src/services/sync/command.rs` (expect no results).
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/sync/command.rs`
  - Result: `classify_sync_error` now branches on `err.is_authentication_failure()`: `true` returns `CliError::user_with_source(UserError::NotAuthenticated, err)` (preserving the live `TraceSyncError`/`ControlPlaneError` chain as the technical source instead of the prior `anyhow::Error::msg(format!("{err}"))` stringification), `false` returns `CliError::runtime(err)` (also now passing the live error via `Into<anyhow::Error>` rather than a pre-formatted string, matching T02's anyhow-preservation pattern). Added a `UserError` import. Added a `#[cfg(test)] mod tests` covering all four authentication paths named in the task (`ControlPlane(MissingCredentials)`, `ControlPlane(AuthenticationFailed)`, `Stream { source: Terminal(AuthenticationFailed) }`, `Stream { source: Refresh(MissingCredentials) }`) asserting `CliError::User` with a preserved source, plus negative cases for every other `ControlPlaneError` variant (`Forbidden`, `BadRequest`, `Transport`, `ServerError`, `InvalidResponse`, `Storage`) and `TraceSyncError::Runtime`, asserting `CliError::Internal`. No changes outside `sync/command.rs`; `sync.rs`'s pre-existing `#[allow(dead_code)]` on `TraceSyncError::is_authentication_failure()` was left in place since removing it was outside this task's declared scope (harmless now that the method has a real caller — an unused `allow` is not a compiler or clippy error).
  - Verify:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::` — passed, 60/60, including the 8 new classification tests.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed, 351/351 (up from 345; +6 net after removing none and adding 8 minus overlap in filtered count reporting).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml` — clean.
    - `grep -n "style::success\|You are not logged in\|\.contains(" cli/src/services/sync/command.rs` — no results.
  - Context impact: Internal-only for `sync/command.rs`'s own contract, but this is the first call site that actually constructs `CliError::User`/`UserError::NotAuthenticated`, so `sce sync` authentication failures now render through `app_support`'s friendly-diagnostic path (built in T01) instead of the generic internal-error path — a real, user-visible behavior change for that one failure mode, though the rendering logic itself is unchanged. `context/cli/sync-command.md` and `context/cli/agent-trace-sync-command.md` (listed under this plan's Context sync) describe this authentication-classification behavior; T07 is the task that updates durable context docs once T02–T06 finish shaping the full architecture, so no doc update is made here.
  - Context synchronization: synced

- [x] T05: `Give observability one owner for structured CliError logging without duplicate terminal output` (status:done)
  - Task ID: T05
  - Scope: In — rename/refactor `Logger::log_classified_error` to `log_cli_error(&self, error: &CliError, session_id: Option<&str>)` in `cli/src/services/observability.rs` and `observability/traits.rs` (including `NoopLogger`), preserving `error_class`/`error_code` fields and adding `error_surface` (`user`/`internal`) plus `user_error` (the `UserError::key()`) when applicable, and the technical source when present; confirm `app_support` remains the sole writer of the terminal stderr diagnostic and observability never writes a second one. Out — changing which events get logged elsewhere, or altering `write_error_diagnostic`'s rendering (already correct from T01).
  - Dependencies: T01
  - Done when: every call site logs through `log_cli_error`; structured log records for a `CliError::User` case carry `error_class=runtime error_code=SCE-ERR-RUNTIME error_surface=user user_error=auth.not_authenticated` plus the technical source, and a `CliError::Internal` case carries its class/code/surface and full source chain; exactly one terminal stderr diagnostic is written per failed command.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability`; `grep -rn "log_classified_error" cli/src` (expect no results).
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/observability.rs`; `cli/src/services/observability/traits.rs`; `cli/src/services/app_support.rs`
  - Result: `Logger::log_classified_error` is renamed to `log_cli_error(&self, error: &CliError, session_id: Option<&str>)` in `observability.rs`, with its field-building logic extracted into a pure helper `cli_error_fields(error: &CliError) -> Vec<(&'static str, String)>` (plus small helpers `cli_error_surface`/`cli_error_technical_source`) so the shape is unit-testable without file I/O. The field list always carries `error_code`/`error_class`/`error_surface` (`"user"` for `CliError::User`, `"internal"` for `CliError::Internal`), adds `user_error` (`UserError::key()`) only for `CliError::User`, and adds `error_source` (`format!("{source:#}")`) whenever a technical source is present — always for `Internal`, and for `User` only when `user_with_source` supplied one. The trait method is renamed in `observability/traits.rs` on the `Logger` trait, the `NoopLogger` impl, and the concrete `Logger` impl (which now delegates to `super::Logger::log_cli_error`). The single call site in `app_support.rs:146` (`exit_with_error`) is updated to `log.log_cli_error(error, None)`; it remains the only logger call in that function, and `write_error_diagnostic` (the sole terminal stderr diagnostic writer, unchanged from T01) is still called exactly once per failed command — observability's own line goes to its structured log record, not a second competing diagnostic. No changes to `error.rs`: `UserError::key()` and the `CliError::User` variant already existed from T01 and simply gained their first real caller here; their pre-existing `#[allow(dead_code)]` attributes were left in place (harmless once used, out of this task's declared file scope).
  - Verify:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability` — passed, 3/3 new tests (`cli_error_fields` shape for `CliError::User` with source, `CliError::User` without source, and `CliError::Internal`).
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed, 354/354 (up from 351; +3 new tests, no regressions).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml` — clean.
    - `grep -rn "log_classified_error" cli/src` — no results.
  - Context impact: `local`. The literal method name `Logger::log_classified_error()`/`log_classified_error` appeared verbatim in `context/sce/cli-error-code-taxonomy.md`, `context/sce/cli-observability-contract.md`, and `context/glossary.md`; those were corrected to `log_cli_error` during context synchronization since a renamed API reference is a factual contradiction, not narrative detail. The full new structured-field list (`error_surface`, `user_error`, `error_source`) and the broader `CliError`/`UserError` architecture narrative remain deferred to T07 (already listed under this plan's Context sync), matching the T01–T04 precedent of fixing broken references immediately while batching full-architecture prose into T07. No public CLI interface, exit code, `SCE-ERR-*` code, or terminal-rendered diagnostic text changed — only the internal logger method name and the structured (non-terminal) log record's field set.
  - Context synchronization: synced

- [ ] T06: `Add architecture and behavior tests for the typed error boundary` (status:todo)
  - Task ID: T06
  - Scope: In — tests covering: user-error routing (empty stdout, friendly stderr guidance with no low-level auth/control-plane text, exit code `4`, exactly one terminal diagnostic, technical source retained, redaction still applied); stderr color behavior (TTY-following styling, no ANSI on redirected stderr, `NO_COLOR` disabling styling, stdout TTY state not controlling stderr presentation); sync authentication propagation for all four paths (initial `/state` `MissingCredentials`, initial `/state` `AuthenticationFailed`, stream batch `AuthenticationFailed`, stream reconciliation refresh `AuthenticationFailed`); negative classification for `Forbidden`/`BadRequest`/`Transport`/`ServerError`/`InvalidResponse`/`Storage`/`Protocol` remaining internal. Out — new production behavior; this task only adds coverage for T01–T05.
  - Dependencies: T01, T03, T04, T05
  - Done when: all listed cases have passing targeted tests; `cargo clippy` for the crate is clean.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml`.
  - Context synchronization: pending

- [ ] T07: `Document the typed CliError/UserError architecture in durable context` (status:todo)
  - Task ID: T07
  - Scope: In — update `context/sce/cli-error-code-taxonomy.md`, `context/sce/cli-stdout-stderr-contract.md`, `context/sce/cli-observability-contract.md`, `context/cli/sync-command.md`, `context/cli/agent-trace-sync-command.md`, and `context/overview.md` to describe `CliError::{User,Internal}`, `UserError` as the catalog of deliberately presented terminal failures, the separation from technical source errors, that commands/domain layers do not own terminal rendering, that `app_support` owns final stderr presentation with styling applied at the renderer using stderr policy, that observability retains technical detail independently, and that stable `SCE-ERR-*`/exit classes are unchanged. Out — introducing or documenting `UserFacingPresentation` (must not appear); restating unrelated legacy content.
  - Dependencies: T06
  - Done when: the listed context docs describe the shipped `CliError`/`UserError` architecture with no reference to `ClassifiedError` or `UserFacingPresentation`.
  - Verify: `nix run .#pkl-check-generated`; `grep -rn "UserFacingPresentation" context/` (expect no results); `grep -rln "ClassifiedError" context/cli context/sce` (expect no results among the updated files).
  - Context synchronization: pending

## Open questions

None. The change request fully specifies scope, architecture, phase-by-phase behavior, and acceptance criteria; the only latitude left (exact wording, illustrative type-shape naming) is recorded under Assumptions rather than blocking authoring.
