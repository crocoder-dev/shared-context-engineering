# Plan: remap-git-notes-action

## Change summary

Build a production-ready TypeScript GitHub Action at `actions/remap-git-notes/` that performs best-effort remediation of Git notes after a PR is merged with GitHub's "Rebase and merge" strategy. GitHub's server-side rebase rewrites commit SHAs, so notes attached to original PR commits are orphaned. The action retrieves the original PR commit SHAs via the GitHub REST API, identifies the corresponding rebased commits on the target branch using a conservative multi-signal confidence model (stable patch ID, monotonic sequence order, content similarity, provenance trailers), copies notes only for high-confidence mappings, merges idempotently with any existing destination notes, and pushes the notes ref with fetch-reconcile-retry on non-fast-forward rejection.

Additionally, wire this repository's own workflow to run the action against `refs/notes/sce-agent-trace` (the Agent Trace notes ref), commit the `dist/` bundle, and add a CI freshness check ensuring `dist/` matches `src/`.

Guiding principle: a missed mapping is acceptable; an incorrect mapping is not. The action never knowingly copies a note to an uncertain commit and never discards an existing destination note.

## Success criteria

- Workflow triggered by `pull_request: [closed]` exits successfully (no-op) when: PR not merged, merge inconsistent with rebase merge, no notes to copy, or no reliable mapping found.
- Inputs supported: `github-token` (required), `notes-ref` (default `refs/notes/commits`), `remote` (default `origin`), `target-branch` (default PR base), `search-depth` (default `50`), `fail-on-unmapped` (default `false`), `dry-run` (default `false`).
- Outputs exposed: `mapped-count`, `copied-count`, `skipped-count`, `unmapped-count`, `conflict-count`, `changed`, `mapping-report` (concise JSON, no large note bodies).
- Mapping uses explicit confidence levels; only high-confidence mappings copy notes by default; every proposed mapping reports old SHA, new SHA, confidence, signals used, and accept/reject reason.
- Note handling is idempotent across reruns: identical notes are no-ops; empty destinations get copies; differing destinations produce a structured merged note (never overwrite/discard) exactly once; conflicts are counted and reported.
- Notes push uses no blind force push; on rejection it fetches, reconciles, reapplies, and retries a bounded number of times.
- Security: no untrusted PR code executed, `execFile`-style argument-safe process execution, token never logged, minimal permissions documented (`contents: write`, `pull-requests: read`), fork considerations documented.
- Markdown job summary written to `$GITHUB_STEP_SUMMARY` including the `| Original | Rebased | Confidence | Result | Reason |` table.
- Unit tests (vitest) cover all listed mapping/notes/error scenarios; integration tests use temporary Git repositories for the eight required scenarios.
- `dist/index.js` bundled with `@vercel/ncc`, committed, with a CI job that fails if `dist/` is stale relative to `src/`.
- `.github/workflows/remap-git-notes.yml` in this repo runs the action with `notes-ref: refs/notes/sce-agent-trace`.
- README documents problem, usage, inputs/outputs, permissions, fork security, mapping limitations, concurrency/retry behavior, dry-run and custom notes-ref examples, troubleshooting.

## Constraints and non-goals

- Node.js 24 runtime (`runs.using: node24`), TypeScript strict mode, `@actions/core`, `@actions/github`, `@actions/exec` (or thin safe wrapper), `@actions/io` where useful.
- Toolchain: plain npm with `package-lock.json`, vitest for tests, `@vercel/ncc` for bundling. All introduced dependencies use their latest published versions (checked at implementation time), pinned exactly. Self-contained under `actions/remap-git-notes/` — no coupling to the repo's Rust/Nix/Pkl toolchain beyond CI wiring.
- Modular layout: `src/main.ts`, `github.ts`, `git.ts`, `mapping.ts`, `notes.ts`, `summary.ts`; no monolithic `main.ts`.
- Conservative defaults: action must not fail because the notes ref doesn't exist, commits lack notes, a commit can't be mapped, the merge was squash/merge-commit, or API metadata is incomplete. It must fail on missing required inputs, unexpected git failures, auth failures, exhausted push retries, or `fail-on-unmapped=true` with unmapped noted commits.
- Provenance trailer names (e.g., `Original-Commit:`) configurable internally (constant list), not exposed as an action input for now.
- Non-goals: squash-merge note remediation, notes remediation for merge-commit merges, GraphQL API usage, publishing to the GitHub Marketplace, cross-repo (fork-writeback) note pushes.

## Assumptions

- Repo workflow will use `github.token` with `contents: write`; the Agent Trace notes ref is pushed by existing hooks per `context/sce/agent-trace-commit-msg-coauthor-policy.md` and this action only remaps after rebase merges.
- "Consistent with a rebase merge" is detected heuristically: merge commit SHA exists, PR base advanced by ≥ PR commit count, and the merge SHA is not a 2-parent merge commit; squash (1 rewritten commit for N>1 PR commits) is treated as non-rebase and skipped. For 1-commit PRs (where squash and rebase are indistinguishable) the action proceeds and relies on the patch-id safety net: notes copy only if the patch-id matches.
- Candidate range anchoring: walk back N (PR commit count) commits from `merge_commit_sha` (GitHub sets it to the last rebased commit); `search-depth` is only a fallback window from branch tip when `merge_commit_sha` is missing or unreachable.

## Task stack

- [x] T01: Scaffold `actions/remap-git-notes/` package (status:done)
  - Completed: 2026-07-15
  - Files changed: actions/remap-git-notes/{action.yml,package.json,package-lock.json,tsconfig.json,vitest.config.ts,.gitignore,src/{main,github,git,mapping,notes,summary}.ts,tests/main.test.ts}
  - Evidence: `npm ci`, `npm run build` (tsc), `npm test` 3/3 passed, `npx tsc --noEmit` clean; action.yml declares 7 inputs + 7 outputs, `runs.using: node24`
  - Notes: deps pinned exactly at latest (@actions/core 3.0.1, @actions/github 9.1.1, @actions/exec 3.0.0, @actions/io 3.0.2; typescript 7.0.2, vitest 4.1.10, @vercel/ncc 0.44.1, @types/node 26.1.1); placeholder `main.ts` reads/validates inputs and exits cleanly; module stubs expose typed interfaces for T02–T08
  - Task ID: T01
  - Goal: Create the action skeleton: `action.yml` (inputs/outputs/runs node24), `package.json` (npm, deps: @actions/core, @actions/github, @actions/exec, @actions/io; devDeps: typescript, vitest, @vercel/ncc — every dependency added at its latest published version at implementation time, then pinned exactly, no `^`/`~` ranges, for deterministic `dist/` rebuilds; verify with `npm view <pkg> version` before pinning), `tsconfig.json` (strict), vitest config, `src/` module stubs with typed interfaces, `.gitignore` for `node_modules`.
  - Boundaries (in/out of scope): In — directory layout, build/test scripts (`build`, `test`, `package` via ncc), placeholder `main.ts` that reads inputs and exits cleanly. Out — any real mapping/notes logic, dist bundle, README.
  - Done when: `npm ci && npm run build && npm test` pass inside `actions/remap-git-notes/`; `action.yml` declares all seven inputs and seven outputs with documented defaults.
  - Verification notes (commands or checks): `cd actions/remap-git-notes && npm ci && npm run build && npm test`; `npx tsc --noEmit`.

- [ ] T02: Implement `github.ts` event parsing and PR commit retrieval (status:todo)
  - Task ID: T02
  - Goal: Parse the `pull_request` payload from `GITHUB_EVENT_PATH`, expose merged/not-merged state, base/head branches, merge commit SHA, PR number; retrieve ordered original PR commit SHAs via the REST API (paginated).
  - Boundaries (in/out of scope): In — typed payload model, octokit wiring via `@actions/github`, treating all API strings as untrusted, unit tests with fixture payloads. Out — git operations, mapping.
  - Done when: unit tests cover merged PR, unmerged PR, missing payload fields, and >100-commit pagination; no token value ever appears in log output.
  - Verification notes (commands or checks): `npm test -- tests/github.test.ts`.

- [ ] T03: Implement `git.ts` safe plumbing layer (status:todo)
  - Task ID: T03
  - Goal: Argument-safe git execution (via `@actions/exec`, never shell interpolation) for: fetching branch/notes refs and commit objects, shallow-clone detection with bounded incremental deepening (`git fetch --deepen`, then `--unshallow` as a last step) until `merge_commit_sha` and PR commits are reachable (clear failure if still unreachable after the cap), `rev-list` of the candidate target range anchored at `merge_commit_sha` walking back the PR commit count (fallback: branch tip + `search-depth` window when `merge_commit_sha` is missing/unreachable), stable patch-id computation (`git show --pretty=format: --binary <sha> | git patch-id --stable`), commit metadata reads (subject, author, timestamp, trailers, changed paths, diffstat).
  - Boundaries (in/out of scope): In — exec wrapper with captured stdout/stderr and explicit error typing, graceful handling of a missing notes ref, unit tests against a temp git repo. Out — note read/write/push (T05/T06), mapping logic.
  - Done when: patch-ids, rev-lists, metadata, and trailer extraction verified against a scripted temp repository; missing notes ref returns a typed "absent" result instead of throwing; a shallow temp clone auto-deepens until required commits are reachable and fails clearly when they are not.
  - Verification notes (commands or checks): `npm test -- tests/git.test.ts`.

- [ ] T04: Implement `mapping.ts` confidence model (status:todo)
  - Task ID: T04
  - Goal: Pure-function mapping engine: candidate generation from the target range, signal scoring (unique stable patch-id, provenance trailer, monotonic sequence position, secondary file/diffstat/subject/author/timestamp similarity), explicit confidence levels, monotonic-order enforcement, ambiguity rejection, and per-mapping structured reasons.
  - Boundaries (in/out of scope): In — deterministic scoring over plain data structures (no git calls), rebase-merge consistency heuristic, unit tests for unique patch-id, duplicate patch-ids, monotonic sequences, reordered candidates, trailer matches, ambiguity rejection, subject-only rejection, and the 1-commit-PR case (squash/rebase indistinguishable: proceed, copy only on patch-id match). Out — note operations, orchestration.
  - Done when: only very-high/high-confidence mappings are marked copyable; duplicate patch-ids resolve solely via monotonic order or are rejected; every decision carries old SHA, new SHA, confidence, signals, and reason.
  - Verification notes (commands or checks): `npm test -- tests/mapping.test.ts`.

- [ ] T05: Implement `notes.ts` read/merge/idempotent apply (status:todo)
  - Task ID: T05
  - Goal: Read notes per commit from the configured ref, and apply the deterministic merge policy: identical → no-op; destination empty → copy; both differ → structured merged note preserving both (`Existing note:` / `Remapped note from <old-sha>:` layout) with a rerun guard that never appends the same remapped block twice — the guard matches the full generated block header (fixed template + 40-char SHA + delimiters) at line boundaries only, so untrusted note content cannot spoof it via substrings; never discard a destination note.
  - Boundaries (in/out of scope): In — note read/write via `git notes --ref=...`, conflict marking, idempotency detection, unit tests for identical/absent/conflicting destination notes and double-run reruns. Out — pushing (T06), mapping.
  - Done when: rerunning apply over an already-remediated repo produces zero changes; conflicting notes yield the documented merged format exactly once.
  - Verification notes (commands or checks): `npm test -- tests/notes.test.ts`.

- [ ] T06: Implement notes push with fetch-reconcile-retry (status:todo)
  - Task ID: T06
  - Goal: Push the notes ref without force; on non-fast-forward rejection: fetch the remote notes ref, reset local notes ref to it, re-read destination notes, re-run the deterministic T05 merge policy to reapply intended updates, commit, and retry with a bounded attempt count; fail with a clear error after exhaustion. No `git notes merge` machinery (avoids `NOTES_MERGE` state), one idempotent code path.
  - Boundaries (in/out of scope): In — push routine in `git.ts`/`notes.ts`, bounded fetch-reset-reapply-retry loop, token-authenticated remote handling without logging the token, tests simulating a concurrently advanced notes ref via a second temp clone. Out — orchestration, summary.
  - Done when: concurrent-update test passes (both writers' notes survive); no `--force` in any push invocation; retries capped and surfaced.
  - Verification notes (commands or checks): `npm test -- tests/push.test.ts`; `grep -rn '\-\-force' actions/remap-git-notes/src` shows no force push.

- [ ] T07: Implement `summary.ts` logging and job summary (status:todo)
  - Task ID: T07
  - Goal: Produce the Markdown `$GITHUB_STEP_SUMMARY` (PR number, branches, notes ref, rebase-merge detection, commit/candidate counts, mapped/skipped/ambiguous/conflict counts, push status, and the `| Original | Rebased | Confidence | Result | Reason |` table) and annotation policy (`core.info` progress, `core.warning` unmapped/ambiguous, `core.error` real failures).
  - Boundaries (in/out of scope): In — summary rendering from a typed report object, Markdown escaping of untrusted strings (subjects, branch names), unit tests on rendered output. Out — computing the report data itself.
  - Done when: summary snapshot tests pass; untrusted strings cannot break table/Markdown structure.
  - Verification notes (commands or checks): `npm test -- tests/summary.test.ts`.

- [ ] T08: Wire `main.ts` orchestration, outputs, and error policy (status:todo)
  - Task ID: T08
  - Goal: Compose T02–T07 into the full flow: input parsing/validation, merged/rebase gating with clean no-op exits, fetch → map → apply → push pipeline, `dry-run` (report only, no writes/pushes), `fail-on-unmapped` enforcement, all seven action outputs including compact JSON `mapping-report`, and the conservative fail/no-fail error policy.
  - Boundaries (in/out of scope): In — orchestration, output setting, exit behavior, unit tests for dry-run, fail-on-unmapped, unmerged-PR no-op, squash-merge no-op, missing notes-ref no-op. Out — new mapping/notes logic.
  - Done when: all no-op conditions exit 0 with an explanatory summary; dry-run performs zero git writes; `fail-on-unmapped=true` fails only when a noted commit is unmapped; outputs match counts in the report.
  - Verification notes (commands or checks): `npm test -- tests/main.test.ts`; `npm run build`.

- [ ] T09: Integration tests with temporary Git repositories (status:todo)
  - Task ID: T09
  - Goal: End-to-end tests in `tests/integration/` that script real temp repos (bare origin + clone) simulating a server-side rebase merge, covering the eight required scenarios: 3-commit clean rebase; conflict-altered commit; two identical-patch commits; destination with same note; destination with different note; double run; absent notes ref; concurrent notes-ref update.
  - Boundaries (in/out of scope): In — temp-repo fixtures/helpers, event-payload fixtures, stubbed PR-commit listing (no live API), assertions on notes content and push results. Out — real GitHub API calls, CI wiring.
  - Done when: all eight scenarios pass locally and in CI; identical-patch scenario maps via monotonic order or conservatively rejects — never crosswires notes.
  - Verification notes (commands or checks): `npm test -- tests/integration`.

- [ ] T10: Bundle `dist/` and commit build artifact (status:todo)
  - Task ID: T10
  - Goal: Produce `dist/index.js` (plus license file) via `@vercel/ncc`, ensure `action.yml` `runs.main` points at it, and commit the bundle so the action is directly usable.
  - Boundaries (in/out of scope): In — `npm run package` script, deterministic bundle output, committed `dist/`. Out — CI freshness check (T12), README.
  - Done when: `npm run package` regenerates an identical `dist/` from a clean tree; a local `act`-style or node smoke run of `dist/index.js` with a fixture event exits 0.
  - Verification notes (commands or checks): `cd actions/remap-git-notes && npm run package && git diff --exit-code dist/`.

- [ ] T11: Write README and example workflow (status:todo)
  - Task ID: T11
  - Goal: `actions/remap-git-notes/README.md` covering: problem statement (why notes are lost across rebase merges), full workflow usage (including the canonical example with `concurrency` group and permissions), input/output tables, fork security considerations, mapping limitations and confidence model, retry/concurrency behavior, dry-run and custom notes-ref examples, troubleshooting, and key design decisions.
  - Boundaries (in/out of scope): In — README plus the documented example workflow snippet. Out — this repo's live workflow (T12), code changes.
  - Done when: README includes every documentation item from the spec and the exact example workflow with `concurrency: git-notes-${{ github.repository }}` and `permissions: contents: write / pull-requests: read`.
  - Verification notes (commands or checks): manual review against the spec's Documentation checklist.

- [ ] T12: Add repo workflow and dist freshness CI check (status:todo)
  - Task ID: T12
  - Goal: Add `.github/workflows/remap-git-notes.yml` running the local action on merged PRs with `notes-ref: refs/notes/sce-agent-trace`, proper permissions, and the serializing concurrency group; add a CI job (in the same or existing CI workflow) that pins Node to a fixed 24.x via `actions/setup-node`, runs `npm ci && npm run package`, and fails on `git diff --exit-code dist/` (byte-compare; deterministic thanks to exactly-pinned devDependencies).
  - Boundaries (in/out of scope): In — the two workflow additions, path filters so the dist check runs on `actions/remap-git-notes/**` changes. Out — action code changes, other CI refactors.
  - Done when: workflow YAML validates (actionlint or CI dry parse); dist check demonstrably fails on an intentionally stale bundle and passes on a fresh one.
  - Verification notes (commands or checks): `nix run nixpkgs#actionlint -- .github/workflows/remap-git-notes.yml` (or `actionlint` if available); temporarily touch `src/` then confirm `npm run package && git diff --exit-code dist/` catches drift.

- [ ] T13: Validation, cleanup, and context sync (status:todo)
  - Task ID: T13
  - Goal: Full verification pass and shared-context synchronization for the new action.
  - Boundaries (in/out of scope): In — run complete test/build/bundle checks, remove dead stubs/TODOs, confirm no token leakage or force pushes, update `context/` docs (context-map entry plus a `context/sce/` note on the remap action and its relation to the Agent Trace notes auto-push policy). Out — new features.
  - Done when: `npm ci && npx tsc --noEmit && npm test && npm run package` all pass cleanly with fresh `dist/`; `git diff --exit-code` after packaging; context docs updated and consistent; no unresolved TODOs in `actions/remap-git-notes/src`.
  - Verification notes (commands or checks): `cd actions/remap-git-notes && npm ci && npx tsc --noEmit && npm test && npm run package && git diff --exit-code`; `grep -rn "TODO" actions/remap-git-notes/src`; review `context/context-map.md` for the new entries.

## Open questions

- None. Confirmed by the user on 2026-07-15:
  - Location `actions/remap-git-notes/`; toolchain npm + vitest + ncc; repo workflow on `refs/notes/sce-agent-trace` + committed `dist/` with CI freshness check.
  - Candidate range anchored at `merge_commit_sha` with `search-depth` tip-window fallback.
  - Merge-method detection: heuristic + patch-id safety net (1-commit PRs proceed, copy only on patch-id match).
  - Push reconcile: fetch remote notes ref, reset, reapply T05 merge policy, bounded retry (no `git notes merge`).
  - Rerun guard: exact structural block-header match at line boundaries.
  - Dist determinism: exactly-pinned devDependencies + pinned Node 24.x in CI; shallow clones auto-deepen with a bounded cap.
