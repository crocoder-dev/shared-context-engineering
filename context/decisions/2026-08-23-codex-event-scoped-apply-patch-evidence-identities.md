# Decision: Use event-scoped synthetic identities for Codex apply_patch evidence

Date: 2026-08-23
Status: Accepted
Plan: `context/plans/codex-cli-integration.md`
Task: `T16`

## Context

Codex `apply_patch` hook input identifies changed content but does not provide
reliable source line ranges. SCE must preserve the touched Add/Update lines so
the existing post-commit intersection can attribute a later commit, while
avoiding a filesystem snapshot, a pending mutation state, or changes to the
generic patch-combination/intersection contract. Multiple apply_patch events
may contain identical content, so restarting synthetic positions for every
file or event would allow the existing combination identity to collide.

T16's implementation and verification established deterministic normalization
from the stable `tool_use_id`, checked allocation across all emitted files and
hunks, safe failure on invalid identities or exhausted ranges, and successful
matching through the unchanged historical `kind` + `content` intersection
fallback. See the task record and its focused normalization/intersection tests.

## Decision

Represent Codex apply_patch Add/Update touched lines with deterministic,
event-scoped synthetic line identities derived from a domain-separated SHA-256
of the trimmed `tool_use_id`. Allocate checked local offsets across the entire
normalized event within a bounded range. These values are evidence identities,
not source line numbers, and are consumed through the existing
`combine_patches` and `intersect_patches` behavior.

## Rationale

Hashing the stable event identity gives repeated normalization of one event the
same result while separating independent events with overwhelming probability.
A bounded range and checked arithmetic make allocation deterministic and prevent
an oversized event or arithmetic failure from producing unsafe evidence. The
approach preserves exact touched-line content and order without claiming
unknown physical positions, and the existing content fallback can reconcile
synthetic positions with real committed line numbers.

## Alternatives considered

- **Use Codex line ranges as real positions** — Rejected because the hook payload
does not provide trustworthy ranges for this integration.
- **Restart positions at one for each file or event** — Rejected because
identical evidence would collide in the existing patch-combination identity.
- **Take a filesystem snapshot or add pending/snapshot state** — Rejected because
that would expand the runtime ownership and persistence model beyond the
approved no-snapshot pipeline.
- **Change generic `combine_patches` or `intersect_patches` semantics** —
Rejected; Codex evidence can use the existing historical content fallback.

## Compatibility and risks

- The synthetic positions are compatible with the existing SCE unified-diff
parser and downstream intersection, but they must never be rendered or
interpreted as physical source line numbers.
- Hash-range separation has a documented negligible collision risk. Invalid,
missing, untrimmed, or overflowed identities fail open without persistence.
- The content fallback can match repeated identical lines ambiguously because
Codex supplies no physical occurrence information; later context and tests must
state that limitation rather than claim occurrence-level certainty.

## Guardrails

- Derive identities only from the stable `tool_use_id`; do not use time,
randomness, filesystem paths, or mutable repository state.
- Keep allocation event-scoped, bounded, and checked across all emitted
operations, hunks, and files.
- Do not add database columns, migrations, snapshots, pending state, or a
Codex-specific intersection algorithm for this identity scheme.
- Keep Delete File and changeless Move evidence out of line-level persistence.

## Consequences

- Separate same-content Codex events survive existing patch combination and can
both be consumed by the current post-commit intersection pipeline.
- Codex evidence remains useful when committed line numbers differ, but repeated
identical content can remain physically ambiguous.
- Future Codex hook changes must preserve the distinction between evidence
identity and source location when adapting this path.

## Follow-up

None.

## References

- Plan: [`codex-cli-integration`](../plans/codex-cli-integration.md)
- Task: `T16`
- Current-state context: [`Codex hook runtime`](../sce/codex-integration-runtime.md)
- Current-state context: [`Agent Trace hooks command routing`](../sce/agent-trace-hooks-command-routing.md)
- Evidence: [`normalize.rs`](../../cli/src/services/hooks/codex/apply_patch/normalize.rs)
- Evidence: [`T16 completed task record`](../plans/codex-cli-integration.md)
- Related context: [`patch service`](../cli/patch-service.md)
