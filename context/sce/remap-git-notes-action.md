# Remap Git Notes GitHub Action

Local TypeScript GitHub Action at `actions/remap-git-notes/` that will perform
best-effort remediation of Git notes after a PR is merged with GitHub's
"Rebase and merge" strategy. GitHub's server-side rebase rewrites commit SHAs,
so notes attached to original PR commits (including Agent Trace notes under
`refs/notes/sce-agent-trace`) are orphaned. The action maps original PR
commits to their rebased counterparts with a conservative multi-signal
confidence model and copies notes only for high-confidence mappings.

Guiding principle: a missed mapping is acceptable; an incorrect mapping is
not. The action never knowingly copies a note to an uncertain commit and
never discards an existing destination note.

Active plan: `context/plans/remap-git-notes-action.md`.

## Current implementation state (T01 scaffold)

Only the package scaffold exists; no mapping/notes logic is implemented yet.

- `action.yml` — declares the full contract: 7 inputs (`github-token`,
  `notes-ref` default `refs/notes/commits`, `remote` default `origin`,
  `target-branch`, `search-depth` default `50`, `fail-on-unmapped` default
  `false`, `dry-run` default `false`), 7 outputs (`mapped-count`,
  `copied-count`, `skipped-count`, `unmapped-count`, `conflict-count`,
  `changed`, `mapping-report`), `runs.using: node24` with `main: dist/index.js`
  (bundle not yet committed).
- `package.json` — plain npm, dependencies pinned exactly (no `^`/`~`) for
  deterministic `dist/` rebuilds: `@actions/core`, `@actions/github`,
  `@actions/exec`, `@actions/io`; devDeps `typescript` (strict), `vitest`,
  `@vercel/ncc`, `@types/node`.
- `src/` module stubs with typed interfaces matching planned boundaries:
  `main.ts` (input parsing implemented; orchestration placeholder that exits
  cleanly), `github.ts` (event parsing/PR commit retrieval), `git.ts` (safe
  argument-based git plumbing), `mapping.ts` (confidence model), `notes.ts`
  (read/merge/apply/push), `summary.ts` (job summary rendering).
- `tests/main.test.ts` — input parsing coverage; scripts: `build` (tsc
  typecheck), `test` (vitest), `package` (ncc bundle).

## Boundaries

- Self-contained under `actions/remap-git-notes/`; not coupled to the repo's
  Rust/Nix/Pkl toolchain beyond future CI wiring.
- Repo workflow wiring (`.github/workflows/remap-git-notes.yml` targeting
  `refs/notes/sce-agent-trace`) and dist freshness CI arrive in later plan
  tasks (T10–T12).
- Related: `context/sce/agent-trace-commit-msg-coauthor-policy.md` (Agent
  Trace notes are pushed by local hooks; this action only remaps after
  rebase merges).

See also: [context-map.md](../context-map.md)
