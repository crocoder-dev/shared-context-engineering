# Mutation-trace Agent Trace attribution

The pure attribution seam in `cli/src/services/mutation_trace/attribution.rs`
adds mutation history as a conservative secondary source for committed touched
lines. It does not read Git, SQLite, the filesystem, or mutation state. A caller
supplies target-shaped direct coverage, the committed lines still unresolved,
and already-reconstructed mutation evidence in newest-first order.

## Resolution contract

- Directly covered lines are removed before mutation matching. Mutation history
  cannot revoke or replace direct evidence.
- Each mutation event is considered in caller-supplied newest-first order. The
  first safe event matching a line resolves it; a later event cannot reclaim
  that line.
- An event contributes mutation-derived AI coverage only when it is untainted,
  has `FailureKind::Healthy`, and carries `Attribution::AiExclusive(_)`.
  Contended, unscoped, unhealthy, and tainted matches resolve as non-AI.
- Mutation AI, resolved non-AI, and still-unresolved results retain the
  committed target patch's file/hunk shape and deterministic target ordering.
  Mutation-derived results do not acquire direct model, session, tool, or
  tool-version provenance.

## Safe matching

A `MutationPatchEvidence` patch is matched to unresolved target lines using a
separate strict matcher; the existing permissive direct `intersect_patches`
operation is unchanged.

1. A file's logical path is `new_path` when it is non-empty, otherwise
   `old_path`.
2. Exact logical-path pairing is attempted first. Normalized suffix pairing is
   allowed only when exactly one mutation file and one unresolved target file
   can be paired.
3. Within a safely paired file, equal `(kind, line_number, content)` keys are
   matched first, one-to-one.
4. Remaining lines may use the historical `(kind, content)` fallback only when
   that key occurs exactly once in each side.
5. Repeated candidates and ambiguous file or line matches are left unresolved;
   attribution prefers false negatives to guessed ownership.

The matcher exposes location-based matches so repeated lines cannot be
silently collapsed in the result. The seam is intentionally independent of
pagination, event reconstruction, worktree identity, and post-commit Agent
Trace persistence; those boundaries remain owned by their respective runtime
and storage services.
