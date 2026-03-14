# Plan: SCE Doctor Operator Environment

## Change summary

Expand `sce doctor` from hook-readiness-only validation into the canonical installed-CLI operator health check for SCE.
The command should help someone who already installed the `sce` binary on their machine answer two questions in one place: "is this machine/repository ready for SCE?" and "what is the exact fix?"

The planned scope is operator environment readiness for an installed CLI, not contributor/developer setup for this repository.

Potential problem inventory to cover in `sce doctor`:

- CLI/runtime identity problems
  - `sce` binary is installed but command/runtime metadata is incomplete or inconsistent.
  - The installed build does not expose the expected SCE command surface.
  - Required runtime directories cannot be resolved on the current platform.
- Global SCE state/config problems
  - global SCE state root cannot be resolved
  - expected global config path cannot be resolved
  - global config file exists but is unreadable, invalid JSON, or fails schema validation
  - Agent Trace local DB path cannot be resolved
  - Agent Trace DB parent directories are missing or not writable
  - Agent Trace DB exists but bootstrap/migration health is broken
- Repository targeting and git problems
  - `sce doctor` is run outside a git repository when repo-scoped checks are required
  - `git` is unavailable or git inspection commands fail
  - repository root resolution fails or points somewhere unexpected
  - repository is bare or otherwise unsupported for local hook rollout
  - effective hooks directory cannot be resolved
  - `core.hooksPath` is set locally or globally to a missing or unexpected location
- Hook rollout problems
  - effective hooks directory is missing
  - required SCE hooks are missing
  - required hooks exist but are not executable
  - required hook payloads differ from the canonical embedded SCE-managed content
  - only some required hooks are current, producing a partial rollout
  - hook files have launcher/shebang/path issues that prevent reliable execution
  - hook rollout is still "ready" for current three-hook policy but misses adjacent rewrite/runtime guidance that operator output should surface clearly
- Repo-installed SCE asset problems
  - expected repo-local `.sce/` state/config directories are missing when a fix path needs them
  - installed repo-facing SCE assets are missing or stale relative to canonical embedded assets
  - prior setup was only partially applied
- Filesystem and permission problems
  - effective hooks directory is not writable for repair
  - repo-local `.sce/` directory is not writable for repair
  - global state/config/db parent directories are not writable
  - backup-and-replace safety cannot proceed because temp/rename/write permissions fail
- Remediation coverage problems
  - `sce doctor` detects an issue but does not yet map it to one canonical SCE repair action
  - an issue is safely auto-fixable but no internal `doctor --fix` repair path exists yet
  - an issue is not auto-fixable and needs deterministic manual remediation guidance in text/JSON output

## Success criteria

- `sce doctor` remains safe by default and performs diagnosis-only unless `--fix` is explicitly requested.
- `sce doctor` reports operator-environment readiness for an installed CLI, global SCE state, and repo-scoped rollout state.
- Text and JSON output both expose a stable problem taxonomy, readiness verdict, and per-problem remediation guidance.
- `sce doctor --fix` performs only safe, idempotent repairs and reports what it fixed, skipped, or could not fix.
- Existing canonical repair flows are reused where possible, especially `sce setup --hooks` semantics and any shared setup/security helpers.
- Gaps with no existing repair command are handled by new internal repair routines behind `sce doctor --fix` rather than leaving fixability fragmented.
- Non-auto-fixable issues return deterministic manual remediation steps instead of vague diagnostics.
- Help text and command-surface docs describe `sce doctor --fix` as the canonical operator repair path.
- Context files describing `doctor`, setup, and operator workflow are updated to reflect the broadened contract.
- Context establishes an explicit maintenance rule: every newly added SCE setup/install surface must define the matching `sce doctor` readiness and remediation coverage before the setup work is considered complete.

## Constraints and non-goals

- Keep the entrypoint as `sce doctor`; do not add a separate top-level repair command.
- Default `sce doctor` behavior must stay read-only.
- `sce doctor --fix` must not perform destructive actions such as deleting unrelated files, overwriting unknown config without explicit ownership rules, or mutating git config unexpectedly.
- Reuse existing setup/install contracts where they already define canonical SCE-managed assets.
- Keep repair behavior idempotent and bounded to SCE-owned paths/files or explicit permission bits on those paths.
- Treat doctor/setup alignment as a standing contract, not a one-off for current hook rollout work.
- Do not expand this task into general-purpose machine diagnostics unrelated to SCE operator readiness.
- Do not treat shared-context-engineering repo contributor tooling as the target environment for this change.

## Task stack

- [x] T01: Define the expanded `sce doctor` operator-health contract (status:done)
  - Task ID: T01
  - Goal: Convert the current hook-readiness-only contract into a canonical operator-environment health contract with explicit problem categories, readiness semantics, fixability classes, and a standing doctor/setup alignment rule for future setup surfaces.
  - Boundaries (in/out of scope):
    - In: `context/sce/agent-trace-hook-doctor.md` replacement or expansion, related `context/overview.md` / `context/glossary.md` / `context/architecture.md` wording updates if the contract name or scope changes.
    - In: Define stable text/JSON output additions for problem inventory, remediation metadata, and `--fix` reporting.
    - In: Define the policy that any future SCE-managed setup/install addition must also extend `sce doctor` coverage and current-state context in the same change stream.
    - Out: Rust implementation changes.
  - Done when: Context clearly defines what `sce doctor` checks, how failures are categorized, which failures are auto-fixable vs manual-only, and how future setup features must register doctor coverage.
  - Verification notes (commands or checks): Review contract coverage against the problem inventory; ensure output fields, readiness rules, and the future setup->doctor maintenance rule are deterministic.

- [x] T02: Add CLI surface and output-shape support for `sce doctor --fix` (status:done)
  - Task ID: T02
  - Goal: Extend CLI parsing, command help, and output schema so `doctor` supports read-only diagnosis and explicit repair mode without ambiguity.
  - Boundaries (in/out of scope):
    - In: `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/command_surface.rs`, doctor request/response types, help text, and parser tests.
    - In: Stable JSON fields for issue severity/fixability/fix result reporting.
    - Out: Actual repair logic.
  - Done when: `sce doctor`, `sce doctor --format json`, and `sce doctor --fix` parse cleanly and their help/output contracts are documented and testable.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml doctor`; verify command-local help mentions `--fix` and output schema stays deterministic.

- [x] T03: Implement installed-CLI and global SCE path/config health checks (status:done)
  - Task ID: T03
  - Goal: Broaden doctor diagnostics beyond hooks to validate state-root resolution, global config health, Agent Trace DB path/bootstrap readiness, and writable parent paths.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/doctor.rs`, shared config/local-db/security helpers, doctor-focused tests.
    - In: Validation for config readability/schema issues and DB bootstrap/readiness checks that remain safe in diagnosis mode.
    - Out: Repo hook rollout checks beyond what is needed to keep this task single-intent.
  - Done when: Doctor reports deterministic status for global config/state/DB health and surfaces precise remediation text for each failure mode.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml doctor`; targeted tests for missing/unwritable/invalid config and DB-path/bootstrap states.

- [x] T04: Implement repo, git, and hook-integrity diagnostics with stale-content detection (status:done)
  - Task ID: T04
  - Goal: Upgrade repo-scoped doctor checks to validate git availability, repo resolution, hook-path source/health, required hook presence/executable bits, and canonical content drift for SCE-managed hooks.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/doctor.rs`, setup hook-asset accessors in `cli/src/services/setup.rs`, doctor tests for default/local/global hook-path cases.
    - In: Deterministic diagnostics for missing repo, bare repo, missing hooks dir, missing hooks, non-executable hooks, and stale hook payloads.
    - Out: Repair execution.
  - Done when: Doctor can distinguish healthy hooks from stale or partially installed hooks and reports exact per-hook problems plus remediation guidance.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml doctor`; fixtures covering missing/stale/non-executable/custom-hook-path scenarios.

- [x] T05: Reuse existing canonical repair flows inside `sce doctor --fix` (status:done)
  - Task ID: T05
  - Goal: Wire `doctor --fix` to reuse safe existing setup/install behavior for issues already owned by current commands and services.
  - Boundaries (in/out of scope):
    - In: Repair orchestration in `cli/src/services/doctor.rs` and/or shared helpers; invocation of canonical hook installation/update logic from setup-service code paths; permission normalization for SCE-managed hooks where ownership is explicit.
    - In: Fix-result reporting (`fixed`, `skipped`, `manual`, `failed`) in text/JSON output.
    - Out: New repair routines for gaps not covered by existing setup/install flows.
  - Done when: `sce doctor --fix` can safely repair canonical existing problem classes such as missing/stale/non-executable SCE-managed hooks using existing ownership-aware logic.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml doctor`; integration-style tests proving diagnosis -> fix -> ready for supported existing repair paths.

- [x] T06: Add new internal doctor repair routines for uncovered safe-fix gaps (status:done)
  - Task ID: T06
  - Goal: Implement bounded internal repair operations for safe issues that have no existing canonical command, so doctor can fix them instead of only diagnosing them.
  - Boundaries (in/out of scope):
    - In: New doctor-owned helpers for SCE-managed writable-path bootstrap, owned-directory creation, safe DB/config parent preparation, or similar gaps discovered during T01-T05 contracting.
    - In: Deterministic refusal behavior for non-owned, unsafe, or ambiguous repairs.
    - Out: Broad new standalone command surfaces or invasive config migration behavior.
  - Done when: Remaining auto-fixable issues in the approved taxonomy are either implemented behind `doctor --fix` or explicitly marked manual-only with stable guidance.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml doctor`; targeted tests for each new doctor-owned fix routine and refusal path.

- [x] T07: Update operator-facing docs and SCE context for the new doctor workflow (status:done)
  - Task ID: T07
  - Goal: Align help text, context contracts, and CLI foundation docs so `sce doctor` is documented as the canonical health-and-repair entrypoint for installed users and as the required companion to new setup/install surfaces.
  - Boundaries (in/out of scope):
    - In: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/placeholder-foundation.md`, `context/context-map.md`, and the focused doctor/setup contract docs.
    - In: Any generated-command source text in `config/pkl/base/shared-content.pkl` only if command/help wording for SCE workflows must change.
    - In: Durable wording that says every new setup capability must update doctor diagnostics/remediation guidance and related context at the same time.
    - Out: Unrelated command docs.
  - Done when: All current-state docs describe the broadened doctor contract, repair mode, relationship to `sce setup`, and the rule that new setup surfaces require corresponding doctor updates.
  - Verification notes (commands or checks): Read-through audit for stale hook-only wording and missing setup->doctor policy wording; `nix run .#pkl-check-generated` if generated source text changes.

- [x] T08: Validation and cleanup (status:done)
  - Task ID: T08
  - Goal: Run final verification, confirm context sync, and ensure no stale hook-only assumptions or setup-without-doctor gaps remain.
  - Boundaries (in/out of scope):
    - In: CLI tests, generated-output parity when needed, and full repo checks required by the lightweight post-task baseline.
    - In: Final context verification for doctor/setup/operator wording.
    - Out: New feature work.
  - Done when: Verification passes, context matches code truth, `sce doctor` / `sce doctor --fix` behavior is fully covered by current-state docs, and the setup->doctor maintenance rule is visible in durable context.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`, `nix run .#pkl-check-generated`, `nix flake check`, plus final read-through for any setup contract lacking corresponding doctor expectations.

## Final validation report

- Commands run:
  - `cargo test --manifest-path cli/Cargo.toml`
  - `nix run .#pkl-check-generated`
  - `nix flake check`
- Exit codes and key outputs:
  - `cargo test --manifest-path cli/Cargo.toml` -> exit 0; `test result: ok. 245 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
  - `nix run .#pkl-check-generated` -> exit 0; `Generated outputs are up to date.`
  - `nix flake check` -> exit 0; evaluated `cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity`
- Failed checks and follow-ups:
  - None.
- Success-criteria verification summary:
  - `sce doctor` and `sce doctor --fix` behavior remains covered by current-state docs in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/placeholder-foundation.md`, and `context/sce/agent-trace-hook-doctor.md`.
  - Final context-sync pass was verify-only for shared root files; no new root-level behavior changed during T08.
  - Feature-existence/discoverability coverage remains present through `context/sce/agent-trace-hook-doctor.md` and its links from `context/context-map.md` and `context/overview.md`.
  - No stale hook-only wording or setup-without-doctor policy gaps were found in the audited doctor/setup context surface.
- Residual risks:
  - None identified beyond the existing documented T06 implementation baseline for `sce doctor` repair coverage.

## Open questions

- None. Scope is clarified: installed-CLI operator environment readiness, with `sce doctor --fix` as the canonical repair path.
