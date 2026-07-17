# Human-readable repository Agent Trace DB directory

## Change summary

Replace the repository-scoped Agent Trace database's on-disk **directory name**
from the raw 64-character `repository_id` hash to a human-readable segment:

```
<state_root>/sce/repos/<slug>-<short>/agent-trace.db
```

- `slug` = the `canonical_identity` lowercased, with every run of
  non-alphanumeric characters collapsed to a single `-`, trimmed of leading and
  trailing `-`.
  Example: `github.com/crocoder-dev/shared-context-engineering`
  → `github-com-crocoder-dev-shared-context-engineering`.
- `short` = the first 4 hex characters of `hex(SHA256(canonical_identity))`,
  computed with **no** domain-separation prefix (deliberately distinct from
  `repository_id`, which keeps its `sce-repository-id-v1\0` domain prefix).

The authoritative 64-character `repository_id` is **unchanged**: it is still
derived exactly as today and still stored in and validated against the
`repository_metadata` table on every open. Only the *directory name* changes.
The `repository_metadata` check remains the hard backstop that makes a
(vanishingly unlikely) slug+short collision fail safe with a mismatch error
instead of writing into the wrong database.

This is a **breaking on-disk layout change**, explicitly accepted: existing
`repos/<64-hex>/agent-trace.db` directories are **not** migrated, copied,
renamed, imported, or deleted. They are orphaned; a first open under the new
scheme re-initializes a fresh empty repository database. No data migration is
in scope.

## Success criteria

- `resolve_agent_trace_storage` and the `default_paths` helpers construct the
  active DB path as `<state_root>/sce/repos/<slug>-<short>/agent-trace.db`.
- `slug` and `short` are derived exactly as specified (lowercase, collapsed
  non-alphanumerics, trimmed; first 4 hex of un-prefixed `SHA256(canonical)`).
- Clones and linked worktrees of the same logical repository still resolve to
  the same directory segment and therefore the same database path.
- Different repositories resolve to different segments/paths.
- `repository_id` in `repository_metadata` is unchanged and still rejects a
  genuine repository mismatch on open.
- `sce trace db list` / `status` / `shell` operate on the new directory names,
  display the human-readable segment as the identifier, and still surface the
  authoritative `repository_id` from metadata.
- No migration, copy, rename, or deletion of pre-existing `repos/<hash>/` dirs.
- Context documentation reflects the new path shape everywhere it is described.
- Full `nix flake check` (tests, clippy, fmt) passes.

## Constraints and non-goals

- Do **not** change how `repository_id` is derived or stored; it stays the
  domain-separated 64-char SHA-256 and remains the identity of record in
  `repository_metadata`.
- Do **not** migrate, read, copy, rename, or delete legacy or prior
  repository-scoped databases.
- Do **not** reintroduce the `--legacy` surface removed by
  `retire-legacy-agent-trace-db` (out of scope; separate decision).
- Do **not** add cloud/sync/daemon behavior.
- Keep path-segment safety: reject any derived segment that is empty or not a
  single safe path component.
- Never render credential-bearing input; only `canonical_identity` (already
  credential-free) feeds the slug/short.

## Assumptions

- The `sce trace` discovery identifier (shown in `db list`/`shell`) becomes the
  directory segment (`<slug>-<short>`); the authoritative full `repository_id`
  continues to come from `repository_metadata` and is shown in `status`. Shell
  resolution matches against the directory segment or an assigned alias.
- Short hash length is fixed at 4 hex chars per the change request; the slug
  carries identity and the short hash only disambiguates slug collisions.

## Task stack

- [x] T01: `Add slug + short-hash directory-segment derivation` (status:done)
  - Task ID: T01
  - Goal: Add a pure function in `services::repository_identity` that turns a
    `canonical_identity` into the directory segment `<slug>-<short>`, where slug
    is lowercased with non-alphanumeric runs collapsed to single `-` and
    trimmed, and short is the first 4 hex chars of un-prefixed
    `SHA256(canonical_identity)`. Optionally expose it as a `dir_segment` field
    on `RepositoryIdentity` populated in `identity_from_canonical`.
  - Boundaries (in/out of scope): In — new derivation function/field plus unit
    tests in `repository_identity/mod.rs`. Out — path-helper wiring, callers,
    discovery, docs (later tasks). Do not touch `repository_id` derivation.
  - Done when: A pure `repository_dir_segment(canonical) -> String` (and/or
    `RepositoryIdentity::dir_segment`) exists and is covered by unit tests for:
    lowercasing, non-alphanumeric collapse (`.`,`/`,`:` → single `-`),
    leading/trailing trim, and short-hash equal to `hex(SHA256(canonical))[..4]`
    with no domain prefix (asserted distinct from the `repository_id` prefix).
  - Verification notes (commands or checks): `nix flake check`; new unit tests
    assert `github.com/crocoder-dev/shared-context-engineering` →
    `github-com-crocoder-dev-shared-context-engineering-<4hex>` and that
    `<4hex>` matches an independently computed un-prefixed SHA-256 prefix.
  - **Status:** done
  - **Completed:** 2026-07-17
  - **Files changed:** `cli/src/services/repository_identity/mod.rs`
  - **Evidence:** `nix flake check` — all checks passed (cli-tests, cli-clippy,
    cli-fmt). Added `repository_dir_segment(canonical) -> String`,
    `RepositoryIdentity::dir_segment()`, and the required unit test asserting the
    concrete example segment plus un-prefixed short-hash distinct from the
    domain-prefixed `repository_id`.
  - **Notes:** Per session request, only the required unit test was kept; extra
    tests were removed. Change is additive/pure (no callers touched); classified
    as a localized change (verify-only for context sync, no root-file edits).

- [ ] T02: `Build repository DB path from the slug segment` (status:todo)
  - Task ID: T02
  - Goal: Switch `default_paths::agent_trace_db_path_for_repository{,_at}` and
    all callers to construct the path from the T01 directory segment instead of
    the raw `repository_id`, keeping path-segment safety validation on the
    derived segment.
  - Boundaries (in/out of scope): In — `default_paths.rs` helper signature/body
    + their unit tests; callers in `agent_trace_storage/mod.rs`,
    `agent_trace_db/lifecycle.rs`, `doctor/inspect.rs`; storage tests proving
    clones/worktrees resolve to the same segment path and that path-unsafe
    segments are rejected. Out — discovery/status/shell identifier semantics
    (T03), docs (T04). Must compile as one commit (all callers move together).
  - Done when: The active DB path is
    `<state_root>/sce/repos/<slug>-<short>/agent-trace.db`; existing
    clone/worktree same-path tests pass against the new segment; empty/unsafe
    segment rejection is preserved and tested; no caller still passes the raw
    64-hex id as the directory segment.
  - Verification notes (commands or checks): `nix flake check`; assert two
    checkouts with the same remote resolve to an identical `db_path`; assert an
    unsafe segment (containing `/`, `\`, `.`, `..`) is rejected.

- [ ] T03: `Update trace discovery/status/shell for the new segment` (status:todo)
  - Task ID: T03
  - Goal: Make `sce trace` discovery, list, status, and shell operate on the new
    directory names — display the human-readable segment as the identifier while
    still surfacing the authoritative `repository_id` from `repository_metadata`
    in status; resolve `db shell` by segment or alias.
  - Boundaries (in/out of scope): In — `trace/discovery.rs`,
    `trace/render_status.rs`/`render_list.rs`/`shell.rs` as needed, and their
    fixtures/tests. Out — path construction (T02), docs (T04). Do not restore
    any `--legacy` surface.
  - Done when: `sce trace db list`/`status`/`shell` work against
    `repos/<slug>-<short>/` directories; the displayed identifier is the segment;
    `status` shows the metadata `repository_id`; discovery tiebreak/alias logic
    stays deterministic; tests cover discovery of a new-segment DB and shell
    resolution by segment.
  - Verification notes (commands or checks): `nix flake check`; discovery test
    seeds a `repos/<slug>-<short>/agent-trace.db` and asserts identifier +
    surfaced `repository_id`.

- [ ] T04: `Document the human-readable DB directory path` (status:todo)
  - Task ID: T04
  - Goal: Update every context doc that hardcodes
    `repos/<repository-id>/agent-trace.db` to describe the new
    `repos/<slug>-<short>/agent-trace.db` shape, the slug/short derivation, that
    `repository_id` stays authoritative in `repository_metadata`, and that no
    migration of prior directories occurs.
  - Boundaries (in/out of scope): In — `context/` docs (overview, architecture,
    patterns, context-map, glossary, `cli/agent-trace-storage.md`,
    `cli/default-path-catalog.md`, `cli/repository-identity.md`,
    `cli/trace-command.md`, `cli/service-lifecycle.md`, `sce/agent-trace-db.md`,
    and any other file matching the old path string). Out — code changes. Docs
    only; one coherent commit.
  - Done when: `grep -rn "repos/<repository-id>" context` returns no stale path
    descriptions; the new segment shape and no-migration note are documented in
    the storage/path docs.
  - Verification notes (commands or checks):
    `grep -rn "repos/<repository-id>\|repos/{repository_id}" context` shows only
    updated wording; `grep -rn "slug" context/cli/agent-trace-storage.md`
    confirms the new description.

- [ ] T05: `Final validation and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Run the full verification suite, confirm success criteria, remove any
    temporary scaffolding, and verify context sync is complete.
  - Boundaries (in/out of scope): In — full checks, success-criteria evidence,
    scaffolding removal, context-sync verification, validation report appended
    to this plan. Out — new behavior.
  - Done when: `nix flake check` passes (tests + clippy + fmt); every success
    criterion is verified with evidence; no stray temp files (e.g.
    `context/tmp/sce.log`); a Validation Report section is appended below.
  - Verification notes (commands or checks): `nix flake check`;
    `grep -rn "repos/<repository-id>" context cli/src` returns no stale strings;
    re-run T01–T03 targeted tests; run `sce-context-sync` verification.

## Open questions

None — design fully specified by the change request (slug transform, 4-hex
un-prefixed short hash, unchanged authoritative `repository_id`, no migration).
