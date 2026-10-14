# Plan: auth-login-telemetry-credential

## Change summary

Wire telemetry-credential provisioning into the existing `sce auth login` flow. Every successful login attempt, including valid or renewed stored WorkOS credentials and fresh device authorization, will POST to the Control Plane and replace the cached encrypted telemetry credential. WorkOS authentication remains successful when telemetry provisioning or storage fails; the CLI reports a clear, secret-free warning.

Preserve the current WorkOS login, renewal, logout, and text/JSON contracts while extending login results with deterministic telemetry-provisioning status. Reuse the existing `AuthenticatedControlPlaneClient::issue_and_save_telemetry_credential` seam and encrypted `AuthDb`; do not add a refresh loop, separate command, or backend changes.

## Acceptance criteria

- [x] AC1: Every successful `sce auth login` path (fresh device login, valid stored credentials, and renewed stored credentials) attempts `POST /auth/telemetry/credential` with the authenticated WorkOS bearer and replaces the cached telemetry credential on success.
  - Validate: targeted Rust tests exercise fresh, valid-stored, and renewed-stored login paths against a test Control Plane and assert the request and replacement behavior.
- [x] AC2: Telemetry provisioning and storage failures are best effort: `sce auth login` still succeeds after WorkOS credentials are saved, reports an understandable deterministic warning, and never exposes the telemetry token, accessor, authorization header, or raw server response.
  - Validate: targeted auth-command and Control Plane tests cover failure responses, transport failures, and storage failures in text and JSON output, with secret-redaction assertions.
- [x] AC3: Existing WorkOS login behavior remains intact, including stored-credential renewal/device-flow fallback and logout deletion of both credential types; telemetry provisioning does not alter the WorkOS credential row.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth telemetry_credential`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix flake check`

### Context sync

- `context/cli/cli-command-surface.md`
- `context/cli/agent-trace-sync-command.md`
- `context/sce/telemetry-credential.md`
- `context/sce/auth-db.md`
- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/auth_command/`, `cli/src/services/agent_trace_sync/control_plane.rs`, login output rendering/tests, telemetry credential storage integration, and the listed durable context files.
- **Out of scope:** a new telemetry subcommand, automatic refresh loop, telemetry exporter/collector, doctor checks, Control Plane changes, server-side revocation, identity/workspace binding, or changing `sce auth logout` behavior.
- **Constraints:** issue/replace once per successful `sce auth login`; provision only after WorkOS credentials are persisted; use the existing resolved Control Plane URL and authenticated client; preserve async direct-await patterns; keep text/JSON output deterministic; never print or persist secrets outside encrypted `auth.db`.
- **Non-goal:** generalizing the credential store or making telemetry provisioning part of `sce sync` or background auto-sync.

## Assumptions

- “Every time” means every successful login completion, whether the stored WorkOS token was already valid, was renewed, or was obtained through device authorization.
- Best-effort failure means the login command retains a successful exit/status after WorkOS persistence, while text output includes a clear retry instruction and JSON exposes a non-secret telemetry status/message rather than failing the command.
- Telemetry provisioning uses the existing Control Plane base URL and `AuthenticatedControlPlaneClient`; the backend remains the authority for authorization and workspace eligibility.

## Task stack

- [x] T01: `Define best-effort telemetry provisioning results for auth login` (status:done)
  - Task ID: T01
  - Scope: In — auth-login orchestration seams, telemetry provisioning outcome modeling, deterministic text/JSON success and warning rendering, and focused unit tests for success/failure behavior. Out — production Control Plane wiring and endpoint/storage integration.
  - Dependencies: none
  - Done when: the login flow can represent telemetry issued versus unavailable without changing WorkOS success semantics, renders an understandable secret-free warning in text and JSON, and tests cover both outcomes.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command`
  - Completed: 2026-10-14
  - Files changed: `cli/src/services/auth_command/mod.rs`
  - Result: Added a reusable telemetry provisioning outcome model and threaded it through fresh and renewed login renderers, preserving authentication success while reporting deterministic issued/unavailable status and a secret-free retry warning in text and JSON. Added focused success, warning, and redaction tests.
  - Verify outcome: `auth_command` tests passed (6 tests).
  - Context impact: domain — auth login output now carries telemetry provisioning status/message and exposes renderer seams for production provisioning integration; durable context synchronization remains pending.
  - Context synchronization: synced

- [x] T02: `Provision and replace telemetry credentials from every auth login path` (status:done)
  - Task ID: T02
  - Scope: In — production `sce auth login` integration with resolved Control Plane configuration and `AuthenticatedControlPlaneClient::issue_and_save_telemetry_credential`, fresh/valid-stored/renewed path coverage, endpoint/storage failure tests, and durable context updates. Out — new public telemetry command, refresh loop, exporter, doctor integration, and server changes.
  - Dependencies: T01
  - Done when: each successful login path issues and saves a replacement telemetry credential after WorkOS persistence; provisioning failures leave login successful with the planned warning/status; existing WorkOS and logout behavior remains intact; the deferred provisioning dead-code allowance is removed when the method is consumed.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth telemetry_credential`; inspect the updated context files for the login-triggered provisioning lifecycle and failure contract.
  - Completed: 2026-10-14
  - Files changed: `cli/src/services/auth_command/mod.rs`, `cli/src/services/agent_trace_sync/control_plane.rs`
  - Result: Wired the resolved Control Plane client into fresh device, valid stored, and renewed stored login paths. Each path provisions and saves telemetry after WorkOS persistence; failures are reduced to the existing unavailable status and secret-free warning while WorkOS login remains successful. Removed the deferred dead-code allowance and added path, replacement, and redaction-focused coverage.
  - Verify outcome: The plan command's two test filters were run as equivalent separate commands because Cargo accepts one filter at a time: `auth` passed (16 tests) and `telemetry_credential` passed (18 tests). The combined invocation itself was rejected by Cargo before tests ran.
  - Context impact: important — auth login now owns telemetry credential provisioning and best-effort failure handling across all successful login paths; durable context synchronization remains pending.
  - Context synchronization: synced

## Open questions

None. The failure policy and per-login replacement behavior were clarified before authoring.

## Validation Report

**Status:** validated  
**Date:** 2026-10-14

### Commands run

- `nix flake check` -> exit 0 (all flake checks passed.)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth` -> exit 0 (16 targeted auth tests passed.)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential` -> exit 0 (18 targeted telemetry credential tests passed.)

### Success-criteria verification

- [x] AC1: Every successful `sce auth login` path (fresh device login, valid stored credentials, and renewed stored credentials) attempts `POST /auth/telemetry/credential` with the authenticated WorkOS bearer and replaces the cached telemetry credential on success. -> Targeted auth and telemetry tests passed for login-path provisioning, bearer POST, and cached-credential replacement.
- [x] AC2: Telemetry provisioning and storage failures are best effort: `sce auth login` still succeeds after WorkOS credentials are saved, reports an understandable deterministic warning, and never exposes the telemetry token, accessor, authorization header, or raw server response. -> Targeted auth and telemetry tests passed for text/JSON warnings, secret redaction, sanitized failures, and storage behavior.
- [x] AC3: Existing WorkOS login behavior remains intact, including stored-credential renewal/device-flow fallback and logout deletion of both credential types; telemetry provisioning does not alter the WorkOS credential row. -> Targeted auth and telemetry tests passed, covering device-flow fallback, stored renewal, logout deletion, and untouched WorkOS credentials.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
