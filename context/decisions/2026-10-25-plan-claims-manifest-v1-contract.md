# Decision: Publish the intent-only sce.plan.claims.v1 claims manifest

Date: 2026-10-25
Status: Proposed
Plan: `context/plans/plan-review-claims.md`
Task: `T02, T03`

## Context

External assessors such as Jev need a machine-readable view of what a plan author intends a change to prove. The plan service already extracts and validates plans, and `sce plan review` already reads plans from a commit. The output must stay stable enough that assessors can join their own judgments to it across commits. Plan design decisions D2, D3, and D4 and the `## Claims JSON v1 contract` section define the shape.

## Decision

`sce plan review --claims` prints the versioned `sce.plan.claims.v1` JSON claims manifest on stdout as an external compatibility contract. The manifest records authored intent only. Each acceptance criterion gives one claim with the identity `<plan-path>#<AC-id>`, `kind` `acceptance_criterion`, and `modality` `intended`. The manifest has no evidence, verdict, confidence, risk, score, or priority field. Plans come only from the `--head` commit. External assessors join their judgments on `repository.slug`, `repository.head_sha`, and `claims[].id`. An incompatible change requires a new schema version.

## Rationale

The plan path and the AC ID stay stable across text edits and commits, because plan rules never renumber or reuse AC IDs. Judgments stay outside the manifest, so the authored source stays authoritative and an assessor can change without a schema change. Commit-backed reads make every line anchor and URL match `--head`.

## Alternatives considered

- **An identity that includes the commit SHA or line number** — every commit or edit changes it.
- **A content-hash identity** — any wording fix changes it.
- **A `verified` modality from checked boxes** — a checkbox is not execution evidence.
- **A `--format json` option or a separate `sce plan claims` command** — the change request requires one review command with a boolean selector.

## Compatibility and risks

- A plan rename or a criterion moved to another plan changes the claim identity, so assessor joins on the old identity break.
- A plan that `sce plan check` accepts can still fail `--claims` when an entry repeats a consumed field.
- D4 carries an open review question about whether `<plan-path>#<AC-id>` is an acceptable stable identity. This record stays `Proposed` until that review ends.

## Guardrails

- The command never runs `Validate:` or `Check:` content, never writes files, and never calls a model or the network.
- Typed `#[derive(Serialize)]` structs in `cli/src/services/plan/claims.rs` fix the property order. The module uses no `serde_json::Value` map and no `skip_serializing_if`.
- The Decision Brief limits and Markdown escaping never apply to the manifest.

## Consequences

- Assessors can attach evidence and verdicts without changes to the plan service.
- An acceptance criterion with several assertions stays one claim until a future atomization step exists.
- Each new field that claims need requires a typed-structure change, and an incompatible change requires a new schema version.

## Follow-up

- Resolve the D4 review question on claim identity.
- T04 documents the manifest contract in `context/cli/plan-service.md`.

## References

- Plan: [`plan-review-claims`](../plans/plan-review-claims.md)
- Task: `T02`, `T03`
- Current-state context: [`plan-service.md`](../cli/plan-service.md)
- Current-state context: [`cli-command-surface.md`](../cli/cli-command-surface.md)
- Evidence: [`claims.rs`](../../cli/src/services/plan/claims.rs)
- Evidence: [`command.rs`](../../cli/src/services/plan/command.rs)
