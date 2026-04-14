# SCE setup git-hooks install contract

## Scope

Task `sce-setup-githooks-any-repo` `T01` defines the canonical behavior contract for git-hook setup via `sce setup`.
This document is the implementation target for T02-T05.

In scope for this contract:

- target repository and hooks-path resolution policy
- required hook ownership and idempotent update rules
- remove-and-replace replacement flow for all repositories
- deterministic outcome vocabulary and failure diagnostics
- `sce doctor` readiness alignment after successful install

Out of scope for this contract task:

- runtime implementation details of file writes
- CLI parser wiring and final flag surface implementation

## Command surface contract

- Canonical operator command: `sce setup --hooks`
- Optional explicit repository target: `sce setup --hooks --repo <path>`
- Default repository target: current working repository when `--repo` is omitted

`--hooks` mode installs and manages exactly three required hooks:

- `pre-commit`
- `commit-msg`
- `post-commit`

No additional hook types are installed by this workflow.

## Target path resolution

For a selected target repository, setup resolves effective hook destination using git truth:

1. repository root (`git rev-parse --show-toplevel`)
2. effective hooks path (`git rev-parse --git-path hooks`)
3. hook-path source classification via config checks:
   - default (`.git/hooks`)
   - per-repo `core.hooksPath`
   - global `core.hooksPath`

Install behavior must write required hooks into the effective hooks directory returned by git, not by path guessing.

## Hook ownership and idempotency rules

Each required hook has one canonical SCE-managed payload.

Per hook, setup reports exactly one deterministic outcome:

- `installed`: hook was missing and is now present
- `updated`: hook existed and was replaced with newer canonical content
- `skipped`: hook already matched canonical content

Re-running setup with unchanged canonical assets must be idempotent and produce `skipped` for all already-synced hooks.

## Preservation and replacement policy

When setup needs to replace an existing hook file, it performs replacement through a staged write/swap flow and preserves executable permissions required by git hooks.

Setup removes the existing hook directly before swapping staged content. No installer-managed backup artifacts are created. Recovery from a failed swap relies on version control state rather than installer-created backups.

## Rollback guarantees

If replacement fails after staged write preparation but before successful finalization, setup must clean temporary staged artifacts used for failed replacement.

Setup does not attempt installer-managed rollback. On swap failure, setup returns deterministic recovery guidance to recover the hook from version control.

Partial writes that leave required hooks in unknown state are not allowed for successful exits.

## Failure diagnostics contract

Failure output must be actionable and deterministic. Diagnostics should identify:

- repository resolution failures (not a git repo, inaccessible repo)
- effective hooks-path resolution failures
- filesystem write/permission failures
- recovery guidance when installer-managed rollback is intentionally not available

Diagnostics should include affected hook name and target path whenever available.

## Doctor alignment contract

After successful `sce setup --hooks`, `sce doctor` should report `ready` for supported hook-path modes when no external modifications occur between setup and doctor runs.

Supported modes for this alignment:

- default hooks path
- per-repo `core.hooksPath`
- global `core.hooksPath`

## Verification targets for downstream tasks

T02-T05 implementation and tests must verify this contract across:

- fresh install in empty hook directories
- rerun idempotency with unchanged assets
- upgrade path from older/non-canonical hook content
- remove-and-replace behavior under injected replacement failures
- post-setup `sce doctor` readiness