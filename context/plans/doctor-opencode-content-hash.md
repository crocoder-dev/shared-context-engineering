# Plan: Add OpenCode content-hash verification to `sce doctor`

## Change summary
- Extend embedded-asset build output to include SHA-256 content hashes.
- Update `sce doctor` to verify repo-root OpenCode asset contents against embedded hashes (presence-only remains for missing files).
- Keep existing text/JSON output structure while adding clear mismatch messaging.

## Success criteria
- `sce doctor` still reports missing required OpenCode files as `[MISS]` with existing missing-file behavior.
- `sce doctor` reports `[FAIL]` when an OpenCode required file exists but content differs from embedded SHA-256.
- Embedded hashes are generated at build time and stored with embedded asset metadata.
- Hash comparisons use file content only (no timestamps/permissions/metadata checks).
- Text output remains aligned with the current doctor layout and status tokens.

## Constraints and non-goals
- Scope limited to OpenCode repo-root assets for content verification (no Claude content checks yet).
- Keep existing hook content checks as-is (no migration required in this change).
- Do not introduce new status tokens beyond `[PASS]`, `[FAIL]`, `[MISS]`.
- Avoid new abstractions; follow current `doctor` and embedded-asset patterns.
- JSON output shape should remain stable (new problems are acceptable; no new top-level schema required).

## Task stack
- [x] T01: Embed SHA-256 hashes in build-time asset manifest (status:done)
  - Task ID: T01
  - Goal: Generate SHA-256 hashes for embedded assets at build time and store them in the embedded metadata used at runtime.
  - Boundaries (in/out of scope):
    - In scope: update `cli/build.rs` to compute hashes, update `EmbeddedAsset` struct, add build-dependency for hashing, update generated manifest output.
    - Out of scope: doctor runtime behavior changes, hook content logic changes, output formatting changes.
  - Done when:
    - `EmbeddedAsset` includes a content-hash field (SHA-256) alongside `relative_path` and `bytes`.
    - `cli/build.rs` emits hash data for every embedded asset entry.
    - Build script has required dependency support (e.g., `sha2` in build-dependencies).
  - Verification notes: defer to T0N full validation (`nix flake check`).
  - Status: done
  - Completed: 2026-04-03
  - Files changed: cli/build.rs, cli/src/services/setup.rs, cli/Cargo.toml
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` (succeeded in 44.12s)
  - Notes: Added SHA-256 field to embedded asset metadata and generated per-file hash literals during build.

- [x] T02: Compare OpenCode repo-root assets against embedded hashes in `doctor` (status:done)
  - Task ID: T02
  - Goal: Detect and report OpenCode asset content mismatches by comparing on-disk SHA-256 hashes to embedded hashes.
  - Boundaries (in/out of scope):
    - In scope: update integration asset inspection to compute hashes for repo-root OpenCode files, classify `PASS`/`FAIL`/`MISS`, and add mismatch problem records.
    - Out of scope: Claude asset checks, new status tokens, changes to hook logic, and new output sections.
  - Done when:
    - For each expected OpenCode asset, doctor reads the file, computes SHA-256, and compares to embedded hash.
    - Missing files remain `[MISS]`; mismatched content is `[FAIL]` with a concise “content mismatch” detail.
    - Problems include a new explicit summary/remediation for content mismatches (category `repo_assets`).
    - Integration group status becomes `[FAIL]` when any child is mismatched or missing.
  - Verification notes: manual spot-check with `sce doctor` in a repo (optional); defer full validation to T0N.
  - Status: done
  - Completed: 2026-04-03
  - Files changed: cli/src/services/doctor.rs
  - Evidence: `nix develop -c cargo check --manifest-path cli/Cargo.toml`
  - Notes: OpenCode integration children now compare on-disk SHA-256 to embedded hashes and surface content mismatches as failures.

- [x] T03: Update doctor contract context to reflect content verification (status:done)
  - Task ID: T03
  - Goal: Sync the doctor contract documentation with the new OpenCode content-hash verification behavior.
  - Boundaries (in/out of scope):
    - In scope: update `context/sce/agent-trace-hook-doctor.md` and any direct references in `context/overview.md` to reflect content verification for OpenCode assets.
    - Out of scope: unrelated context edits or historical summaries.
  - Done when:
    - The doctor contract no longer states OpenCode integrations are presence-only.
    - The overview reflects that OpenCode installed assets are verified for content drift.
  - Verification notes: ensure wording matches implemented behavior and does not introduce new contracts beyond this change.
  - Status: done
  - Completed: 2026-04-03
  - Files changed: context/overview.md, context/sce/agent-trace-hook-doctor.md
  - Evidence: not run (context-only updates)
  - Notes: Updated doctor contract and overview to reflect OpenCode content-hash verification.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run required validations and confirm the plan is ready for completion.
  - Boundaries (in/out of scope):
    - In scope: repository validation and final sanity checks; verify context updates are aligned.
    - Out of scope: new feature work or additional refactors.
  - Done when:
    - `nix run .#pkl-check-generated` completes successfully.
    - `nix flake check` completes successfully.
    - Doctor output examples can be produced for match/miss/mismatch scenarios.
  - Verification notes: run `nix run .#pkl-check-generated` and `nix flake check` from repo root.
  - Status: done
  - Completed: 2026-04-03
  - Files changed: cli/src/services/doctor.rs
  - Evidence:
    - `nix run .#pkl-check-generated` (success)
    - `nix flake check -L --keep-going` (success)
  - Notes: Refactored OpenCode integration health inspection helpers to satisfy clippy `too_many_lines`; full flake check now passes.

## Open questions
- None.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date.)
- `nix flake check -L --keep-going` -> exit 0 (all checks passed)
- `nix flake check` -> exit 0 (all checks passed)

### Failed checks and follow-ups
- None.

### Success-criteria verification
- [x] `nix run .#pkl-check-generated` completes successfully.
- [x] `nix flake check` completes successfully.
- [x] Doctor output examples for match/miss/mismatch provided in task response.

### Residual risks
- None identified.
