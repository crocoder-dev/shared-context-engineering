# Plan: Add remote OpenCode plugin comparison in `sce doctor`

## Change summary
- Add `sce doctor --opencode-plugin-remote` to compare local `.opencode/plugins/*` with the canonical plugin files in `shared-context-engineering` on `main`.
- Run the remote comparison only when `.opencode/` exists locally, and surface an aggregated `repo_assets` warning when any differences are detected.
- Treat remote fetch failures as warnings and keep existing local checks intact.

## Success criteria
- New flag `--opencode-plugin-remote` triggers a remote comparison against `https://github.com/crocoder-dev/shared-context-engineering` at `main`.
- Remote comparison runs only if `.opencode/` exists locally.
- Any difference between local `.opencode/plugins/*` and remote `config/.opencode/plugins/*` yields one aggregated `repo_assets` problem with **severity=warning** and **fixability=manual_only**.
- Remote fetch errors (HTTP, network, JSON parse, rate limit) yield one aggregated `repo_assets` **warning** with `manual_only` remediation.
- No auto-fix is attempted for remote mismatches; `sce doctor --fix` reports manual-only outcomes for these findings.
- Tests cover: flag gating, `.opencode/` gating, remote mismatch warning, and remote fetch failure warning.

## Constraints and non-goals
- Do not run remote comparison without the `--opencode-plugin-remote` flag.
- Do not change the local plugin registry/file/drift checks already implemented.
- No caching or background fetch; comparison is synchronous and best-effort.
- Scope is limited to `.opencode/plugins/*` (no registry or other `.opencode/` content).
- No changes to generated assets or plugin content.

## Task stack
- [x] T01: Add CLI flag + request plumbing for remote plugin check (status:done)
  - Task ID: T01
  - Goal: Parse `--opencode-plugin-remote` and plumb it into doctor execution.
  - Boundaries (in/out of scope): In - CLI schema, request struct, report render metadata as needed. Out - remote fetch/compare logic.
  - Done when: Doctor execution can detect the flag and pass intent to the service layer.
  - Verification notes (commands or checks): `nix flake check`.

- [x] T02: Implement remote plugin comparison with aggregated warning (status:done)
  - Task ID: T02
  - Goal: Fetch remote `config/.opencode/plugins/*` from GitHub `main`, compare with local `.opencode/plugins/*`, and emit one aggregated `repo_assets` warning for any mismatch or fetch failure.
  - Boundaries (in/out of scope): In - remote fetch via GitHub API/Raw URLs, file list comparison, aggregated warning, `.opencode/` gating. Out - any auto-fix, caching, or registry checks.
  - Done when: Remote mismatch/fetch failure yields a single manual-only warning; no issues emitted when file sets and contents match.
  - Verification notes (commands or checks): Unit tests with stubbed network; `nix flake check`.

- [x] T03: Extend doctor tests for remote plugin flag (status:done)
  - Task ID: T03
  - Goal: Add tests that assert flag gating, `.opencode/` gating, mismatch warning, and fetch-failure warning.
  - Boundaries (in/out of scope): In - deterministic tests using mocked fetches. Out - live network calls.
  - Done when: Tests cover all success criteria cases in a sandbox-safe manner.
  - Verification notes (commands or checks): `nix flake check`.

- [x] T04: Sync context docs with remote check behavior (status:done)
  - Task ID: T04
  - Goal: Update doctor context docs to describe `--opencode-plugin-remote` and aggregated warning behavior.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-hook-doctor.md` and related CLI context files. Out - broad documentation rewrites.
  - Done when: Context reflects the new optional remote comparison, severity mapping, and manual-only remediation.
  - Verification notes (commands or checks): Manual review.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run required repo checks and ensure the plan state is current.
  - Boundaries (in/out of scope): In - `nix flake check` (and `nix run .#pkl-check-generated` only if generated outputs change). Out - extra tests not required.
  - Done when: Validation passes and plan status reflects completion.
  - Verification notes (commands or checks): `nix flake check`.

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/cli_schema.rs, cli/src/app.rs, cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Added `--opencode-plugin-remote` flag, request plumbing, and parser coverage; no behavior changes yet.

### T02
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Added remote plugin snapshot fetch and aggregated warning comparison gated by `--opencode-plugin-remote` and `.opencode/` presence.

### T03
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Added tests covering flag gating, `.opencode/` gating, mismatch warning, and fetch failure warning.

### T04
- Status: done
- Completed: 2026-03-24
- Files changed: context/sce/agent-trace-hook-doctor.md, context/cli/placeholder-foundation.md
- Evidence: Manual review for alignment with implemented doctor behavior.
- Notes: Documented optional `--opencode-plugin-remote` behavior, gating, and aggregated warning semantics.

### T05
- Status: done
- Completed: 2026-03-24
- Files changed: context/plans/doctor-opencode-plugin-remote.md
- Evidence: `nix flake check`
- Notes: No generated assets changed; skipped `nix run .#pkl-check-generated`.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed; omitted incompatible systems: aarch64-darwin, aarch64-linux, x86_64-darwin)

### Success-criteria verification
- [x] `--opencode-plugin-remote` triggers remote comparison -> implemented in `inspect_opencode_plugin_remote_health` gated by `opencode_plugin_remote`.
- [x] Remote comparison runs only if `.opencode/` exists -> guarded by `.opencode/` existence in `inspect_opencode_plugin_remote_health` and test `doctor_remote_plugin_check_skips_without_opencode_root`.
- [x] Any plugin differences yield one aggregated warning -> aggregated `repo_assets` warning emitted when snapshot maps differ.
- [x] Remote fetch errors yield one aggregated warning -> handled by fetch failure path with warning summary.
- [x] No auto-fix attempted for remote mismatches -> warning fixability `manual_only`, no auto-fix path added.
- [x] Tests cover gating/mismatch/failure -> `doctor_remote_plugin_check_*` tests in `cli/src/services/doctor.rs`.

### Residual risks
- Remote comparison depends on GitHub API availability and rate limits when flag is enabled.
