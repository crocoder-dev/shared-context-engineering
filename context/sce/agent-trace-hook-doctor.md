# SCE doctor operator environment contract

## Scope

Task `sce-doctor-operator-environment` `T01` defines the approved contract for broadening `sce doctor` from hook-readiness validation into the canonical installed-CLI operator health and repair entrypoint for SCE.
This document is the implementation target for `T02` through `T07` in that plan.

In scope for this contract task:

- operator-environment readiness semantics for installed CLI, global SCE state, and repo-scoped rollout state
- deterministic problem taxonomy, severity/fixability classes, and remediation metadata
- stable text/JSON output additions for diagnosis and `--fix` reporting
- the standing maintenance rule that every new SCE-managed setup/install surface must extend `sce doctor` coverage in the same change stream

Out of scope for this contract task:

- Rust implementation changes
- parser/help wiring beyond the contract needed for downstream tasks
- broad machine diagnostics unrelated to SCE-owned operator readiness

## Current implementation baseline

At the current implementation point, the runtime in `cli/src/services/doctor.rs` exposes the approved doctor command surface and stable output-shape scaffolding, and now covers the global-readiness slice plus the current repo-integrity, database-inventory, and repair slices:

- explicit mode selection through `sce doctor` (`diagnose`) and `sce doctor --fix` (`fix`)
- explicit all-SCE database inventory selection through `sce doctor --all-databases`
- command/help wiring for `--fix` and `--all-databases` plus stable text/JSON mode reporting
- stable repo-vs-all database inventory reporting in text and JSON output
- stable problem records with category, severity, fixability, and remediation metadata
- deterministic fix-result records in fix mode with `fixed`, `skipped`, `manual`, and `failed` outcomes
- state-root reporting as `present` or `expected`
- default global/local config-file location reporting, plus validation of existing global and repo-local `sce/config.json` readability and schema compliance
- Agent Trace local DB location reporting, DB parent-directory readiness checks, and existing-DB health validation
- an empty repo-scoped database section in the default readiness view because no repo-owned SCE database currently exists
- explicit all-SCE database inventory rendering for the canonical Agent Trace DB only
- text output is grouped into sections (`Environment`, `Configuration`, `Repository`, `Git Hooks`, `Integrations`) with an `SCE Doctor` header and divider lines
- section headings and divider lines are untagged; status tags (`[PASS]`, `[FAIL]`, `[MISS]`, `[WARN]`) still prefix all content lines
- status tag prefixes are colorized in text output (PASS=green, FAIL=red, WARN=yellow, MISS=blue) and only the prefix is colored when `supports_color()` allows it (TTY + no `NO_COLOR`)
- text output includes separate OpenCode entries (`OpenCode plugin`, `OpenCode agent`, `OpenCode command`, `OpenCode skills`) with PASS/FAIL summary lines and indented detail lines when failures occur
- explicit git-unavailable, outside-repo, and bare-repo repository-targeting failures
- effective hook-path source (`default`, local `core.hooksPath`, global `core.hooksPath`)
- repository root and hooks directory resolution when a repository target is detected
- required hook presence and executable permissions for `pre-commit`, `commit-msg`, and `post-commit` when repo-scoped checks apply
- byte-for-byte stale-content detection for required hook payloads against canonical embedded SCE-managed hook assets
- repo-scoped OpenCode root directory presence is required; missing `.opencode/` is reported as a manual-only `repo_assets` error
- repo-scoped OpenCode structure checks for required `.opencode/agent`, `.opencode/command`, and `.opencode/skills` directories, with missing/non-directory paths reported as manual-only errors
- repo-scoped OpenCode embedded asset presence checks for expected files under `.opencode/agent`, `.opencode/command`, and `.opencode/skills`, with missing/non-file paths reported as manual-only `repo_assets` errors
- repo-scoped OpenCode plugin registry/file presence checks for the `sce-bash-policy` plugin run when `.opencode/` exists, plus runtime dependency and preset catalog presence checks; registry-missing is an error while file/runtime/preset findings are warnings (manual-only remediation)
- repair-mode reuse of `cli/src/services/setup.rs::install_required_git_hooks` for missing hooks directories plus missing, stale, or non-executable required hooks
- doctor-owned bootstrap of the missing canonical SCE-owned Agent Trace DB parent directory, with deterministic refusal when the resolved path does not match the expected owned location

Downstream tasks must expand the runtime from this current baseline to the full contract below without changing the command entrypoint.
The focused database inventory contract for `sce-doctor-database-and-config-coverage` task `T02` lives in `context/sce/doctor-database-inventory-contract.md` and owns repo-scoped versus all-SCE database inventory expectations.

## Command surface contract

- Canonical operator command: `sce doctor`
- Canonical explicit repair mode: `sce doctor --fix`
- Canonical explicit all-database inventory mode: `sce doctor --all-databases`
- Stable output modes: text (default) and `--format json`

Default `sce doctor` behavior remains diagnosis-only and read-only.
`sce doctor --fix` is the only canonical repair path and may perform only safe, idempotent repairs bounded to SCE-owned paths/files or explicit permission normalization on those paths.

`sce doctor --fix` must not:

- delete unrelated files
- overwrite unknown config without explicit SCE ownership rules
- mutate git config unexpectedly
- repair non-owned or ambiguous targets without deterministic refusal guidance

## Readiness model

`sce doctor` reports one top-level readiness verdict for the inspected operator environment:

- `ready`: no blocking SCE operator issues were detected
- `not_ready`: one or more blocking issues were detected

The readiness verdict covers three check domains:

1. installed CLI/runtime identity
2. global SCE state/config/DB readiness
3. repository and hook rollout readiness when repo-scoped checks are applicable

Repo-scoped issues make readiness `not_ready` only when a repository target is required for the inspected surface. Non-repo global checks must still run and report deterministically when `sce doctor` is executed outside a git repository.

## Problem taxonomy

Every detected issue must map to exactly one stable problem category:

- `runtime_identity`: installed binary/runtime metadata or command-surface problems
- `global_state`: global state-root, config-path, config-contents, or Agent Trace DB readiness problems
- `repository_targeting`: repository resolution, git availability, bare-repo, or hooks-path discovery problems
- `hook_rollout`: missing, partial, stale, non-executable, or otherwise unhealthy required SCE-managed hooks
- `repo_assets`: missing or stale repo-local SCE-managed assets outside the hook files themselves
- `filesystem_permissions`: missing parent directories, unwritable owned paths, rename/temp/backup barriers, or permission failures blocking safe repair
- `remediation_coverage`: gaps where doctor can diagnose an issue but does not yet own a canonical repair path

Each problem record must also include stable severity and fixability classes.

### Severity classes

- `error`: blocks readiness and requires repair or explicit manual action
- `warning`: non-blocking but operator-visible risk, drift, or partial-state concern that must still surface in output
- `info`: non-failure contextual guidance that helps explain status or next actions

### Fixability classes

- `auto_fixable`: safe for `sce doctor --fix` to repair idempotently
- `manual_only`: not safe for automatic repair; output must include deterministic manual remediation
- `not_yet_implemented`: intended to become auto-fixable, but current doctor repair coverage is incomplete and must report the gap explicitly

## Required check inventory

The broadened contract for `sce doctor` must cover the following problem inventory.

### Installed CLI/runtime identity

- binary/runtime metadata is incomplete or inconsistent
- installed build does not expose the expected SCE command surface
- required runtime directories cannot be resolved on the current platform

### Global SCE state/config readiness

- global SCE state root cannot be resolved
- expected global config path cannot be resolved
- global config file exists but is unreadable, invalid JSON, or fails schema validation
- Agent Trace local DB path cannot be resolved
- Agent Trace DB parent directories are missing or not writable
- Agent Trace DB exists but bootstrap or migration health is broken

### Repository targeting and git readiness

- repo-scoped checks are required but `sce doctor` is run outside a git repository
- `git` is unavailable or repository inspection commands fail
- repository root resolution fails or resolves unexpectedly
- repository is bare or unsupported for local hook rollout
- effective hooks directory cannot be resolved
- local or global `core.hooksPath` points to a missing or unexpected location

### Hook rollout integrity

- effective hooks directory is missing
- required SCE hooks are missing
- required hooks exist but are not executable
- required hook payloads differ from the canonical embedded SCE-managed content
- only some required hooks are current, producing a partial rollout
- hook files have launcher, shebang, or path issues that prevent reliable execution
- adjacent rewrite/runtime guidance required for healthy rollout is missing from operator output

### Repo-installed SCE assets

- expected repo-local `.sce/` state/config directories are missing when the repair path needs them
- installed repo-facing SCE assets are missing or stale relative to canonical embedded assets
- prior setup was only partially applied

### Filesystem and permission barriers

- effective hooks directory is not writable for repair
- repo-local `.sce/` directory is not writable for repair
- global state/config/DB parent directories are not writable
- backup-and-replace safety cannot proceed because temp, rename, or write permissions fail

### Remediation coverage gaps

- doctor detects an issue but does not map it to one canonical SCE repair action
- an issue is safely auto-fixable but no internal `doctor --fix` repair path exists yet
- an issue is not auto-fixable and needs deterministic manual remediation guidance in text/JSON output

## Remediation contract

Every reported problem must include deterministic remediation metadata:

- whether the issue is `auto_fixable`, `manual_only`, or `not_yet_implemented`
- one canonical next action (`doctor_fix`, `setup_hooks`, `manual_steps`, or another stable action label introduced by downstream tasks)
- concise human-readable remediation text in text mode
- stable machine-readable remediation fields in JSON mode

When an issue is `manual_only`, `sce doctor` must return explicit manual steps instead of vague diagnostics.
When an issue is `not_yet_implemented`, output must say that doctor recognizes the issue but does not yet own an automatic repair path.

## `--fix` execution contract

`sce doctor --fix` must report deterministic per-problem repair results using this outcome vocabulary:

- `fixed`: doctor repaired the issue successfully
- `skipped`: doctor intentionally left the issue unchanged because it was already healthy or did not require action
- `manual`: doctor identified the issue but it requires manual remediation
- `failed`: doctor attempted an allowed repair but the repair did not complete successfully

Repair behavior must:

- reuse existing canonical SCE repair flows when ownership already exists, especially `sce setup --hooks` semantics and shared setup/security helpers
- add new internal doctor-owned repair routines only for safe gaps with no existing canonical repair command
- stay idempotent across repeated `--fix` runs
- remain bounded to SCE-owned paths/files and explicit permission normalization on those paths

## Output shape contract

Text and JSON output must both expose:

- command identity and inspection mode (`diagnose` or `fix`)
- readiness verdict
- inspected environment summary across runtime/global/repo domains
- stable problem records with category, severity, fixability, and remediation guidance
- `--fix` result reporting when repair mode is used

The JSON contract must remain stable enough for downstream automation and include machine-readable problem and fix-result records rather than free-form diagnostics only.

Text output uses grouped sections (`Environment`, `Configuration`, `Repository`, `Git Hooks`, `Integrations`) with an `SCE Doctor` header and divider lines. Section headings and divider lines are untagged; status tags (`[PASS]`, `[FAIL]`, `[MISS]`, `[WARN]`) still prefix all content lines so operators can scan status quickly. The status prefix is colorized (PASS=green, FAIL=red, WARN=yellow, MISS=blue) while the remainder of the line is uncolored, and colorization is gated by `supports_color()` (TTY + no `NO_COLOR`).

## Setup and doctor alignment rule

Doctor/setup alignment is a standing repository contract, not a one-off for hook rollout.

Every newly added SCE-managed setup/install surface must define, in the same change stream:

- what `sce doctor` checks for that surface
- how readiness changes when that surface is missing, stale, partial, or misconfigured
- whether each new issue class is `auto_fixable`, `manual_only`, or `not_yet_implemented`
- which canonical repair action owns remediation
- which durable context files describe the resulting current-state contract

No new SCE setup/install capability is considered complete until matching `sce doctor` readiness and remediation coverage is defined and synchronized in context.

## Downstream verification targets

`T02` through `T08` must verify this contract through parser/help tests, doctor-focused service tests, and final repo verification.
At minimum, downstream work must prove deterministic coverage for output shape, readiness transitions, diagnosis-only safety, supported `--fix` repairs, refusal paths for unsafe repairs, and setup-to-doctor alignment for newly managed surfaces.
