# Fix Git hook CLI guidance

## Change summary

Make the three canonical Git hook assets under `cli/assets/hooks/` consistently tolerate a missing `sce` executable. Each hook must use POSIX `sh`, fail fast for ordinary script errors, detect `sce` with `command -v`, print the canonical installation guidance to stderr when it is unavailable, and exit successfully so Git operations are not blocked solely because the CLI is missing.

The `pre-commit` and `commit-msg` hooks will add this guard while preserving their existing `sce hooks ...` invocations and argument forwarding. The already-compliant `post-commit` hook will retain its Git remote URL discovery and forwarding behavior; remote URL forwarding remains exclusive to `post-commit` because only that CLI subcommand accepts `--vcs` and `--remote-url`.

Resolved plan name: `fix-git-hook-cli-guidance`.

## Success criteria

- `cli/assets/hooks/pre-commit`, `cli/assets/hooks/commit-msg`, and `cli/assets/hooks/post-commit` remain POSIX `#!/bin/sh` scripts using `set -eu`.
- When `sce` is unavailable, every canonical Git hook prints `sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli` to stderr and exits with status 0 without attempting its `sce hooks` command.
- When `sce` is available, `pre-commit` executes `sce hooks pre-commit` and forwards all hook arguments unchanged.
- When `sce` is available, `commit-msg` executes `sce hooks commit-msg` and forwards the commit message path and any other arguments unchanged.
- When `sce` is available, `post-commit` preserves its current behavior: it executes `sce hooks post-commit --vcs git`, includes the quoted origin URL through `--remote-url` when non-empty, and forwards all hook arguments unchanged.
- Embedded hook assets used by setup, doctor, and hook lifecycle repair reflect the updated source assets, and repository validation plus direct shell/asset inspection confirm the missing-CLI contract for all three hooks.
- Repository validation passes and shared context accurately documents the non-blocking missing-CLI behavior.

## Constraints and non-goals

### Constraints

- Scope is limited to the canonical Git hooks in `cli/assets/hooks/` and the tests/context needed to protect and document their behavior.
- Keep the scripts POSIX `sh`; do not introduce Bash-only syntax.
- Preserve safe quoting for `"$remote_url"` and `"$@"`, even though the compact request example omitted quotes.
- Use the existing canonical guidance text and URL exactly, with output sent to stderr.
- Missing `sce` is intentionally non-blocking (`exit 0`); failures from an available `sce` continue to propagate through `exec`.
- Keep the hook assets as thin orchestration around the Rust CLI. Do not move hook policy or domain logic into shell.
- Run Rust validation through Nix, with `nix flake check` as the preferred repository-level check.

### Non-goals

- Changes to Claude hooks, generated Claude wrappers, OpenCode/Pi integrations, or third-party hooks.
- Adding remote URL options to `pre-commit` or `commit-msg`.
- Changing Rust hook subcommand behavior, CLI parsing, attribution policy, trace persistence, setup installation semantics, or doctor stale-content comparison logic.
- Changing behavior when Git itself is unavailable or when an available `sce` command fails.
- Reformatting unrelated shell scripts or generated configuration.

## Task stack

- [x] T01: `Guard every canonical Git hook when sce is unavailable` (status:done)
  - Task ID: T01
  - Goal: Add the canonical non-blocking `command -v sce` guard to `pre-commit` and `commit-msg` while preserving the compliant `post-commit` orchestration.
  - Boundaries (in/out of scope):
    - In: `cli/assets/hooks/pre-commit`, `cli/assets/hooks/commit-msg`, and `cli/assets/hooks/post-commit` only if a consistency adjustment is needed.
    - Out: Rust hook runtime/parser behavior, non-Git hook wrappers, generated config trees, setup/doctor lifecycle refactors, and unrelated shell cleanup.
  - Done when: all three source assets contain the same exact missing-CLI guidance guard; missing `sce` returns 0 and emits guidance on stderr for each hook; available `sce` receives the expected subcommand and unchanged arguments; `post-commit` still conditionally forwards the origin URL; the change is one coherent atomic commit.
  - Verification notes (commands or checks): inspect the three asset diffs, validate the scripts with POSIX `sh -n`, verify the exact guidance string, and rely on `nix flake check` for repository validation.
  - Completed: 2026-07-23
  - Files changed: `cli/assets/hooks/pre-commit`, `cli/assets/hooks/commit-msg`, `context/sce/setup-githooks-hook-asset-packaging.md`, `context/context-map.md`
  - Evidence: `nix flake check --print-build-logs` passed for tracked check surfaces, including 171/171 Rust unit tests, clippy, rustfmt, and generated-output parity; direct shell/asset inspection passed; `git diff --check` passed.
  - Notes: A standalone hook integration suite created during execution was removed at user request, and durable context now records that behavioral shell-asset coverage is not retained. Context sync classified this as a localized change: root shared context was verify-only, while the existing hook-packaging domain document and context-map entry were refreshed to current code truth.

- [x] T02: `Document non-blocking missing-CLI Git hooks` (status:done)
  - Task ID: T02
  - Goal: Synchronize shared context with the canonical Git hook bootstrap behavior introduced by T01.
  - Boundaries (in/out of scope):
    - In: the existing hook setup/install context documents under `context/sce/` and corresponding `context/context-map.md` entries that describe packaged hook assets, installation, or lifecycle repair; document that all three hooks warn and exit successfully when `sce` is missing, while available-CLI failures still propagate and only `post-commit` forwards remote metadata.
    - Out: user-facing website documentation, broad architecture rewrites, decision records, and implementation changes.
  - Done when: relevant shared context matches the implemented scripts, no context claims that only `post-commit` provides missing-CLI guidance, context-map descriptions remain accurate, and the documentation update is one coherent atomic commit.
  - Verification notes (commands or checks): review `context/sce/setup-githooks-hook-asset-packaging.md`, `context/sce/setup-githooks-install-contract.md`, related hook lifecycle/setup context, and `context/context-map.md`; use `rg -n "sce CLI not found|missing.*sce|post-commit" context/sce context/context-map.md` to find stale statements; inspect the final documentation diff.
  - Completed: 2026-07-23
  - Files changed: `context/sce/setup-githooks-install-contract.md`, `context/sce/setup-githooks-install-flow.md`, `context/sce/agent-trace-hook-doctor.md`, `context/context-map.md`
  - Evidence: targeted stale-claim and exact-guidance searches matched the all-hook contract; only `cli/assets/hooks/post-commit` contains `--remote-url`; all touched context files remain at or below 250 lines; `git diff --check` passed.
  - Notes: Documentation-only task; no build was applicable. Context sync classification is verify-only for root overview/architecture/glossary because the localized behavior was already implemented in T01; domain documents and context-map discoverability were synchronized.

- [x] T03: `Validate Git hook assets and clean up` (status:done)
  - Task ID: T03
  - Goal: Run full repository validation, confirm generated/embedded asset consistency and context sync, and remove any temporary test artifacts.
  - Boundaries (in/out of scope):
    - In: `nix flake check`, generated-output parity verification if the implementation touches generated files, final shell/asset diff review, context sync verification, worktree cleanliness checks for temporary fake commands or repositories, and only minimal fixes required to make the approved change pass validation.
    - Out: new hook behavior, unrelated refactors, and opportunistic documentation changes.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes if applicable; all three packaged hook assets satisfy the success criteria; no temporary artifacts remain; context accurately reflects behavior; any validation-only correction is landable as at most one coherent atomic commit.
  - Verification notes (commands or checks): `nix flake check`; conditionally `nix run .#pkl-check-generated`; `git diff --check`; inspect `git status --short`; verify the exact guidance string appears in all three `cli/assets/hooks/*` files and that only `post-commit` contains `--remote-url`.
  - Completed: 2026-07-23
  - Files changed: `context/plans/fix-git-hook-cli-guidance.md`, `context/sce/setup-githooks-hook-asset-packaging.md`, `context/sce/setup-githooks-install-contract.md`, `context/context-map.md`; removed `cli/tests/hook_assets.rs`
  - Evidence: `nix flake check --print-build-logs` passed; `nix run .#pkl-check-generated` reported generated outputs up to date; all three hook scripts passed `sh -n`; the exact guidance appeared once in each hook; only `post-commit` contained `--remote-url`; `git diff --check` passed; no task-created temporary artifacts remained.
  - Notes: Validation-only final task. The standalone behavioral hook tests were removed at user request after the initial done-gate review, and context was revised to describe the current validation posture. Context-sync classification is verify-only for root shared context because this task introduced no behavior, architecture, policy, or terminology change.

## Validation report

### Commands run

- `nix flake check --print-build-logs` -> exit 0; all compatible-system checks passed, covering the repository test, clippy, rustfmt, generated parity, JavaScript, workflow, portability, and Flatpak validation surfaces.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- POSIX shell syntax and static asset assertions -> exit 0; all three hooks passed `sh -n`, each contained the exact guidance once, and only `post-commit` contained `--remote-url`.
- Ad hoc isolated hook behavior validation with temporary fake `sce` and `git` commands -> exit 0; all three missing-CLI cases warned and returned success, and all three available-CLI cases forwarded the expected subcommand and arguments, including the quoted post-commit origin URL.
- `git diff --check` -> exit 0.
- Temporary artifact inspection -> no task-created artifacts remain; the temporary validation directory was removed by its command trap, and `cli/tests/` was removed at user request.

### Success-criteria verification

- [x] All canonical hooks remain POSIX `#!/bin/sh` scripts using `set -eu` -> direct source inspection and `sh -n`.
- [x] Missing `sce` prints the exact canonical stderr guidance and exits 0 for all three hooks -> isolated behavior validation.
- [x] Available `pre-commit` and `commit-msg` invocations forward all arguments unchanged -> isolated behavior validation.
- [x] Available `post-commit` preserves `--vcs git`, quoted non-empty origin URL forwarding, and unchanged hook arguments -> isolated behavior validation.
- [x] Setup, doctor, and lifecycle repair consume the updated canonical embedded assets -> canonical asset/source inspection plus successful flake build and parity checks.
- [x] Shared context describes the all-hook missing-CLI behavior and post-commit-only remote forwarding -> domain context and context-map review.
- [x] Repository validation and generated-output parity pass -> commands above.

### Failed checks and follow-ups

- A direct targeted Cargo integration-test command attempted before the user-requested test removal was blocked by repository bash policy, which requires `nix flake check`; the policy-approved full check passed. No product validation failures remain.

### Residual risks

- Behavioral shell-asset tests are not retained in the automated Rust test suite by user request. Final behavior was validated ad hoc, while syntax, embedding, buildability, and generated parity remain covered by repository validation.

## Open questions

None. Scope and remote forwarding behavior were resolved during clarification.
