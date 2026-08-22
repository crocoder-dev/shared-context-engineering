# Decision: Resolve Codex apply_patch paths through canonical worktree containment

Date: 2026-08-23
Status: Accepted
Plan: `context/plans/codex-cli-integration.md`
Task: `T20`

## Context

Codex `apply_patch` supplies paths relative to its event cwd and can also supply
absolute paths or valid parent traversal. SCE must accept upstream-compatible
inputs without allowing evidence paths to escape the real Git worktree,
including through symlinks. Add File targets may not exist yet, so checking only
the final path is insufficient.

## Decision

Resolve each Codex apply_patch source and move-destination path independently
from the event cwd against the canonical Git root, accepting relative `..` and
absolute paths only when canonical resolution remains inside the worktree. For
missing targets, canonicalize and validate the nearest existing prefix, then
emit the resulting repository-relative UTF-8 slash path. Reject malformed,
outside, or symlink-escaping mappings before normalization or database access.

## Rationale

This preserves current upstream path compatibility while keeping the evidence
boundary tied to the real repository rather than the hook process cwd. Prefix
canonicalization protects both existing paths and not-yet-created Add File
paths without snapshots or filesystem-delta observation.

## Alternatives considered

- **Reject all absolute and parent-traversal paths** — safer lexically but
  incompatible with valid current Codex inputs.
- **Use lexical normalization only** — accepts compatible syntax but cannot
  detect symlink escapes.
- **Take a filesystem snapshot** — could provide stronger mutation evidence but
  violates the Codex no-snapshot design and is outside this integration's scope.

## Compatibility and risks

- Valid upstream-accepted paths inside the worktree now resolve successfully;
  unsafe or ambiguous mappings fail open with no evidence.
- Missing targets are represented from their validated existing prefix, so a
  later filesystem change between the hook and commit can still affect physical
  occurrence attribution; the existing post-commit intersection remains the
  final filter.

## Guardrails

- The canonical Git root and event cwd must resolve to existing directories.
- Every source and move destination is resolved independently.
- No snapshot, pending state, schema migration, or generic intersection change
  is introduced by this path contract.

## Consequences

- Codex evidence contains only repository-relative UTF-8 slash paths.
- Add File paths can be absent at hook time, while existing and missing symlink
  escapes are rejected conservatively.

## Follow-up

None.

## References

- Plan: [`codex-cli-integration`](../plans/codex-cli-integration.md)
- Task: `T20`
- Current-state context: [`Codex hook runtime`](../sce/codex-integration-runtime.md)
- Evidence: [`path resolution implementation`](../../cli/src/services/hooks/codex/apply_patch/path.rs)
- Related decision: [`Codex root-aware hook invocation`](2026-08-23-codex-root-aware-hook-invocation.md)
