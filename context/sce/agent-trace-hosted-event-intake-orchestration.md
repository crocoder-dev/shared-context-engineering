# Agent Trace Hosted Event Intake Orchestration

## Scope

- Implements T12 for plan `agent-trace-attribution-no-git-wrapper`.
- Accepts hosted provider rewrite events and turns them into replay-safe reconciliation run requests.
- Covers provider parsing/signature/idempotency intake only; mapping heuristics remain out of scope (`T13`).

## Code ownership

- Hosted intake service: `cli/src/services/hosted_reconciliation.rs`.
- Public intake seam: `ingest_hosted_rewrite_event`.
- Service module registration: `cli/src/services/mod.rs`.

## Intake contract

- Provider coverage is explicit for GitHub and GitLab (`HostedProvider`).
- GitHub webhook signatures use crate-backed HMAC-SHA256 (`hmac` + `sha2`) and require `sha256=<hex>` match against payload body.
- GitLab webhook signatures use token equality against the configured shared secret.
- Intake payload parsing uses structured `serde_json::Value` extraction (no manual substring scanning) for `before`, `after`, and provider-specific repository identity.
- Intake requires resolvable rewrite heads (`before`, `after`) and provider-specific repository identity (`repository.full_name` for GitHub, `project.path_with_namespace` for GitLab).
- Missing fields, invalid container types, and non-string required values fail with deterministic `invalid hosted event payload: ...` messages.
- `before` and `after` values must be SHA-like 40-char hex commit IDs.

## Reconciliation run orchestration contract

- Provider events are normalized into `HostedReconciliationRunRequest` with provider, repo, event, old/new heads, and deterministic idempotency key.
- Deterministic replay key derivation uses provider + event + repo + old/new heads + delivery ID material and crate-backed SHA256 digesting (`sha2`).
- Run storage is abstracted behind `ReconciliationRunStore`; ingestion returns created vs duplicate outcome (`ReconciliationRunInsertOutcome`) for replay-safe semantics.

## Validation coverage

- GitHub signature verification + run creation.
- GitLab token verification + run creation.
- Duplicate event replay behavior returns duplicate outcome without creating a new side effect class.
- Required payload field validation for old/new head resolution.
- Required payload field type/object-shape validation for deterministic parse failures.
- Deterministic idempotency key stability for identical inputs.

## Verification evidence

- `cargo test --manifest-path cli/Cargo.toml hosted_reconciliation`
- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `cargo build --manifest-path cli/Cargo.toml`

## Related context

- `context/sce/agent-trace-implementation-contract.md`
- `context/sce/agent-trace-reconciliation-schema-ingestion.md`
- `context/plans/agent-trace-attribution-no-git-wrapper.md`
