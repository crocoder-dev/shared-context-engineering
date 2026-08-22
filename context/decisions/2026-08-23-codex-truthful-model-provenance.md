# Decision: Preserve Codex model IDs without inferring a provider

Date: 2026-08-23
Status: Accepted
Plan: `context/plans/codex-cli-integration.md`
Task: `T17`

## Context

The Codex hook payload exposes a `model` value but no separate trustworthy
provider field. Prefixing every unqualified value with `openai/` would turn an
unverified assumption into persisted Agent Trace provenance and could mislabel
custom or future Codex model identifiers. Blank or absent values also need to
remain distinguishable from reported attribution.

T17's implementation and verification covered already-qualified IDs,
custom-qualified IDs, unqualified IDs, blank values, and missing values through
Codex diff-trace persistence and model-normalization tests. The resulting
values are consumed by the existing Agent Trace attribution pipeline without a
new schema field or provider-inference path.

## Decision

For Codex events, trim the reported `model` value, persist it unchanged when
non-empty, and persist `None` when it is absent or blank. Do not infer or add a
provider prefix, and do not invent a separate provider field.

## Rationale

Preserving the producer's value is the only truthful transformation available
when the payload does not identify a provider independently. It retains useful
custom and qualified identifiers, avoids false OpenAI attribution, and keeps
missing provenance explicit for downstream Agent Trace rendering.

## Alternatives considered

- **Prefix every unqualified value with `openai/`** — Rejected because the
  payload does not establish that provider identity for every model string.
- **Infer a provider from model-name patterns** — Rejected because pattern
  matching would be speculative and would create unstable provenance.
- **Add a provider field to Codex persistence** — Rejected because the upstream
  payload exposes no trustworthy separate provider value and the existing
  schema does not require a new field.

## Compatibility and risks

- Existing already-qualified IDs remain byte-for-byte unchanged; newly
  persisted unqualified IDs no longer carry the previously fabricated
  `openai/` prefix.
- Downstream consumers must treat an unqualified non-empty ID as producer-
  reported but provider-unspecified. Blank and missing values remain nullable.
- A future upstream provider field may require a new decision and explicit
  schema/consumer work; this record does not authorize provider inference.

## Guardrails

- Apply this normalization only to Codex model values; other producers retain
  their existing model conventions.
- Trim only surrounding whitespace and never rewrite the model's remaining
  content.
- Keep provider identity out of the Codex event model and Agent Trace schema
  unless upstream supplies trustworthy data and a separate change approves it.

## Consequences

- Codex Agent Trace attribution is truthful but may be provider-unspecified for
  unqualified custom model IDs.
- Existing downstream storage and intersection flows remain unchanged, with
  `model_id` carrying the raw reported value or `NULL`.
- Tests and runtime documentation must preserve the distinction between a
  model identifier and a provider-qualified identifier.

## Follow-up

None.

## References

- Plan: [`codex-cli-integration`](../plans/codex-cli-integration.md)
- Task: `T17`
- Current-state context: [`Codex hook runtime`](../sce/codex-integration-runtime.md)
- Current-state context: [`Agent Trace hooks command routing`](../sce/agent-trace-hooks-command-routing.md)
- Evidence: [`hooks/mod.rs`](../../cli/src/services/hooks/mod.rs)
- Evidence: [`apply_patch/mod.rs`](../../cli/src/services/hooks/codex/apply_patch/mod.rs)
- Related decision: [`Use event-scoped synthetic identities for Codex apply_patch evidence`](2026-08-23-codex-event-scoped-apply-patch-evidence-identities.md)
