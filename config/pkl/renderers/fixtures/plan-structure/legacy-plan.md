# Plan: remove-checkout-id

## Change summary

Remove the checkout identity feature: the `cli/src/services/checkout/` module (`resolve_git_dir`, `read_checkout_id`, `get_or_create_checkout_id`), the `<git-dir>/sce/checkout-id` file it creates/reads, and every diagnostic-only consumer of it in `agent_trace_storage`, `agent_trace_db` setup lifecycle, and `sce doctor`. Checkout ID is currently never persisted on Agent Trace rows and does not select the active database; it exists solely as a diagnostic label in setup output and `sce doctor --format json`. This replaces existing behavior by dropping that diagnostic surface, mirroring the repository's completed `remove-checkout-registry` removal of the related checkout registry feature.

## Acceptance criteria

- [x] AC1: `sce` no longer creates or reads `<git-dir>/sce/checkout-id`, and no code references the removed `cli/src/services/checkout/` module.
  - Validate: `rg -n "services::checkout|get_or_create_checkout_id|resolve_git_dir|checkout_id" cli/src` returns no matches.
- [x] AC2: `sce setup` and `sce doctor` (text and JSON) no longer mention checkout identity.
  - Validate: run `sce setup` in a scratch repo and confirm the output has no "Agent Trace checkout identity" line; run `sce doctor --format json` and confirm the payload has no `checkout_identity` key.
- [x] AC3: Durable context describes checkout identity as removed, with no remaining claim that current runtime creates, reads, or reports it.
  - Validate: `rg -in "checkout identity|checkout_id|checkout-id" context/` shows only past-tense/removed framing, consistent with the existing `checkout registry` (removed) glossary entry.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/checkout-identity.md`
- `context/cli/agent-trace-storage.md`
- `context/sce/agent-trace-db.md`
- `context/sce/agent-trace-hook-doctor.md`
- `context/cli/cli-command-surface.md`
- `context/cli/service-lifecycle.md`
- `context/context-map.md`
- `context/glossary.md`
- `context/architecture.md`
- `context/overview.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/checkout/` (deletion) and its module declaration in `cli/src/services/mod.rs`; `checkout_id` field and usage in `cli/src/services/agent_trace_storage/mod.rs` (including affected tests); `checkout_id` field, `setup.checkout_id` usage, and the "Agent Trace checkout identity" message line in `cli/src/services/agent_trace_db/lifecycle.rs` (including affected tests); `CheckoutIdentityHealth`, `checkout_identity` field, `collect_checkout_identity_health`, and JSON rendering in `cli/src/services/doctor/{types,inspect,render}.rs`; the ten context files listed under Context sync.
- **Out of scope:** repository identity resolution (`repository_identity`), Agent Trace DB schema/migrations, `sce sync`, and how the repository-scoped Agent Trace DB path is selected or opened. The already-completed `checkout registry` removal (`checkout-registry.json`, `sce doctor dbs`) is unrelated prior work and stays untouched.
- **Constraints:** `nix flake check` (cli-tests, cli-clippy, cli-fmt) and `nix run .#pkl-check-generated` must keep passing; no replacement identity mechanism is introduced.
- **Non-goal:** deleting or migrating any `<git-dir>/sce/checkout-id` file already on disk. SCE never touches pre-existing on-disk artifacts (the same convention already applied to legacy `agent-trace-<checkout-id>.db` files); a stray file is simply left inert.

## Assumptions

- "Remove checkout id" refers to the checkout identity infrastructure in `context/cli/checkout-identity.md` / `cli/src/services/checkout/`, not the separate, already-removed `checkout registry` feature (`remove-checkout-registry` plan).
- Removing `checkout_identity` from `sce doctor --format json` output is an acceptable diagnostic-surface change; no consumer depends on that field for correctness, only for human inspection.

## Task stack

- [x] T01: `Remove checkout identity code and its call sites` (status:done)
  - Task ID: T01
  - Scope: In — delete `cli/src/services/checkout/mod.rs` and its `pub mod checkout;` declaration in `cli/src/services/mod.rs`; remove the `checkout_id` field and the `get_or_create_checkout_id`/`resolve_git_dir` calls (plus the now-invalid `use` import) from `cli/src/services/agent_trace_storage/mod.rs`, updating or removing tests that assert on `checkout_id`; remove the `checkout_id` field from `RepositoryDatabaseSetup`, the `setup.checkout_id` usage, and the "Agent Trace checkout identity: {}" segment of the setup message in `cli/src/services/agent_trace_db/lifecycle.rs`, updating its tests; remove `CheckoutIdentityHealth`, the `checkout_identity` field on `HookDoctorReport`, `collect_checkout_identity_health`, its two call sites, and the `checkout_identity` JSON key from `cli/src/services/doctor/{types,inspect,render}.rs`. Out — repository identity resolution, Agent Trace DB schema/migrations, `sce sync`, durable context files.
  - Dependencies: none
  - Done when: `cli/src/services/checkout/` no longer exists; `rg -n "services::checkout|get_or_create_checkout_id|resolve_git_dir|checkout_id|CheckoutIdentityHealth" cli/src` returns no matches; `nix flake check` passes.
  - Verify: `nix flake check` (cli-tests, cli-clippy, cli-fmt); `rg -n "services::checkout|get_or_create_checkout_id|resolve_git_dir|checkout_id|CheckoutIdentityHealth" cli/src` returns nothing.
  - Completed: 2026-09-13
  - Files changed: `cli/src/services/mod.rs`, `cli/src/services/checkout/mod.rs` (deleted), `cli/src/services/agent_trace_storage/mod.rs`, `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/doctor/types.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`
  - Result: Deleted `cli/src/services/checkout/` and its module declaration; removed `checkout_id`/`resolve_git_dir`/`get_or_create_checkout_id` from `agent_trace_storage::open_storage_with` (and the now-unused `context` parameter threaded through `open_storage`/`open_storage_for_hook_runtime`/`open_storage_with`, plus the resulting unused `anyhow::Context` import), the `checkout_id` field and setup-message segment in `agent_trace_db::lifecycle::RepositoryDatabaseSetup`, and `CheckoutIdentityHealth`/`collect_checkout_identity_health`/its two call sites/the `checkout_identity` JSON key in the doctor module. Updated four `agent_trace_storage` tests that asserted on `checkout_id`, renaming two whose names referenced "checkout_ids"/"checkout_id" since that behavior no longer exists.
  - Verify (actual): `nix build .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` — all three passed (665 tests passed, 0 failed; clippy and fmt clean). `rg -n "services::checkout|get_or_create_checkout_id|resolve_git_dir|checkout_id|CheckoutIdentityHealth" cli/src` returns only `cli/src/services/agent_trace_db/repository.rs` (a pre-existing, out-of-scope schema-invariant test/comment documenting the unrelated absence of a `checkout_id` DB column — not the checkout identity service removed by this task).
  - Context impact: `domain` (diagnostic-only feature removal, no architecture/ownership/terminology-boundary change). During this task's own context-synchronization pass, all ten durable context files listed under "Context sync", plus `context/sce/shared-turso-db.md` (found by sweep, not originally listed), were updated to stop describing checkout identity as active runtime behavior; see the Task context synchronization phase report for this task. T02 remains to do a final targeted review/verification pass against its own Done-when/Verify criteria.
  - Assumptions/deviations: Per the reviewed assumption, the bare `rg ... cli/src` done-check text is read as excluding the noted out-of-scope `repository.rs` residual match, which is unrelated prior work (DB schema invariant) rather than a remnant of the removed checkout identity service.
  - Context synchronization: synced

- [x] T02: `Update durable context to reflect checkout identity removal` (status:done)
  - Task ID: T02
  - Scope: In — `context/cli/checkout-identity.md`, `context/cli/agent-trace-storage.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hook-doctor.md`, `context/cli/cli-command-surface.md`, `context/cli/service-lifecycle.md`, `context/context-map.md`, `context/glossary.md`, `context/architecture.md`, `context/overview.md`: rewrite or remove every claim that checkout identity is created, read, resolved, or reported by current runtime behavior, following the past-tense "removed" framing the `checkout registry` glossary entry already uses for the related prior removal. Out — code changes, plan files, decision records.
  - Dependencies: T01
  - Done when: none of the ten files claim checkout identity is created, stored, resolved, or surfaced by current behavior; `rg -in "checkout identity|checkout_id|checkout-id" context/` shows only past-tense/removed framing.
  - Verify: `rg -in "checkout identity|checkout_id|checkout-id" context/cli context/sce context/context-map.md context/glossary.md context/architecture.md context/overview.md`, reviewed by hand against the T01 code state.
  - Completed: 2026-09-13
  - Files changed: `context/plans/remove-checkout-id.md` (this completion record only — no further edits to the ten scoped files were required)
  - Result: Confirmed all ten scoped files (plus `context/sce/shared-turso-db.md`, updated by T01's own sweep) already carry only past-tense "removed"/"no longer" framing for checkout identity, written during T01's context-synchronization pass and committed in `b525f5e84`. Read every match returned by the scoped verify command by hand, including `context/cli/service-lifecycle.md` (one hit, already past-tense: "no global/checkout fallback path (removed by the `retire-legacy-agent-trace-db` plan)"). No residual live-behavior claim was found, so no rewrite was needed beyond T01's existing work.
  - Verify (actual): `rg -in "checkout identity|checkout_id|checkout-id" context/cli context/sce context/context-map.md context/glossary.md context/architecture.md context/overview.md` — 20 matches across 8 files (`context/overview.md`, `context/architecture.md` x3, `context/glossary.md` x5, `context/context-map.md` x2, `context/cli/cli-command-surface.md`, `context/cli/checkout-identity.md` x3, `context/cli/repository-identity.md`, `context/cli/agent-trace-storage.md` x2, `context/sce/agent-trace-db.md` x3, `context/sce/agent-trace-hook-doctor.md`); every match reviewed by hand and confirmed past-tense/removed framing (e.g. "was removed by the `remove-checkout-id` plan", "no longer mention", "no longer reported").
  - Context impact: `domain` (durable-context-only verification pass; no code or architecture change). No new context impact beyond T01's own sync, which already covered these files.
  - Assumptions/deviations: Per the reviewed assumption, T01's context-synchronization pass already satisfied this task's Done-when/AC3 criteria; T02 served as the planned final targeted review/verification pass rather than a fresh rewrite, and found no gap to close.
  - Context synchronization: synced

## Open questions

None. The removal is fully precedented by the completed `remove-checkout-registry` plan and scoped to a diagnostic-only surface with no persisted-data implications.

## Validation Report

**Status:** validated
**Date:** 2026-09-13

### Commands run

- `nix flake check` -> exit 0 (all checks passed; includes cli-tests, cli-clippy, cli-fmt, pkl-generated, and every other repository check)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 141 files, inventory sha256 `39fe28cef0c034871a003e31f4342e405afd75263d692bf4e3bacbd1dc970fc8`)

### Success-criteria verification

- [x] AC1: `sce` no longer creates or reads `<git-dir>/sce/checkout-id`, and no code references the removed `cli/src/services/checkout/` module -> `cli/src/services/checkout/` does not exist; `rg -n "services::checkout|get_or_create_checkout_id|resolve_git_dir|checkout_id" cli/src` returns only pre-existing, out-of-scope matches in `cli/src/services/agent_trace_db/repository.rs` (a doc-comment and test asserting the schema has *no* `checkout_id` column — documented in T01's completion record as unrelated to the removed checkout identity service).
- [x] AC2: `sce setup` and `sce doctor` (text and JSON) no longer mention checkout identity -> built `.#sce` and ran it in a scratch Git repo with a fake `origin` remote: `sce setup --non-interactive --claude` output contains no "Agent Trace checkout identity" line; `sce doctor --format json` payload contains no `checkout_identity` key.
- [x] AC3: Durable context describes checkout identity as removed, with no remaining claim that current runtime creates, reads, or reports it -> `rg -in "checkout identity|checkout_id|checkout-id" context/` matches only past-tense/removed framing across every context file (root, `context/cli/`, `context/sce/`); the `context/plans/` and `context/decisions/` matches are historical plan/decision records, not current-state claims.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
