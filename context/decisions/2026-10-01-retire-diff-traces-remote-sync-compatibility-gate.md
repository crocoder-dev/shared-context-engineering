# Decision: Retire diff_traces from remote sync through a staged compatibility gate

Date: 2026-10-01
Status: Accepted
Plan: `context/plans/retire-diff-traces-remote-sync.md`
Task: T01

## Context

The local Agent Trace database captures four streams: `messages`, `parts`, `diff_traces`, and `agent_traces`. `sce sync` previously uploaded all four to the Control Plane ingestion API. `diff_traces` is being retired from the Control Plane ingestion contract, but it stays essential locally: hook ingress, structured-patch reconstruction, post-commit patch intersection, staged-diff AI-overlap, and attribution all read it.

The client and the Control Plane are released independently. A released SCE client decodes `cursors.diffTraces` in the `POST /agent-trace/ingestion/state` response as a mandatory field, and consumers of `sce sync --format json` read `streams.diffTraces`. Removing either surface in one step would break released clients or JSON consumers.

## Decision

`sce sync` remotely synchronizes exactly three streams — `messages`, `parts`, `agent_traces` — and `diff_traces` remains a local-only capture stream, retired from Control Plane ingestion through a staged compatibility gate in which each step waits for the one before it:

1. **Phase 2 (SCE client):** stop reading and POSTing `diff_traces` batches, while still decoding the mandatory `/state.cursors.diffTraces` field.
2. Merge and release a new SCE version.
3. Verify and adopt that version so supported clients no longer upload `diff_traces`.
4. **Phase 3 (Control Plane):** stop accepting and storing new `diff_traces` batches, while `/state` keeps returning `cursors.diffTraces`.
5. **Later compatibility-removal phase:** remove `/state.cursors.diffTraces` and the retained client and server compatibility surfaces from both sides together.

During the compatibility window the client keeps `streams.diffTraces` in `sce sync --format json` as a zero-upload entry that echoes the server cursor (`uploaded: 0`, `batches: 0`, `initialCursor == finalCursor == state.cursors.diffTraces`), and keeps the unused protocol surfaces `IngestionStream::DiffTraces`, `ingest_diff_traces`, `read_diff_traces_after`, `AgentTraceDiffTraceExportRow`, and `AgentTraceCursors.diff_traces`.

## Rationale

Stopping the upload first means no supported client depends on the Control Plane accepting `diff_traces` by the time the server rejects it. Keeping the `/state` cursor and the JSON entry through the window means neither a Phase 2 client nor a JSON consumer sees a shape change until both sides can drop the field together. Building the JSON entry from the server cursor alone keeps sync from reading local `diff_traces` rows at all, so a local row the export reader would reject cannot fail a sync.

## Alternatives considered

- **Remove the `diff_traces` protocol surfaces and the `/state` cursor in the same change** — breaks released clients that decode the cursor as mandatory, and couples the client and Control Plane releases.
- **Make `AgentTraceCursors.diff_traces` optional** — alters the `/state` DTO inside the compatibility window and weakens cursor validation for no client benefit.
- **Gate the behavior behind a feature flag** — adds a second supported sync shape to test and document for a change that is meant to be one-directional.

## Compatibility and risks

- The `/state` request and response contract is unchanged; `cursors.diffTraces` stays required.
- `streams.diffTraces` in the JSON report no longer means the stream is synchronized. A consumer that reads `uploaded` or cursor movement there as sync progress will see a constant zero-upload entry.
- Text-mode progress shows three rows; anything that scraped a `diff_traces` progress row loses it.
- If Phase 3 lands before the Phase 2 client is adopted, older clients fail their `diff_traces` batch requests. The gate order is the mitigation.
- Historical `diff_traces` rows already stored by the Control Plane are neither backfilled nor deleted by this decision.

## Guardrails

- Sync must not read local `diff_traces` rows, including to build the compatibility entry.
- The local `diff_traces` table, migrations, hook ingress, and every local consumer stay untouched.
- The retained protocol surfaces keep their signatures and serialization until the compatibility-removal phase.
- Phase 3 must not remove `/state.cursors.diffTraces`.

## Consequences

- The governing distinction is four local capture streams versus three remotely synchronized streams.
- `AgentTraceExportReader` exposes four readers while `sce sync` consumes three; `read_diff_traces_after` has no active caller.
- The client carries unused compatibility code until the removal phase.
- The "already synced" text heading depends only on the three active streams.

## Follow-up

- Release and adopt the SCE version carrying Phase 2.
- Phase 3 in the Control Plane.
- The later compatibility-removal phase across both sides.

## References

- Plan: [`retire-diff-traces-remote-sync`](../plans/retire-diff-traces-remote-sync.md)
- Task: `T01`
- Current-state context: [`sce sync command`](../cli/sync-command.md), [`Agent Trace sync architecture`](../cli/agent-trace-sync-command.md), [`Agent Trace export readers`](../sce/agent-trace-export-readers.md), [`Agent Trace DB`](../sce/agent-trace-db.md)
- Evidence: [`sync orchestration`](../../cli/src/services/sync/sync.rs), [`sync progress`](../../cli/src/services/sync/progress.rs), [`sync rendering`](../../cli/src/services/sync/render_sync.rs)
- Related decision: [`Keep trace-sync progress on stderr while stdout remains payload-only`](2026-08-13-trace-sync-progress-stream-contract.md)
