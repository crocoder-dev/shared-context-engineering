# Plan: CI Hardening and Hygiene

## Change summary

The repository recently added `.github/workflows/pr-ci.yml` (on the `PR-CI`
branch) as the primary pull-request validation path: a Linux + macOS matrix that
installs Nix, enables Magic Nix Cache, runs `nix flake check`, builds
`.#default`, and smoke-tests the CLI. Release and publish workflows already
exist and are documented in `context/patterns.md` and `context/overview.md`.

This plan hardens and polishes that CI surface without changing the underlying
flake check set or release topology. Concrete gaps found during review:

1. **Unpinned third-party Actions in PR CI** — `pr-ci.yml` uses
   `nix-installer-action@main` and `magic-nix-cache-action@main`, while release
   workflows already pin Determinate Systems actions (`@v22`,
   `@v3.17.3`). Floating `@main` refs are a reproducibility and supply-chain
   risk.
2. **No PR run deduplication** — superseded commits on the same PR keep
   running to completion; there is no `concurrency` cancellation.
3. **No workflow lint in the validation baseline** — `actionlint` passes locally
   today (`nix run nixpkgs#actionlint -- .github/workflows/*.yml`), but nothing
   in `nix flake check` enforces it, so workflow regressions can land unnoticed.
4. **Stale contributor docs** — `config/pkl/README.md:59` still claims GitHub CI
   runs `pkl-generated-parity.yml`, which was removed in favor of the
   `pkl-parity` flake check inside `nix flake check`.
5. **No visible CI status in README** — the completed `pr-ci-magic-nix-cache`
   plan deferred a PR CI badge; README still shows only crates.io and npm
   badges.
6. **Main-branch gap** — PR CI triggers only on `pull_request` and
   `workflow_dispatch`, not `push` to `main`, so direct pushes to `main` would
   skip automated validation.
7. **Minor PR CI ergonomics** — no `timeout-minutes`; the `nix flake metadata`
   step is diagnostic-only and adds latency without gating quality.

Release/publish workflow normalization (mixed `nix-installer-action` vs
`determinate-nix-action`, no Magic Nix Cache on release builds) is noted as an
optional follow-up outside this plan's primary scope.

## Success criteria

- `.github/workflows/pr-ci.yml` pins Determinate Systems action refs to explicit
  version tags (no `@main`), with Magic Nix Cache still immediately after Nix
  install.
- PR CI cancels superseded runs on the same ref via a `concurrency` group and
  has a sensible `timeout-minutes` guard.
- A new `workflow-actionlint` derivation is exposed under `checks.<system>` in
  `flake.nix` and is exercised by `nix flake check` on both Linux and macOS CI
  legs.
- `config/pkl/README.md` accurately describes current CI (`pr-ci.yml` /
  `nix flake check` / `pkl-parity`) with no reference to
  `pkl-generated-parity.yml`.
- `README.md` includes a GitHub Actions badge for the `Nix CI` workflow once it
  is available on `main` (or uses the canonical workflow filename badge URL).
- PR CI also triggers on `push` to `main` so the default branch stays guarded.
- Redundant `nix flake metadata` step is removed from PR CI without dropping
  `nix flake check`, `nix build .#default`, or CLI smoke tests.
- `nix flake check --print-build-logs` and `nix run nixpkgs#actionlint --
  .github/workflows/*.yml` pass locally after all tasks land.
- `context/overview.md` and `context/patterns.md` reflect the updated CI
  contract where applicable.

## Constraints and non-goals

- Constraints
  - Keep all validation and build steps routed through Nix in PR CI; do not add
    direct `cargo`, `bun`, `npm`, or `biome` invocations to workflows.
  - Preserve the existing `nix flake check` derivation set; only add
    `workflow-actionlint` as a new check.
  - Keep PR CI matrix on `ubuntu-latest` + `macos-latest` with
    `fail-fast: false` and job names `Nix CI (<os>)` so branch-protection check
    names stay stable.
  - Keep Magic Nix Cache as the only cache layer in PR CI.
  - Use `actions/checkout@v6` (already pinned).
- Non-goals
  - Do not modify branch-protection rules from inside the repo (admin configures
    required checks out-of-band).
  - Do not change release artifact topology, signing, or publish-stage contracts.
  - Do not normalize release workflow Nix installers or add Magic Nix Cache to
    release builds in this plan (optional follow-up).
  - Do not add path filters, change-detection skips, or split flake checks into
    separate workflow jobs (would increase complexity and runner cost without
    clear payoff at current repo size).
  - Do not reintroduce a standalone `pkl-generated-parity.yml` workflow.

## Assumptions

- The `PR-CI` branch (or equivalent) merges to `main` before T05's badge URL is
  validated against a green `main` workflow run; if not merged yet, T05 uses the
  standard `?branch=main` badge URL and notes first-green-run verification.
- Pin targets align with existing release workflow choices where compatible:
  `DeterminateSystems/nix-installer-action@v22` for PR CI (already used in
  `release-sce-linux.yml` / `release-sce-linux-arm.yml`); Magic Nix Cache pinned
  to a recent stable tag from Determinate Systems releases (exact tag chosen at
  implementation time).
- `workflow-actionlint` runs on all `.github/workflows/*.yml` files including
  release/publish workflows, since they share the same lint surface.

## Task stack

- [x] T01: `Pin Determinate Systems action refs in PR CI` (status:done)
  - Task ID: T01
  - Goal: Replace floating `@main` pins in `.github/workflows/pr-ci.yml` with
    explicit version tags for `nix-installer-action` and `magic-nix-cache-action`.
  - Boundaries (in/out of scope): In — editing `pr-ci.yml` action `uses:` refs
    and any required `with:` blocks for the pinned actions. Out — release
    workflows, flake.nix, README, concurrency/timeouts.
  - Done when: `pr-ci.yml` contains no `@main` Determinate Systems refs;
    `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` is clean; cache
    step remains immediately after Nix install.
  - Verification notes (commands or checks):
    - `grep '@main' .github/workflows/pr-ci.yml` → no Determinate Systems hits
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml`
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `.github/workflows/pr-ci.yml`, `context/overview.md`
  - **Evidence:** `grep '@main' .github/workflows/pr-ci.yml` → no hits; `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` clean
  - **Notes:** Pinned `nix-installer-action@v22` (aligned with Linux release workflows) and `magic-nix-cache-action@v14` (latest stable tag); cache step remains immediately after Nix install

- [x] T02: `Add PR CI concurrency and job timeout` (status:done)
  - Task ID: T02
  - Goal: Cancel outdated PR CI runs on the same head ref and cap runaway job
    duration with `timeout-minutes`.
  - Boundaries (in/out of scope): In — `concurrency` at workflow or job level
    in `pr-ci.yml`, `cancel-in-progress: true` for PR events, and a
    `timeout-minutes` value (suggest 90–120 based on cold `nix flake check`
    history). Out — changing matrix OS list, triggers, or flake checks.
  - Done when: Workflow defines a PR-scoped concurrency group;
    `cancel-in-progress: true` is set; job has `timeout-minutes`; actionlint
    clean.
  - Verification notes (commands or checks):
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml`
    - Inspect workflow YAML for `concurrency:` and `timeout-minutes:`
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `.github/workflows/pr-ci.yml`, `context/overview.md`
  - **Evidence:** `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` clean; workflow defines `concurrency` with `cancel-in-progress: true` and job `timeout-minutes: 90`
  - **Notes:** Workflow-level concurrency group uses `github.workflow` + `github.ref`; 90-minute job timeout within plan's 90–120 range

- [x] T03: `Add workflow-actionlint flake check` (status:done)
  - Task ID: T03
  - Goal: Expose `checks.workflow-actionlint` in `flake.nix` that runs
    `actionlint` against all `.github/workflows/*.yml` files so workflow edits
    are gated by `nix flake check`.
  - Boundaries (in/out of scope): In — new check derivation in `flake.nix`,
    wiring into the `checks` attrset. Out — editing workflow contents beyond any
    fixes required to make actionlint pass under the new check.
  - Done when: `nix flake check --print-build-logs` builds and passes
    `workflow-actionlint` on the current host; `nix build .#checks.x86_64-linux.workflow-actionlint`
    (or system-appropriate path) exits 0.
  - Verification notes (commands or checks):
    - `nix flake check --print-build-logs`
    - `nix build .#checks.$(nix eval --raw --impure --expr builtins.currentSystem).workflow-actionlint`
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `flake.nix`, `context/overview.md`, `context/patterns.md`
  - **Evidence:** `nix build .#checks.x86_64-linux.workflow-actionlint` exit 0; `nix flake check --print-build-logs` exit 0 (~24s)
  - **Notes:** `workflowActionlintCheck` copies `.github/workflows/` and runs `pkgs.actionlint` on all YAML files; no workflow content edits required

- [x] T04: `Fix stale Pkl README CI reference` (status:done)
  - Task ID: T04
  - Goal: Update `config/pkl/README.md` so contributor docs describe the
    current parity path (`pkl-parity` via `nix flake check` / `pr-ci.yml`)
    instead of the removed `pkl-generated-parity.yml` workflow.
  - Boundaries (in/out of scope): In — the stale paragraph around line 59 in
    `config/pkl/README.md`. Out — Pkl sources, generated config, other README
    files.
  - Done when: `grep -rni 'pkl-generated-parity' config/pkl/README.md` is
    empty; doc mentions `nix flake check` / `nix run .#pkl-check-generated` /
    `pr-ci.yml` accurately.
  - Verification notes (commands or checks):
    - `grep -rni 'pkl-generated-parity' config/pkl/README.md` → empty
    - `grep -rni 'pkl-generated-parity' . --include='*.md'` → only historical
      plan/context references if any
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `config/pkl/README.md`
  - **Evidence:** `grep -rni 'pkl-generated-parity' config/pkl/README.md` → empty; updated paragraph references `pr-ci.yml`, `nix flake check`, `pkl-parity`, and `nix run .#pkl-check-generated`
  - **Notes:** Replaced removed `pkl-generated-parity.yml` workflow reference with current PR CI + flake check parity path

- [x] T05: `Add Nix CI status badge to README` (status:done)
  - Task ID: T05
  - Goal: Add a GitHub Actions badge for the `Nix CI` workflow beside the
    existing crates.io and npm badges in `README.md`.
  - Boundaries (in/out of scope): In — one badge line in `README.md` top matter.
    Out — broader README restructuring, badges for release/publish workflows.
  - Done when: README contains a badge linking to the `Nix CI` workflow status
    for `crocoder-dev/shared-context-engineering`; badge URL uses workflow file
    `pr-ci.yml` (or workflow name `Nix CI`).
  - Verification notes (commands or checks):
    - `grep -n 'github/workflows/pr-ci.yml\|Nix CI' README.md`
    - Markdown preview shows three badges in the title block
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `README.md`
  - **Evidence:** `grep -n` confirms badge line with `Nix CI` label and `pr-ci.yml` workflow URLs (`?branch=main`); title block now has three badges (crates.io, npm, Nix CI)
  - **Notes:** Badge uses canonical workflow-file URL; first-green-run on `main` pending until `pr-ci.yml` merges

- [x] T06: `Extend PR CI to validate pushes to main` (status:done)
  - Task ID: T06
  - Goal: Add `push: branches: [main]` to `.github/workflows/pr-ci.yml` so
    direct merges and pushes to `main` run the same Nix validation matrix.
  - Boundaries (in/out of scope): In — `on.push.branches` trigger in `pr-ci.yml`;
    ensure concurrency still behaves sensibly for `push` vs `pull_request`. Out —
    tagging releases, changing required check names.
  - Done when: Workflow triggers on `pull_request`, `push` to `main`, and
    `workflow_dispatch`; actionlint clean; `context/overview.md` updated if the
    trigger list changed.
  - Verification notes (commands or checks):
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml`
    - `grep -A5 '^on:' .github/workflows/pr-ci.yml` shows `push` + `main`
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `.github/workflows/pr-ci.yml`, `context/overview.md`
  - **Evidence:** `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` clean; `on:` block includes `pull_request`, `push`/`main`, and `workflow_dispatch`; existing concurrency group (`github.workflow` + `github.ref`) keeps push and PR runs scoped per ref
  - **Notes:** `main`-only push trigger; no change to required check job names

- [x] T07: `Remove redundant PR CI metadata step` (status:done)
  - Task ID: T07
  - Goal: Drop the diagnostic `nix flake metadata` step from `pr-ci.yml` to
    reduce CI latency without weakening gates.
  - Boundaries (in/out of scope): In — removing the metadata step only. Out —
    removing `nix flake check`, `nix build .#default`, or smoke tests.
  - Done when: `pr-ci.yml` no longer runs `nix flake metadata`; remaining steps
    unchanged in relative order; actionlint clean; `context/overview.md` no longer
    lists the metadata step if it did.
  - Verification notes (commands or checks):
    - `grep 'flake metadata' .github/workflows/pr-ci.yml` → empty
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml`
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `.github/workflows/pr-ci.yml`, `context/overview.md`
  - **Evidence:** `grep 'flake metadata' .github/workflows/pr-ci.yml` → empty; `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` clean; step order unchanged: cache → flake check → build → smoke tests
  - **Notes:** Diagnostic-only metadata step removed; all quality gates retained

- [x] T08: `Validate CI hardening plan and sync context` (status:done)
  - Task ID: T08
  - Goal: Final validation — confirm all prior tasks landed, full flake check
    passes (including `workflow-actionlint`), workflow lint is clean, docs are
    aligned, and this plan is marked complete.
  - Boundaries (in/out of scope): In — running `nix flake check`, actionlint,
    doc greps, marking tasks done, updating `context/overview.md` and
    `context/patterns.md` for the final CI contract. Out — implementing new CI
    behavior beyond this plan.
  - Done when: `nix flake check --print-build-logs` passes; actionlint passes on
    all workflows; stale `pkl-generated-parity` contributor reference gone;
    README badge present; all T01–T07 checkboxes marked `status:done`; open
    questions resolved or deferred with rationale.
  - Verification notes (commands or checks):
    - `nix flake check --print-build-logs`
    - `nix build .#default --print-build-logs`
    - `nix run nixpkgs#actionlint -- .github/workflows/*.yml`
    - `nix run .#sce -- --help && nix run .#sce -- version`
    - `grep -rni 'pkl-generated-parity' config/pkl/README.md` → empty
    - `git status` clean or only intended plan-status edits
  - **Status:** done
  - **Completed:** 2026-06-20
  - **Files changed:** `context/patterns.md`, `context/plans/ci-hardening-hygiene.md`
  - **Evidence:** see Validation Report below
  - **Notes:** Plan complete; all T01–T08 tasks done

## Validation Report

### Commands run
- `nix flake check --print-build-logs` → exit 0 (`all checks passed!`; includes `workflow-actionlint`, `pkl-parity`, `cli-tests` 92 passed)
- `nix build .#default --print-build-logs` → exit 0 (`sce-0.2.0.drv`)
- `nix run nixpkgs#actionlint -- .github/workflows/*.yml` → exit 0 (no diagnostics)
- `nix run .#pkl-check-generated` → exit 0 (`Generated outputs are up to date.`)
- `nix run .#sce -- --help` → exit 0
- `nix run .#sce -- version` → exit 0 (`shared-context-engineering 0.2.0`)
- `grep -rni 'pkl-generated-parity' config/pkl/README.md` → exit 1 (empty)
- `grep -n 'github/workflows/pr-ci.yml\|Nix CI' README.md` → badge present (line 5)
- `grep '@main' .github/workflows/pr-ci.yml` → exit 1 (no `@main` pins)
- Doc/context alignment: `context/overview.md` CI contracts section + `context/patterns.md` verification guidance updated

### Success-criteria verification
- [x] PR CI pins Determinate Systems actions (`@v22`, `@v14`); cache immediately after Nix install → `pr-ci.yml` grep
- [x] PR CI concurrency + `timeout-minutes: 90` → `pr-ci.yml` workflow/job config
- [x] `workflow-actionlint` in `nix flake check` → flake check output lists derivation; build passed
- [x] `config/pkl/README.md` describes current parity path; no `pkl-generated-parity` → grep empty
- [x] README Nix CI badge → `README.md` line 5
- [x] PR CI triggers on `push` to `main` → `on.push.branches: [main]`
- [x] `nix flake metadata` step removed → grep empty on `pr-ci.yml`
- [x] `nix flake check` + actionlint pass locally → command outputs above
- [x] `context/overview.md` and `context/patterns.md` reflect final CI contract → updated in T03/T06/T07/T08

### Residual risks / deferred items
- **Branch protection** — repo admin must still mark `Nix CI (ubuntu-latest)` and `Nix CI (macos-latest)` as required checks after `pr-ci.yml` merges to `main` (out of repo scope).
- **README badge** — may show no status until first green `main` workflow run after merge.
- **Release workflow Nix stack normalization** — deferred follow-up (see Open questions).

## Open questions

- **Release workflow Nix stack normalization** — Should a follow-up plan align
  all release build workflows on the same pinned `nix-installer-action` +
  `magic-nix-cache-action` stack as PR CI, or keep `determinate-nix-action` on
  macOS release legs? Deferred; out of scope here.
- **Branch protection** — Repo admin still needs to mark `Nix CI (ubuntu-latest)`
  and `Nix CI (macos-latest)` as required checks after `pr-ci.yml` is on
  `main`; cannot be done from this repository alone.