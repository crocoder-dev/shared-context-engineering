# Plan: mutation-scope-provenance

## Change summary

Add durable **scope provenance** to the mutation-scope attribution pipeline so
AI-authored filesystem mutations discovered through mutation tracing keep the
session and model that produced them. Today the mutation runtime proves that a
filesystem transition belongs to one exclusive AI scope
(`AiExclusive(scope_id)` -> `LineProvenance::MutationAi { scope_id }`), but
`cli/src/services/mutation_trace/runtime/mutation_attribution.rs:369` matches
that variant with `{ .. }` and discards the `ScopeId`. The mutation-derived
patch therefore reaches Agent Trace as anonymous AI evidence, and
`cli/src/services/agent_trace.rs` binds both `Conversation.contributor.model_id`
and `Conversation.related` to the direct intersection only. A file created
through `Bash` is classified `ai`, but carries no model and no session link.

This plan preserves enough metadata to resolve that `ScopeId` later. A new
insert-once table `mutation_trace_scope_provenance`
(`005_mutation_scope_provenance.sql`) maps `scope_id -> session_id + model_id?`.
Both shipped producers populate it: Codex reads `model` straight off its
`PreToolUse` payload (already present in the probe fixtures), and Claude
snapshots the existing exact `claude_model_state(cc_<session>, agent_id)`
register at admission through an injectable resolver seam. Post-commit, the
mutation projection resolves the scope against that provenance and annotates
`TouchedLine.session_id` plus a conservatively derived `PatchHunk.model_id`;
Agent Trace then unions direct and mutation session links and selects a model
only when the contributing evidence agrees.

This extends existing behavior. The verified mutation protocol, the Quint model,
the mutation attribution algorithm, `mutation_trace_scopes`, and
`config/schema/agent-trace.schema.json` are all unchanged. Provenance is
observational metadata about an already-established scope; it never participates
in deciding `AiExclusive` / `AiContended` / `IneligibleUnscoped`.

## Stack and base

- **Predecessor:** PR #268 `Codex mutation-scope integration`, branch
  `codex-mutation-scope-integration`.
- **This branch:** `mutation-scope-provenance`, created from `c3bee66c`, which
  was PR #268's head at plan creation. `c3bee66c` is the base the branch was cut
  from, not its current tip.
- **PR base while the stack is unmerged:** `codex-mutation-scope-integration`,
  not `main`.
- Compare the completed PR against `origin/codex-mutation-scope-integration`. If
  #268's head moves during implementation, rebase `mutation-scope-provenance`
  onto the latest #268 head before final validation.
- **PR:** #275, already open and titled `Mutation scope provenance`. No title
  change is required.

## Design

### D1 — Scope provenance is metadata about a scope, not mutation-protocol state

The verified protocol keeps owning `ScopeId`, `WorktreeId`, `ActorKind`,
`ScopeStatus`, and the `IneligibleUnscoped` / `AiExclusive(scope_id)` /
`AiContended` decision. Provenance answers a different question: given
`scope_id`, which session and model did that scope represent?

```rust
struct ScopeProvenance {
    scope_id: ScopeId,
    session_id: String,
    model_id: Option<String>,
}
```

Do **not** add `session_id`, `model_id`, `agent_id`, or any harness-specific
field to `ScopeState`, `MutationEvent`, `Attribution`, `ProtocolState`, or the
Quint model. The pure protocol must keep working when provenance is entirely
absent.

### D2 — One shared durable provenance table

`cli/migrations/agent-trace-repository/005_mutation_scope_provenance.sql`:

```sql
CREATE TABLE mutation_trace_scope_provenance (
    scope_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    model_id TEXT,
    created_at TEXT NOT NULL ...
);
```

`scope_id` is the identity boundary. Do not duplicate `actor_kind`,
`worktree_id`, or `status` — those stay owned by `mutation_trace_scopes`. Do not
persist `agent_id`; Claude uses it only to resolve the correct model state
before the snapshot is taken.

`session_id` is the canonical SCE session identity produced by the existing
`prefixed_diff_trace_session_id` helper (`cc_` for Claude, `cx_` for Codex).
`model_id` is already normalized by the producer-specific helper and may be
`NULL`.

### D3 — Session identity is immutable; the model is first-observed metadata

Provenance has two parts with different strengths:

- `session_id` is the **immutable identity fact**. A `scope_id` belongs to
  exactly one session, permanently.
- `model_id` is **immutable first-observed descriptive metadata**. It describes
  the actor as it appeared when the scope was first admitted, and is *not* part
  of scope identity.

Persistence is insert-once and the first persisted provenance row wins for
`model_id`. A scope created while Claude used Sonnet stays Sonnet after a switch
to Opus; a later `claude_model_state` lookup never rewrites it. Symmetrically, a
later *discovery* of a model for a scope first recorded with `model_id = NULL`
never backfills it.

Exact replay/conflict matrix for an incoming provenance registration:

```text
existing: session S, model X
incoming: session S, model X
=> success, idempotent no-op

existing: session S, model NULL
incoming: session S, model X
=> success, keep NULL

existing: session S, model X
incoming: session S, model NULL
=> success, keep X

existing: session S, model X
incoming: session S, model Y
=> success, keep X

existing: session S1, ...
incoming: session S2, ...
=> identity conflict / error
```

That matrix governs an incoming registration for a scope that **already has** a
provenance row. Whether a row may be *created* in the first place is D4's
admission-bounded rule.

A model disagreement or a later model discovery must **never** cause
mutation-scope admission to fail. This matters because provenance does not
participate in the correctness of `AiExclusive`, `AiContended`, or
`IneligibleUnscoped`: a disagreement about descriptive metadata is not a reason
to deny a mutation-capable tool.

The **only** provenance conflict that fails a `Start` is a `scope_id` being
associated with a different `session_id`. That returns an error and never
rewrites the stored row — session identity is never silently rewritten.

`model_id = NULL` is valid provenance. Missing model information never
invalidates otherwise valid AI mutation attribution.

### D4 — Provenance is optional on the generic Start contract, with explicit durable ordering

Extend only the generic `start` ingress shape in
`cli/src/services/hooks/mutation_scope.rs`:

```json
{
  "operation": "start",
  "scope_id": "...",
  "event_id": "...",
  "actor_kind": "codex",
  "provenance": { "session_id": "cx_...", "model_id": "..." }
}
```

`provenance` itself is optional, so producers that do not yet supply it keep
working. When present: `session_id` is required and non-blank; `model_id` is
optional/nullable; unexpected provenance keys are rejected by the existing
`reject_unexpected_keys` discipline; provenance is accepted only for `start`.
`advance`, `close`, `flush`, and `abandon` never update provenance.

`RuntimeBoundary::Start` may carry this optional metadata, but the pure protocol
`Boundary::Start` is unchanged.

**Durable Start ordering.** A `Start` carrying provenance runs in exactly this
order, and this ordering stays inside the existing protected-worktree boundary:

```text
initialize worktree
    ↓
register scope(scope_id, worktree_id, actor_kind) -> ScopeState
    ↓
conditionally register provenance(scope_id, session_id, model_id?)
    ↓
run pure protocol prepare/commit for Start
```

Invariants:

- provenance registration requires that `mutation_trace_scopes` already contains
  the owning `scope_id`;
- provenance registration must not create a scope implicitly;
- a provenance row must never exist without an owning `mutation_trace_scopes`
  row;
- if provenance registration fails before the protocol `Start` commits, the
  `Start` fails;
- an already-created `NeverSeen` scope row is acceptable after such a failure,
  matching the runtime's existing register-before-protocol behavior;
- provenance remains outside `ProtocolState` and is not part of the CAS
  transition;
- replaying a `Start` with identical session provenance remains safe and
  idempotent.

**Provenance creation is admission-bounded.** `ScopeId` proves mutation
ownership; `ScopeProvenance` describes the owning scope **as observed at
admission**. It is therefore insert-once *and* creation-bounded, and must never
be attached retroactively to a scope whose protocol `Start` already committed.

The middle step is conditional, decided from the `ScopeState` that
`register_scope` returns together with the existing provenance row:

```text
provenance row exists
    -> register as usual; D3's replay/conflict matrix is authoritative
       (same session -> success, first persisted model wins;
        different session -> identity conflict / error)

no provenance row + scope.status == NeverSeen
    -> register the incoming provenance

no provenance row + scope.status == Active or terminal
    -> do nothing; provenance stays absent
       the Start continues through normal replay / guarded behavior
```

Once a scope has transitioned beyond `NeverSeen`, absence of provenance is
permanent. A later `Start` replay carrying provenance is not an error — it
follows the protocol's ordinary replay semantics and simply persists nothing.

This closes a retroactive-attachment hole. Without it, a `Start` that admitted a
scope with no provenance could gain one from any later replay; for Claude that
means a replay after a `PostModelSwitch` could resolve the *newer* model and
attach it to an *older* scope.

Two deliberate consequences:

- The rule keys on **durable scope status**, not on "the scope row already
  exists", so the legitimate retry survives: a first attempt that registered the
  scope but never committed the protocol `Start` leaves it `NeverSeen`, so the
  retry may still register provenance and then commit.
- Only *creation* is bounded, not *validation*. An existing provenance row is
  loaded and re-registered on every provenance-carrying `Start` whatever the
  scope's status, so a replay of an already-admitted scope naming a different
  `session_id` still fails as an identity conflict.

None of this changes the pure protocol or the Quint design; the ordering lives
entirely in the runtime adapter layer.

A missing model is not an error, and neither is a model that disagrees with an
already persisted row (D3). The `Start` fails only on a malformed provenance
payload, a `scope_id` already bound to a different `session_id`, or a failure to
durably register provenance for a registered scope.

### D5 — Codex populates provenance directly from PreToolUse

Codex `PreToolUse` already carries `session_id`, `model`, and `tool_use_id` —
confirmed present in
`cli/src/services/hooks/codex_mutation_scope/fixtures/*.pre_tool_use.json`.
Extend the Codex parser so `CodexToolExecution` retains the model (it currently
holds only `identity`, `agent_type`, and `tool_input`).

On first admission of a tracked Codex mutation tool, canonicalize the session to
`cx_<session>` via `prefixed_diff_trace_session_id` and normalize the model via
the existing `normalize_codex_model_id` in `cli/src/services/hooks/mod.rs:1103`,
then send that `ScopeProvenance` with the generic `Start`. Both `Bash` and
`apply_patch` receive provenance because both are tracked mutation scopes.

Direct `apply_patch -> diff_traces` attribution is unchanged and keeps taking
precedence during post-commit direct-coverage exclusion; scope provenance never
causes the same lines to be counted twice.

### D6 — Claude resolves its model through an explicit injectable seam

Claude `PreToolUse` gives the adapter `session_id`, optional `agent_id`, and
`tool_use_id`, but normally not the model. The Claude mutation adapter does not
directly own Agent Trace DB access — the model-state lookup currently lives in
the normal Claude diff-trace path — so the model is resolved through an explicit
injectable lookup seam rather than the adapter querying the database itself:

```rust
type ClaudeModelStateResolver =
    Fn(repository_root, canonical_session_id, agent_id) -> Result<Option<String>>;
```

The exact Rust type and name may vary during implementation. What matters is
that it is an injectable seam handed to the adapter, so tests can supply a
resolver returning a model, `None`, or an error without a real database.

Flow:

```text
Claude PreToolUse
    │
    ├─ raw session_id
    └─ agent_id?
          │
          ▼
canonicalize to cc_<session>
          │
          ▼
ClaudeModelStateResolver
          │
          ▼
existing repository Agent Trace DB
claude_model_state_by_session_and_agent(
    cc_<session>,
    exact agent_id or ""
)
          │
          ▼
model_id?
          │
          ▼
ScopeProvenance
          │
          ▼
generic mutation Start
```

Semantics:

- the main agent uses `agent_id = ""`; a subagent uses the exact `agent_id`;
- the lookup is exact — a subagent never inherits main-agent state, mirroring
  the existing `resolve_diff_trace_model_id` rule at
  `cli/src/services/hooks/mod.rs:1388`;
- `Ok(None)` means `model_id = NULL`;
- missing model state is not an admission failure;
- model normalization and `claude_model_state` semantics reuse the existing
  Claude machinery unchanged;
- a later `PostModelSwitch` changes current Claude state but never historical
  `ScopeProvenance`.

**Resolver failures.** Keep these two conditions separate:

```text
model unavailable
```

and

```text
mutation-scope Start could not be established
```

The inability to establish a *model value* — no row, a blank row, an
unnormalizable value — always degrades to `model_id = NULL`. A genuine
infrastructure failure while performing the local lookup (the repository
database cannot be opened or read at all) is a different condition, reported as
such by the resolver's `Err`; the adapter still admits the scope and records
`model_id = NULL`. Model availability is never a requirement for safe mutation
attribution, and failure to identify a model is never a reason to deny a
mutation-capable tool. Only the failures named in D3/D4 deny a `Start`.

The generic `mutation_scope` ingress stays harness-neutral: all Claude-specific
DB and model-state resolution happens before the generic `Start` payload is
constructed.

### D7 — Preserve ScopeId until mutation evidence is enriched

`mutation_attribution.rs:369` currently matches `LineProvenance::MutationAi { .. }`
and inserts a bare `PatchLineLocation`, and `attribution::patch_for_locations`
then clones lines straight off the committed target patch. Neither step can
carry provenance. Keep the `scope_id` alongside each selected location, resolve
`ScopeProvenance(scope_id)` once per distinct scope, and build the mutation
patch with the resolved metadata:

- `TouchedLine.session_id = provenance.session_id` for every mutation-attributed
  line.
- `PatchHunk.model_id` is set only when **all** mutation-attributed lines
  contributing to that hunk resolve to the same non-null model.

```text
scope A -> X, scope A -> X   => hunk.model_id = X
scope A -> X, scope B -> X   => hunk.model_id = X
scope A -> X, scope B -> Y   => hunk.model_id = NULL
scope A -> X, scope B -> ?   => hunk.model_id = NULL
```

The attribution algorithm itself is unchanged; only the projection of already
attributed `MutationAi(scope)` lines gains metadata. A provenance row that is
absent is simply an unknown model with no session, and must not downgrade the
line's AI classification.

### D8 — Agent Trace combines direct and mutation provenance

`build_trace_file(...)` at `cli/src/services/agent_trace.rs:556` already receives
both `intersection_patch` and `mutation_ai_patch` and locates the matching file
for each. Extend it so mutation evidence contributes provenance as well.

Related sessions become the distinct union of session IDs found on the matched
direct/intersection hunk and the matched mutation hunk, so a mutation-only Bash
hunk can emit a session link.

Model attribution follows one conservative agreement rule across both sources:

```text
direct-only X               -> X
mutation-only X             -> X
mutation-only unknown       -> NULL
direct X + mutation X       -> X
direct X + mutation Y       -> NULL
direct X + mutation unknown -> NULL
```

An **absent** evidence source does not count as unknown, so a direct-only hunk
keeps its current `model_id` exactly.

`direct X + mutation unknown -> NULL` is settled and deliberate.
`Contributor.model_id` describes the contributor for the whole attributed
hunk/range. If some mutation-attributed AI lines in that range have an unknown
model, preserving direct model `X` would overclaim that *all* AI contribution
came from `X`. Dropping the model is the honest answer; the hunk keeps its `ai`
classification and its related sessions. Mixed direct + mutation hunks therefore
gain stricter model semantics by design — see AC7.

Do not change `Contributor`, `ConversationRelated`, `AgentTrace`, or
`config/schema/agent-trace.schema.json`.

### D9 — Direct evidence remains authoritative for direct coverage

The two evidence paths stay separate: direct tool evidence through `diff_traces`
and post-commit intersection, filesystem mutation evidence through mutation
scopes, events, and lineage replay. `attribution::exclude_direct_coverage` stays
in place before mutation attribution, so a Codex `apply_patch` change already
proven by its `diff_traces` row is not duplicated merely because the same tool
execution also had a mutation scope. Scope provenance exists for the scope;
mutation attribution still only contributes the portion not already covered
directly.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Durable scope provenance exists independently of verified scope
  state. Migration `005_mutation_scope_provenance.sql` provides insert-once
  `scope_id -> session_id + model_id?` storage that accepts a nullable model and
  stores canonical prefixed sessions verbatim. D3's matrix holds exactly:
  identical replay is an idempotent no-op; `existing NULL + incoming X` keeps
  `NULL`; `existing X + incoming NULL` keeps `X`; `existing X + incoming Y`
  succeeds and keeps `X`; only a different `session_id` for an existing
  `scope_id` errors, and it never rewrites the stored row. Inserting provenance
  for a `scope_id` unknown to `mutation_trace_scopes` fails, registration never
  creates a scope implicitly, and the supported provenance write path through
  `MutationTraceStore` requires an existing owning scope row and leaves no orphan
  row when that scope is missing. `mutation_trace_scopes` is structurally
  unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::agent_trace_db` and
    `services::mutation_trace::store`; `git diff
    origin/codex-mutation-scope-integration --
    cli/migrations/agent-trace-repository/004_mutation_trace_protocol.sql` is
    empty.
- [ ] AC2: The generic mutation `Start` accepts optional provenance without
  changing protocol semantics, in D4's order. `start` with valid provenance
  initializes the worktree, registers the scope, registers provenance, then runs
  the pure protocol prepare/commit, all inside the existing protected-worktree
  boundary; a valid registered scope accepts provenance; the protocol `Start` is
  not committed if provenance registration fails, and that failure leaves at
  most an owning `NeverSeen` scope row and no provenance row without a
  registered scope; `start` without provenance still works; a replayed `start`
  with identical session provenance is idempotent and still succeeds; a `start`
  whose provenance disagrees only on the model still succeeds. Provenance
  creation is admission-bounded: a row is created only while the durable scope
  is `NeverSeen`, so a `start` replayed against an already-admitted scope with
  no provenance row succeeds and still persists none, while a scope that is
  still `NeverSeen` after a failed earlier attempt may gain provenance on
  retry; an existing provenance row is still validated on every
  provenance-carrying `start` whatever the scope's status, so a different
  `session_id` remains a conflict. Provenance on
  `advance` / `close` / `flush` / `abandon` is rejected by the strict parser; a
  blank `session_id` or an unexpected provenance key is rejected. Provenance
  stays outside `ProtocolState` and the CAS transition, and pure mutation
  protocol tests are unchanged and passing.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::mutation_scope` and
    `services::mutation_trace::`; `git diff
    origin/codex-mutation-scope-integration --
    cli/src/services/mutation_trace/protocol.rs spec/` is empty.
- [ ] AC3: Codex tracked mutations persist exact scope provenance. A
  `PreToolUse(Bash)` fixture produces a scope whose provenance row holds the
  expected `cx_`-prefixed session and the normalized Codex model; `apply_patch`
  behaves identically; a `PreToolUse` with no usable model produces
  `model_id = NULL` while keeping session attribution.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`.
- [ ] AC4: Claude tracked mutations persist scope provenance from exact Claude
  model state. A main-agent scope resolves `(cc_<session>, "")`; a subagent scope
  resolves `(cc_<session>, exact agent_id)`; a subagent with no exact row gets
  `model_id = NULL` rather than the main agent's model; a `PostModelSwitch`
  after scope creation leaves the existing provenance row untouched.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::claude_mutation_scope` and
    `services::hooks::claude_model_state`.
- [ ] AC5: Mutation lineage preserves provenance into the mutation-derived
  patch. `MutationAi(scope_id)` lines carry their scope's canonical
  `session_id`; a hunk carries a model only when every mutation-attributed line
  in it resolves to the same known model; conflicting-model,
  partially-unknown, and missing-provenance hunks leave `model_id` unset;
  existing AI / non-AI / unresolved classification results are unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::mutation_trace::runtime::mutation_attribution` and
    `services::mutation_trace::`.
- [ ] AC6: Agent Trace emits mutation-derived session and model attribution. A
  Codex `Bash`-created file yields `contributor.type = "ai"`, the normalized
  Codex `model_id`, and a related `cx_` session URL; a Claude `Bash`-created
  file yields the equivalent Claude model and `cc_` session URL; mutation-only
  evidence works with no direct `diff_traces` match; mixed direct + mutation
  evidence unions related sessions and emits a model only when the contributing
  evidence agrees.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::agent_trace` and `services::hooks::`.
- [ ] AC7: Direct-only attribution and direct-coverage precedence are unchanged.
  Direct-only Codex `apply_patch` evidence behaves exactly as before, producing
  its existing `diff_traces` session and model attribution; direct-only Claude
  structured evidence behaves exactly as before; the existing Claude model
  precedence remains `direct > exact transcript > exact session/agent state >
  NULL`; a hunk with direct evidence and no mutation evidence keeps exactly the
  `model_id` and `related` it emits today, and direct-only golden output is
  byte-identical; direct coverage is still excluded before mutation-derived
  attribution. Mixed direct + mutation hunks are deliberately outside this
  criterion: they follow D8's agreement rule and may therefore lose `model_id`
  when mutation provenance is conflicting or unknown. That is an intended
  semantic change, not a regression.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::` and `services::agent_trace`; the
    `direct_only` golden fixture under
    `cli/src/services/agent_trace/fixtures/` is byte-unchanged.
- [ ] AC8: No verified-protocol or Agent Trace schema expansion is introduced.
  `spec/mutation_cursor.qnt`, `spec/mutation_cursor.md`,
  `cli/src/services/mutation_trace/protocol.rs`, the pure protocol `ScopeState` /
  `MutationEvent` attribution types, and
  `config/schema/agent-trace.schema.json` have no semantic change. Migration
  `005` is the only schema addition and is observational metadata only.
  - Validate: `git diff origin/codex-mutation-scope-integration -- spec/
    cli/src/services/mutation_trace/protocol.rs
    config/schema/agent-trace.schema.json` is empty; `git diff --name-only
    origin/codex-mutation-scope-integration --
    cli/migrations/agent-trace-repository/` lists only
    `005_mutation_scope_provenance.sql`.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::codex_mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_db`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff origin/codex-mutation-scope-integration -- spec/ cli/src/services/mutation_trace/protocol.rs config/schema/agent-trace.schema.json` must be empty (AC8).

Final branch comparison is against `origin/codex-mutation-scope-integration`
(#268 head), not `main`, while this PR remains stacked on #268.

### Context sync

- Update `context/cli/mutation-scope-hook-ingress.md` — the `start` operation's
  accepted key set now includes optional `provenance`, with its validation rules
  and the "only `start`" restriction.
- Update `context/cli/mutation-scope-runtime.md` — `RuntimeBoundary::Start`
  carries optional provenance, plus D4's durable `Start` ordering (worktree init
  -> scope registration -> conditional provenance registration -> pure protocol
  prepare/commit) and its invariants: provenance needs an existing owning scope,
  never creates one implicitly, never outlives one, and stays outside
  `ProtocolState` and the CAS transition.
- Update `context/cli/mutation-scope-provenance.md` — D4's admission-bounded
  creation rule: a provenance row may be created only while the durable scope is
  `NeverSeen`; after admission, absent provenance stays absent permanently and a
  later `Start` replay cannot backfill it; an existing row is still validated on
  every provenance-carrying `Start`, so a different `session_id` remains an
  identity conflict.
- Update `context/cli/claude-mutation-scope-integration.md` — the injectable
  `ClaudeModelStateResolver` seam, the exact `(cc_<session>, agent_id)`
  model-state snapshot at admission, the no-inheritance rule for subagents, and
  the separation between `model unavailable` (always `model_id = NULL`) and
  `mutation-scope Start could not be established`.
- Update `context/cli/codex-mutation-scope-integration.md` — `PreToolUse.model`
  is now retained and normalized into scope provenance for both tracked tools.
- Update `context/cli/mutation-trace-agent-attribution.md` — the "No fabricated
  provenance" paragraph changes: the mutation-AI patch now carries resolved
  session and conservatively derived model metadata, while `ScopeId`,
  `ActorKind`, and `AiExclusive(scope)` still never become direct provenance.
- Update `context/cli/mutation-trace-store.md` — the new provenance read/write
  seam beside the existing mutation-trace persistence.
- Update `context/sce/agent-trace-db.md` — migration `005`, the provenance table
  shape, its insert-once semantics including D3's replay/conflict matrix and the
  owning-scope requirement, and that it is local attribution state outside the
  four export streams.
- Update `context/sce/agent-trace-minimal-generator.md` — `model_id` and
  `related` are no longer bound to the direct intersection only; record D8's
  combined agreement rule, including that a mixed direct + mutation hunk with
  conflicting or unknown mutation provenance intentionally emits no `model_id`.
- Update `context/sce/agent-trace-hooks-command-routing.md` — the `start`
  payload's new optional field.
- Update `context/architecture.md`, `context/glossary.md`,
  `context/context-map.md` — add `ScopeProvenance` and state the boundary
  explicitly: mutation protocol attribution `!=` scope provenance; `ScopeId`
  proves ownership, `ScopeProvenance` describes the owning scope.
- ADR: only if implementation reveals a genuinely new system-wide constraint.
  Adding one observational metadata table beside a verified protocol does not by
  itself meet the repository's ADR threshold.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-repository/005_mutation_scope_provenance.sql`;
  `cli/src/services/agent_trace_db/repository.rs`;
  `cli/src/services/mutation_trace/store.rs`;
  `cli/src/services/mutation_trace/runtime/` (`coordinator.rs`,
  `mutation_attribution.rs`, `mod.rs` re-exports);
  `cli/src/services/mutation_trace/attribution.rs`;
  `cli/src/services/hooks/mutation_scope.rs`;
  `cli/src/services/hooks/claude_mutation_scope/`;
  `cli/src/services/hooks/codex_mutation_scope/`;
  `cli/src/services/agent_trace.rs` and its fixtures; the durable context files
  named under **Context sync**.
- **Out of scope:** changing mutation exclusivity semantics or the
  `AiExclusive` / `AiContended` rules; adding model/session fields to the
  verified mutation protocol; adding `agent_id` to Agent Trace or `diff_traces`;
  exporting or synchronizing `claude_model_state` or the provenance table;
  model history; OpenCode/Pi mutation adapters; attributing currently untracked
  Codex MCP mutations; changing direct diff-trace capture; changing the Agent
  Trace JSON schema; changing mutation attribution of removed lines beyond
  current behavior.
- **Constraints:** provenance is insert-once per `ScopeId` — `session_id` is
  immutable identity and `model_id` is immutable first-observed metadata, with
  the first persisted row winning; only a `session_id` conflict fails a `Start`;
  canonical session IDs reuse the existing `prefixed_diff_trace_session_id`
  prefixes; model normalization reuses the existing producer helpers
  (`normalize_codex_model_id`, the Claude `claude/`-prefix normalization);
  unknown model means `NULL`, never guessed, and never blocks admission; the
  Claude subagent model lookup stays exactly scoped and runs behind an
  injectable resolver seam so the generic ingress stays harness-neutral; direct
  evidence stays excluded before mutation-derived attribution; provenance
  metadata never affects the formal attribution decision; the Codex adapter's
  only mutation-stack dependency remains the in-process
  `run_mutation_scope_from_payload` seam.
- **Non-goal:** merging the direct diff-trace system and mutation tracing into
  one evidence path. They stay two independent sources with direct coverage
  excluded first.

## Assumptions

- Every tracked Claude/Codex mutation scope has a stable harness session ID.
- Codex model information observed on `PreToolUse` describes that exact tool
  execution.
- Claude's best available scope-time model is the exact current
  `claude_model_state` value for `(session_id, agent_id)`, read through the
  injectable `ClaudeModelStateResolver` seam rather than by the mutation adapter
  querying the repository database itself. `claude_model_state` can legitimately
  be missing or stale because of Claude lifecycle timing; provenance records what
  SCE could establish at admission and claims no stronger causal ordering.
- A conflicting provenance insert (same `scope_id`, different `session_id`)
  returns an error to the caller rather than being silently ignored, and a
  fail-closed `Start` therefore denies the tool. This is the **only** provenance
  condition that denies a mutation-capable tool. It cannot occur in production
  for either shipped producer, because both `ScopeId` formats
  (`cc-tool-v1|…|s=<len>:<session>|…`, `cx-tool-v1|…|s=<len>:<session>|…`)
  embed the session length-prefixed in the identity itself.
- A model disagreement — same `scope_id` and `session_id`, a different or newly
  discovered `model_id` — is **not** a conflict. It succeeds, keeps the
  first-persisted value, and never denies the tool, because `model_id` is
  descriptive metadata that no attribution decision depends on.
- Model availability is not a precondition for safe mutation attribution.
  Neither a missing Claude model-state row nor an infrastructure failure while
  reading local model state can become a denied `Start`; both degrade to
  `model_id = NULL`.
- The existing `ParsedPatch` provenance fields are sufficient to carry mutation
  evidence to Agent Trace: `TouchedLine.session_id` (`patch.rs:84`) and
  `PatchHunk.model_id` (`patch.rs:66`).
- `Conversation.related` already represents multiple sessions, so the union in
  D8 needs no schema change.
- The single `Contributor.model_id` field means model disagreement inside one
  final Agent Trace hunk degrades to no model rather than an arbitrary pick.

## Task stack

- [x] T01: `Add durable mutation-scope provenance storage` (status:done)
  - Task ID: T01
  - Scope: In — `cli/migrations/agent-trace-repository/005_mutation_scope_provenance.sql`;
    the typed `ScopeProvenance` value and its insert/read API on
    `MutationTraceStore`, backed by `RepositoryAgentTraceDb`'s generic
    `execute`/`query_map` primitives; migration-readiness wiring. Out — any
    ingress, adapter, attribution, or Agent Trace change.
  - Dependencies: none
  - Done when: provenance can be inserted with a known model and with a `NULL`
    model and read back by `scope_id`; D3's matrix is proven row by row — an
    identical replay is an idempotent no-op, `existing NULL + incoming X` keeps
    `NULL`, `existing X + incoming NULL` keeps `X`, `existing X + incoming Y`
    succeeds and keeps `X`, and only a differing `session_id` for an existing
    `scope_id` returns an error without mutating the stored row; inserting
    provenance for a `scope_id` that `mutation_trace_scopes` does not contain
    fails, and registration never creates a scope implicitly, so the supported
    provenance write path requires an existing owning scope row and leaves no
    orphan row behind when that scope is missing.
    `AGENT_TRACE_REPOSITORY_MIGRATIONS` includes `005_mutation_scope_provenance`
    and schema readiness accounts for it.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_db`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store`.
  - Completed: 2026-09-09
  - Files changed: `cli/migrations/agent-trace-repository/005_mutation_scope_provenance.sql` (new); `cli/src/services/mutation_trace/store.rs`; `cli/src/services/agent_trace_db/repository.rs`
  - Result: Added migration `005_mutation_scope_provenance.sql` defining `mutation_trace_scope_provenance` (`scope_id TEXT PRIMARY KEY`, `session_id TEXT NOT NULL`, nullable `model_id TEXT`, `created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` matching `004`'s convention), discovered automatically by `build.rs`'s migrations directory scan, so `AGENT_TRACE_REPOSITORY_MIGRATIONS` gains `005_mutation_scope_provenance` and `TursoDb::ensure_schema_ready` derives its expected-migration set from that same list with no constant to edit. Added the typed `ScopeProvenance { scope_id: ScopeId, session_id: String, model_id: Option<String> }` value plus `MutationTraceStore::register_scope_provenance` and `MutationTraceStore::load_scope_provenance` in `store.rs`, alongside `SELECT_SCOPE_PROVENANCE_SQL`, `INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL` (`ON CONFLICT (scope_id) DO NOTHING`), and `scope_provenance_row_from_turso`, following the same access pattern the five `mutation_trace_*` tables from `004` already use. `register_scope_provenance` mirrors `register_scope`'s shape: it pre-checks the owning `mutation_trace_scopes` row via `load_scope` and bails before any insert when it is absent, performs the idle insert, then re-reads the stored row and returns it, erring only when the stored `session_id` differs from the incoming one. Insert-once therefore holds in both directions of `model_id` (stored `NULL` is not backfilled; a stored model is neither cleared nor overwritten) without any `UPDATE` path existing at all. Enforcing the owning-scope requirement in Rust rather than by a `FOREIGN KEY` left `004_mutation_trace_protocol.sql` byte-unchanged. `ScopeProvenance` lives in `store.rs`, not `types.rs`, so the pure-protocol type module is untouched; `#[allow(clippy::struct_field_names)]` follows the existing repository convention for `-D clippy::pedantic` (`attribution.rs`, `claude_mutation_scope/mod.rs`, `codex_mutation_scope/mod.rs`). Extended the three `repository.rs` migration-list assertions and the three schema-table lists to cover `005` / `mutation_trace_scope_provenance`, including the hook-runtime test proving the no-migration path still creates neither.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_db` — passed, 29/29; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store` — passed, 97/97 (9 new provenance tests). Also ran, though not required by the task: `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed; `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed.
  - Done checks: provenance inserts with a known model and with a `NULL` model and reads back by `scope_id` (verified — `register_scope_provenance_stores_a_known_model_and_reads_it_back`, `register_scope_provenance_stores_a_null_model_and_reads_it_back`); D3's matrix proven row by row — identical replay is an idempotent no-op leaving exactly one row (verified — `register_scope_provenance_replayed_identically_is_an_idempotent_no_op`), `existing NULL + incoming X` keeps `NULL` (verified — `register_scope_provenance_keeps_a_stored_null_model_when_one_is_later_discovered`), `existing X + incoming NULL` keeps `X` (verified — `register_scope_provenance_keeps_a_stored_model_when_the_replay_has_none`), `existing X + incoming Y` succeeds and keeps `X` (verified — `register_scope_provenance_keeps_the_first_model_when_a_later_one_disagrees`), and only a differing `session_id` errors while leaving the stored row byte-identical (verified — `register_scope_provenance_errors_on_a_session_conflict_without_rewriting_the_row`); provenance for a `scope_id` absent from `mutation_trace_scopes` fails, creates no provenance row, and creates no scope implicitly (verified — `register_scope_provenance_errors_for_an_unregistered_scope_and_creates_no_rows`, asserting both a zero provenance row count and `load_scope` still `None`); a scope without provenance reads back `None` rather than erroring (verified — `load_scope_provenance_returns_none_for_a_scope_without_provenance`); `AGENT_TRACE_REPOSITORY_MIGRATIONS` includes `005_mutation_scope_provenance` and schema readiness accounts for it (verified — `open_at_initializes_the_full_repository_schema` asserts the five-migration order and the new table then calls `ensure_schema_ready_for_hooks`, and `baseline_and_source_instance_fixture_migrates_to_mutation_trace_protocol_through_setup` proves a `001`+`002`-only fixture upgrades through `005`); `git diff --stat` over `004_mutation_trace_protocol.sql`, `protocol.rs`, `spec/`, and `config/schema/agent-trace.schema.json` is empty (verified).
  - Context impact: local — additive schema-only migration plus one new store seam that nothing consumes yet. No ingress, adapter, attribution, or Agent Trace behavior changed. T01 documented the new durable storage boundary in `context/cli/mutation-scope-provenance.md` (new), `context/cli/mutation-trace-store.md`, `context/sce/agent-trace-db.md`, and `context/context-map.md`. Those files currently document only the storage-layer provenance seam; later tasks extend the relevant context as mutation `Start` ingress, the Claude/Codex producers, mutation reconstruction, and Agent Trace consumption are implemented, and T07 still performs the final cross-system synchronization pass. T02 is the first consumer of this seam.
  - Context synchronization: synced

- [x] T02: `Carry optional provenance through mutation Start` (status:done)
  - Task ID: T02
  - Scope: In — `MutationScopePayload::Start` gains an optional provenance field;
    `parse_mutation_scope_payload` validation including the per-operation key
    sets; `RuntimeBoundary::Start` metadata plumbing through
    `runtime/coordinator.rs` and the `runtime/mod.rs` re-exports; durable
    registration through T01's seam in D4's order (worktree init -> scope
    registration -> conditional provenance registration -> pure protocol
    prepare/commit), inside the existing protected-worktree boundary, gated by
    D4's admission-bounded creation rule. Out — pure protocol
    `Boundary::Start`, `protocol.rs`, Quint semantics, and any harness adapter.
  - Dependencies: T01
  - Done when: `start` with valid provenance runs D4's order and durably
    registers provenance before the call reports success; provenance for an
    unknown `scope_id` fails while a valid registered scope accepts it; a
    provenance registration failure leaves the protocol `Start` uncommitted,
    leaves at most an owning `NeverSeen` scope row, and leaves no provenance row
    without a registered scope; a replayed `start` with identical session
    provenance succeeds idempotently; a `start` whose model disagrees with the
    stored row still succeeds; only a differing `session_id` for the same
    `scope_id` fails the `Start`; `start` without provenance behaves exactly as
    today; provenance creation is admission-bounded — a `start` replayed against
    an already-admitted scope with no provenance row succeeds under normal replay
    semantics and still persists no provenance, leaving the revision and
    processed-event set unchanged, while a scope still `NeverSeen` after a failed
    earlier attempt can still gain provenance on retry and then commit, and an
    existing provenance row is still validated on an admitted scope so a
    differing `session_id` still conflicts; provenance on `advance` / `close` / `flush` / `abandon` is rejected
    with the existing `Invalid mutation-scope payload from STDIN: <detail>.`
    diagnostic; a blank `session_id`, a non-object `provenance`, and an
    unexpected provenance key are each rejected; `git diff` against
    `origin/codex-mutation-scope-integration` for `protocol.rs` and `spec/` is
    empty.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`.
  - Completed: 2026-09-09
  - Files changed: `cli/src/services/hooks/mutation_scope.rs`; `cli/src/services/mutation_trace/runtime/coordinator.rs`; `cli/src/services/mutation_trace/runtime/mod.rs`; `cli/src/services/mutation_trace/runtime/tests.rs`
  - Result: Added `StartProvenance { session_id: String, model_id: Option<String> }` in `runtime/coordinator.rs`, re-exported from `runtime/mod.rs`, and gave `RuntimeBoundary::Start` a `provenance: Option<StartProvenance>` field. The value deliberately omits `scope_id` — the boundary already names the scope — and `coordinate_boundary_inner` composes T01's `ScopeProvenance` from both. Registration sits between the existing `register_scope` call and the CAS prepare/commit loop, so D4's order (worktree init -> scope registration -> provenance registration -> pure protocol prepare/commit) holds inside the existing protected-worktree boundary, and a retried CAS attempt re-runs no provenance write. Failures map to a new pre-commit `CoordinateError::ScopeProvenanceRegistration(anyhow::Error)`, displayed as its source and grouped with `ScopeIdentityConflict` / `LockAcquisition`; the hook's existing catch-all branch renders it as a boundary failure before durable completion, so a rejected `Start` denies the tool. `into_protocol_boundary` still maps to the unchanged pure `Boundary::Start`, and `provenance` never enters `ProtocolState` or the CAS transition. On the ingress side, `MutationScopePayload::Start` gained the same optional field; `parse_scope_boundary` was split so `start` (`parse_start`) accepts `provenance` in its key set while `advance` / `close` keep today's exact set and `flush` / `abandon` are untouched, all sharing the extracted `parse_scope_boundary_identity`. `parse_provenance` requires a JSON object, rejects any key other than `session_id` / `model_id` with an `unexpected field 'provenance.<key>'` diagnostic, requires a non-blank `session_id`, and reads `model_id` through a new `optional_non_blank_str` helper that treats an absent key and an explicit `null` as `None`. A blank or whitespace-only `model_id` is rejected rather than coerced to `NULL`, matching the strict-parser discipline: both shipped producers normalize an unknown model to `None` (`normalize_codex_model_id` already returns `Option`), so a blank string is malformed input, never a legitimate "no model". Every existing `RuntimeBoundary::Start` construction site in `coordinator.rs` and `runtime/tests.rs` gained `provenance: None`, which is also the exact-behavior path for a producer that sends no provenance. No harness adapter, `protocol.rs`, or `spec/` file was touched.
  - Amendment (2026-09-09, same task): the provenance step was made conditional so `ScopeProvenance` stays a true admission-time snapshot. `coordinate_boundary_inner` now binds the `ScopeState` that `register_scope` returns instead of discarding it, and passes it to a new `register_start_provenance(store, boundary, registered_scope)` helper (extracted so `coordinate_boundary_inner` stays under `clippy::too_many_lines`). That helper loads the existing provenance row first: when a row exists it always calls `register_scope_provenance`, so T01's immutable-session-identity and first-observed-model rules stay authoritative on every replay including for a long-admitted scope; when no row exists it registers only while `registered_scope.status == ScopeStatus::NeverSeen` and otherwise returns `Ok(())` without writing, letting the `Start` continue through the protocol's ordinary replay/guard behavior. The rule is keyed on durable scope status rather than on the scope row's existence precisely so a retry whose earlier attempt registered the scope but never committed the protocol `Start` can still register provenance. This closes a retroactive-attachment hole: previously `Start(A, provenance=None)` could admit a scope and a later `Start(A, provenance=Some(..))` replay would insert a row, which for Claude means a replay after a `PostModelSwitch` could attach the newer model to an older scope. Both the load failure and the registration failure map to the existing pre-commit `CoordinateError::ScopeProvenanceRegistration`. `ScopeStatus` was added to `coordinator.rs`'s `types` import; nothing else in the ordering, the error surface, the ingress, `ProtocolState`, the CAS transition, `protocol.rs`, or `spec/` changed.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope` — passed, 50/50 (13 provenance parser/ingress tests plus `test13`); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::` — passed, 350/350 (2 coordinator ordering tests plus 3 admission-boundedness regressions), including the unchanged Quint-refinement MBT suites. Also ran, though not required by the task: `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed; `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed.
  - Done checks: `start` with valid provenance runs D4's order and durably registers provenance before the call reports success (verified — `start_registers_provenance_after_its_owning_scope_and_before_the_protocol_commits` asserts the provenance row exists and the scope reached `Active`, and `test8_start_with_provenance_registers_it_before_the_protocol_start_commits` asserts the scope row, provenance row, and the committed `e1` processed event through the real-git/real-DB ingress); provenance for an unknown `scope_id` fails while a valid registered scope accepts it (verified — the store guard is T01's `register_scope_provenance_errors_for_an_unregistered_scope_and_creates_no_rows`, and the ingress side is proven by the two ordering tests above plus `test12_no_provenance_row_ever_exists_without_its_owning_scope`, whose LEFT JOIN asserts zero orphan rows); a provenance registration failure leaves the protocol `Start` uncommitted, leaves at most an owning `NeverSeen` scope row, and leaves no provenance row without a registered scope (verified — `a_provenance_registration_failure_rejects_the_start_before_the_protocol_commits` asserts `CoordinateError::ScopeProvenanceRegistration`, the scope still `NeverSeen`, empty `processed_events`, revision 0, and the stored row unchanged); a replayed `start` with identical session provenance succeeds idempotently and a `start` whose model disagrees still succeeds (verified — `test10_replayed_start_provenance_is_idempotent_and_keeps_the_first_model` replays identically, then with no model, then with a disagreeing model, asserting one row, the first model kept, and an unchanged revision and processed-event list); only a differing `session_id` for the same `scope_id` fails the `Start` (verified — `test11_conflicting_provenance_session_fails_the_start_without_rewriting_the_row` asserts the `already has provenance for session` diagnostic, the untouched row, and that event `e9` was never processed); `start` without provenance behaves exactly as today (verified — `test9_start_without_provenance_persists_no_provenance_row` asserts an `active` scope, zero provenance rows, and the committed `e1`, with `test1`–`test7` and every pre-existing runtime test unchanged); provenance on `advance` / `close` / `flush` / `abandon` is rejected with the existing diagnostic (verified — `provenance_is_rejected_on_every_operation_other_than_start` asserts the exact `Invalid mutation-scope payload from STDIN: unexpected field 'provenance'.` string for all four); a blank `session_id`, a non-object `provenance`, and an unexpected provenance key are each rejected (verified — `blank_or_non_string_provenance_session_id_is_rejected`, `non_object_provenance_is_rejected`, `unexpected_provenance_key_is_rejected`, plus `provenance_without_a_session_id_is_rejected` and `blank_or_non_string_provenance_model_id_is_rejected`); `git diff origin/codex-mutation-scope-integration -- cli/src/services/mutation_trace/protocol.rs spec/` is empty (verified — zero lines); provenance creation is admission-bounded (verified — `start_without_provenance_then_replay_with_provenance_does_not_backfill` admits scope `A` with `provenance: None`, asserts no provenance row, replays the same `Start` carrying `cc_session-1` / `claude/opus`, and asserts the provenance row is still absent with the worktree revision and processed-event set byte-identical to the admitted projection, and `test13_a_start_admitted_without_provenance_is_never_backfilled_by_a_replay` proves the same through the real-git/real-DB ingress with `SELECT COUNT(*) FROM mutation_trace_scope_provenance = 0`, the scope still `active`, and an unchanged revision); a retry before admission may still register provenance (verified — `never_seen_scope_can_receive_provenance_before_successful_start` seeds a `NeverSeen` scope row with no provenance, asserts that status, drives a `Start` carrying provenance, and asserts both the inserted row and the scope reaching `Active`); an existing provenance row is still validated after admission (verified — `an_admitted_scope_with_provenance_still_rejects_a_different_session` admits the scope with `cc_session-1`, replays with `cc_session-2`, and asserts `CoordinateError::ScopeProvenanceRegistration`, the unchanged stored row, and an unchanged revision and processed-event set; `test11_conflicting_provenance_session_fails_the_start_without_rewriting_the_row` covers the same through the ingress).
  - Context impact: local — one optional ingress field, one runtime boundary field, one new pre-commit error variant, and one durable registration step that reuses T01's seam. No harness adapter, attribution, or Agent Trace behavior changed, and the pure protocol and Quint model are byte-unchanged. T02 documents the ingress and runtime halves of the provenance path in `context/cli/mutation-scope-hook-ingress.md`, `context/cli/mutation-scope-runtime.md`, and `context/sce/agent-trace-hooks-command-routing.md`, and owns the admission-bounded creation rule in `context/cli/mutation-scope-provenance.md` (whose insert-once storage semantics remain T01's); the producer, reconstruction, and Agent Trace context files stay owned by T03–T06, and T07 still performs the final cross-system synchronization pass. T03 and T04 are the first producers of this optional field.
  - Context synchronization: synced

- [x] T03: `Populate Codex scope provenance` (status:done)
  - Task ID: T03
  - Scope: In — retain `model` on `CodexToolExecution` in the Codex `PreToolUse`
    parser; canonicalize the session with `prefixed_diff_trace_session_id` and
    normalize the model with `normalize_codex_model_id`; attach the resulting
    provenance to the `Start` payload for both tracked tools. Out — Codex tool
    classification, cleanup signals, `sce setup` registration, and the existing
    `sce hooks codex` diff/conversation pipeline.
  - Dependencies: T02
  - Done when: a `PreToolUse(Bash)` fixture and a `PreToolUse(apply_patch)`
    fixture each produce a provenance row with the expected `cx_` session and
    normalized model; a `PreToolUse` whose model is absent or unnormalizable
    produces `model_id = NULL` with the session still recorded; untracked and
    delegation tools still create no scope and no provenance.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::codex_mutation_scope`.
  - Completed: 2026-09-09
  - Files changed: `cli/src/services/hooks/codex_mutation_scope/mod.rs`
  - Result: `CodexToolExecution` gained a `model: Option<String>` field, read by `parse_pre_tool_use` through a new `tolerated_model` helper and the new `MODEL_FIELD` constant. That read is deliberately lenient rather than using the module's strict `optional_non_blank_str`: an absent, `null`, blank, or non-string `model` yields `None` instead of a parse error, because `model` is descriptive metadata and a malformed one must never fail-closed-deny a mutation-capable tool (D3), and because the task's own done check requires an unnormalizable model to produce `model_id = NULL` rather than a rejected event. Every other field on the Codex `PreToolUse` payload keeps its existing strict validation. Added `CodexScopeProvenance { session_id, model_id }` and `codex_scope_provenance(execution)`, which canonicalizes the session with `prefixed_diff_trace_session_id(CODEX_TOOL_NAME, ..)` (`cx_` prefix, already-prefixed sessions passed through) and normalizes the model with `normalize_codex_model_id` (trim, blank -> `None`); both helpers are reached through a new `use crate::services::hooks::{normalize_codex_model_id, prefixed_diff_trace_session_id, CODEX_TOOL_NAME}` — private-to-`hooks` items are visible to this descendant module, so no visibility was widened. `handle_pre_tool_use` builds the provenance once, after the Bash policy preflight and git-dir resolution and before the boundary lock, and hands it to `establish_start`, which now composes the payload through a new `scope_start_payload(scope_id, event_id, provenance)` instead of the shared `scope_boundary_payload`. Both tracked tools (`Bash`, `apply_patch`) take that one path, so both receive provenance. `scope_boundary_payload` is left byte-identical and now serves only `close`, keeping `close` / `abandon` / `flush` payloads unchanged — which matters because T02's ingress rejects `provenance` on every operation other than `start`. An unavailable model is emitted as JSON `null`, which T02's `optional_non_blank_str` parser already reads as `None`; a blank string is never emitted, so the ingress's blank-`model_id` rejection is unreachable from this producer. Whether a provenance row is actually created stays owned by T02's admission-bounded runtime rule; this task only supplies the optional field. No tool classification, cleanup signal, `sce setup` registration, or `sce hooks codex` diff/conversation behavior changed.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::codex_mutation_scope` — passed, 156/156 (10 new provenance tests). Also ran, though not required by the task: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::` — passed, 502/502; `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed; `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed.
  - Done checks: a `PreToolUse(Bash)` fixture and a `PreToolUse(apply_patch)` fixture each produce a provenance row with the expected `cx_` session and normalized model (verified — `test27_tracked_fixtures_persist_scope_provenance_ac3` drives `probe01`'s shell and apply_patch fixtures through the real-git/real-DB `CodexRepo` harness and asserts the persisted `mutation_trace_scope_provenance` row is exactly `("cx_01a07c1e-e08e-7172-8032-cb9d62af21d9", Some("gpt-5.6-sol"))` with a single row and an `active` codex scope; `ac3_pre_tool_use_fixtures_retain_the_codex_model` and `ac3_scope_provenance_canonicalizes_the_session_and_normalizes_the_model` prove the parser and helper halves, and `ac3_tracked_start_carries_scope_provenance_for_both_tracked_tools` asserts the emitted `start` payload's `provenance` object for every entry of `TRACKED_MUTATION_TOOL_NAMES`); a `PreToolUse` whose model is absent or unnormalizable produces `model_id = NULL` with the session still recorded (verified — `test28_a_tracked_execution_without_a_model_persists_a_null_model_ac3` asserts the persisted row is `("cx_session-no-model", None)`, `ac3_a_start_without_a_usable_model_still_carries_its_session` asserts `{"session_id":"cx_session-1","model_id":null}` for absent, `null`, blank, and non-string models, and `ac3_an_unusable_model_yields_no_model_id_without_rejecting_the_event` additionally covers a numeric and an object `model` without the event being rejected); untracked and delegation tools still create no scope and no provenance (verified — `test29_untracked_and_delegation_tools_persist_no_provenance_ac3` drives the MCP, `spawn_agent`, and `wait_agent` fixtures and asserts zero rows in both `mutation_trace_scopes` and `mutation_trace_scope_provenance`, with the pre-existing `untracked_mcp_pre_tool_use_creates_no_scope_and_never_touches_seam_or_git_dir` and `unknown_and_delegation_pre_tool_use_create_no_scope_ac3` still green). Additionally `ac3_only_the_start_boundary_carries_provenance` asserts the `close` payload carries no `provenance` key, and `ac3_scope_provenance_keeps_an_already_prefixed_session_id` proves an already-`cx_`-prefixed session is not double-prefixed.
  - Context impact: local — one producer now populates the optional `Start` field T02 already accepts. No storage, ingress, runtime, attribution, or Agent Trace behavior changed, and the pure protocol and Quint model are untouched. T03 documents the Codex producer half of the provenance path in `context/cli/codex-mutation-scope-integration.md` and extends the producer section of `context/cli/mutation-scope-provenance.md`; the storage semantics stay T01's and the admission-bounded creation rule stays T02's. T04 does the same for Claude, T05–T06 consume the stored rows, and T07 still performs the final cross-system synchronization pass.
  - Context synchronization: synced

- [ ] T04: `Populate Claude scope provenance` (status:todo)
  - Task ID: T04
  - Scope: In — introduce the injectable `ClaudeModelStateResolver` seam
    (`(repository_root, canonical_session_id, agent_id) -> Result<Option<String>>`;
    exact type and name may differ) and hand it to the Claude mutation adapter;
    canonicalize the Claude session to `cc_<session_id>`; back the production
    resolver with the existing repository
    `claude_model_state_by_session_and_agent` using the exact `agent_id` (`""`
    for the main agent) at admission; attach the snapshot to the `Start` payload
    before the harness-neutral generic ingress is entered. Out —
    `claude_model_state` write semantics, the `sce hooks claude-model-state`
    intake, giving the generic `mutation_scope` ingress any Claude-specific DB
    knowledge, and the Claude adapter's scope identity, cleanup, or recovery
    behavior.
  - Dependencies: T02
  - Done when: the adapter takes the resolver as an injected dependency and
    tests drive it without a real database; a main-agent scope resolves
    `(cc_<session>, "")`; a subagent scope resolves
    `(cc_<session>, exact agent_id)`; a subagent with no exact row records
    `model_id = NULL` instead of the main agent's model; a resolver returning
    `Ok(None)` records `model_id = NULL`; a resolver returning `Err` — an
    infrastructure failure reading local state — also records `model_id = NULL`
    and the `Start` still succeeds, keeping `model unavailable` distinct from
    `mutation-scope Start could not be established`; a `PostModelSwitch` applied
    after a scope is created leaves that scope's provenance row byte-identical.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_model_state`.
  - Context synchronization: pending

- [ ] T05: `Preserve ScopeId provenance through post-commit mutation reconstruction` (status:todo)
  - Task ID: T05
  - Scope: In — keep the `scope_id` alongside each AI-selected
    `PatchLineLocation` in `runtime/mutation_attribution.rs`; resolve
    `ScopeProvenance` once per distinct scope through T01's read seam; build the
    mutation-AI patch with `TouchedLine.session_id` set and `PatchHunk.model_id`
    derived by the all-lines-agree rule (extending or wrapping
    `attribution::patch_for_locations` rather than changing its direct-coverage
    behavior). Out — the attribution algorithm, the bounded-replay window, the
    lineage module's provenance propagation, and Agent Trace construction.
  - Dependencies: T01
  - Done when: a single-scope hunk carries that scope's session and model; two
    scopes with the same model still carry that model; two scopes with different
    models leave `model_id` unset; a scope with `model_id = NULL` and a scope
    with a known model leave `model_id` unset; a missing provenance row leaves
    session and model unset without downgrading the line's AI classification;
    multi-session hunks record each line's own session; existing AI / non-AI /
    unresolved classification counts in the current tests are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::mutation_attribution`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`.
  - Context synchronization: pending

- [ ] T06: `Emit mutation-derived provenance in Agent Trace` (status:todo)
  - Task ID: T06
  - Scope: In — `build_trace_file(...)` and the related conversation-construction
    helpers in `cli/src/services/agent_trace.rs`: union the related session IDs
    from the matched direct and matched mutation hunks, and select
    `contributor.model_id` by the combined agreement rule where an absent
    evidence source does not count as unknown; add evidence fixtures for the new
    cases. Out — `Contributor` / `ConversationRelated` / `AgentTrace` type
    shapes, `config/schema/agent-trace.schema.json`, the hunk `ai` / `mixed` /
    `unknown` classification rule, and `line_changes` bucketing.
  - Dependencies: T05
  - Done when: a mutation-only hunk emits its model and related session; a
    direct-only hunk emits exactly what it emits today (the `direct_only` golden
    fixture is byte-unchanged); `direct X + mutation X` emits `X`;
    `direct X + mutation Y` and `direct X + mutation unknown` omit `model_id`;
    related sessions are the deduplicated, deterministically ordered union; all
    built payloads still validate against the embedded schema.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace`.
  - Context synchronization: pending

- [ ] T07: `Add end-to-end provenance regressions and synchronize context` (status:todo)
  - Task ID: T07
  - Scope: In — real temporary-repository, real Agent Trace DB regressions in
    `cli/src/services/hooks/mod.rs` covering the full
    `Bash -> mutation scope -> commit -> persisted Agent Trace JSON` path for
    both producers; the durable context updates named under **Context sync**;
    the final branch comparison against `origin/codex-mutation-scope-integration`.
    Out — new production behavior; every behavior this task exercises is already
    delivered by T01–T06.
  - Dependencies: T03, T04, T06
  - Done when: a Claude `Bash`-created file and a Codex `Bash`-created file each
    reach persisted `agent_traces.trace_json` with `contributor.type = "ai"`,
    their available model, and their canonical `cc_` / `cx_` related session URL;
    the existing direct-attribution regressions and the three-layer persistence
    separation assertions stay green; each context file named under **Context
    sync** describes the implemented behavior, including the explicit
    `mutation protocol attribution != scope provenance` boundary and the
    `ScopeId` proves ownership / `ScopeProvenance` describes the owning scope
    statement.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`.
  - Context synchronization: pending

## Open questions

None.

Every semantic choice is decided in the design: `direct X + mutation unknown ->
NULL` is settled in D8, with AC7 rewritten to cover direct-only attribution and
direct-coverage precedence rather than all direct attribution; provenance
replay/conflict semantics are fixed by D3's matrix; durable `Start` ordering and
its invariants are fixed by D4; the Claude model-resolution seam is fixed by D6.
Multiple models in one hunk omit `model_id`; an unknown model keeps the session
and leaves the model null; a Claude subagent without an exact row inherits
nothing; a model switch after scope creation does not rewrite history; direct
coverage exclusion stays authoritative on overlap; OpenCode/Pi keep provenance
optional until those adapters are wired.
