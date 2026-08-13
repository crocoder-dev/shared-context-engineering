# Decision: Keep trace-sync progress on stderr while stdout remains payload-only

Date: 2026-08-13
Status: Accepted
Plan: `context/plans/trace-sync-progress.md`
Task: T01, T02, T03, T04

## Context

`sce trace sync` needs observable feedback during slow text-mode uploads, but CLI callers depend on stdout containing the command payload and JSON mode remaining machine-readable. The completed plan adds accepted-batch progress plus UTC lifecycle timestamps and must establish a durable stream boundary for this user-visible behavior.

## Decision

Emit human-readable `sce trace sync` progress and lifecycle timestamps only on `stderr`; keep the final text report and the unchanged JSON payload on `stdout`, with JSON mode emitting no progress or lifecycle text.

## Rationale

This provides live feedback without contaminating redirected or piped command payloads and preserves the existing machine-readable JSON contract. A single format-gated reporter also keeps the behavior deterministic and limits progress to presentation rather than sync protocol or persistence state.

## Alternatives considered

- **Emit progress on stdout** — would mix transient lines with the final payload and break consumers that redirect or parse command output.
- **Add progress fields or messages to JSON** — would change the established machine-readable schema and make JSON unsuitable for callers expecting only the payload.
- **Use terminal-only redraw output** — would make redirected human output ambiguous and add unnecessary TTY-specific behavior.

## Compatibility and risks

- Text-mode callers that separately capture stderr will now receive progress and lifecycle lines; stdout payload shape remains unchanged.
- JSON consumers retain the existing stdout schema and receive no human side channel. Progress output remains newline-delimited, flushed, credential-free, and limited to batch/stream summaries.

## Guardrails

- Do not add progress fields to the JSON report or alter final text/JSON renderers.
- Keep progress on stderr, preserve fixed stream order, and emit no per-row payloads, credentials, raw responses, or local database rows.
- Keep timestamps UTC RFC3339 and report start before the first control-plane request and finish after terminal success or failure.

## Consequences

- Operators can observe slow text-mode synchronization before the final report completes.
- CLI integrations can continue treating stdout as the command payload and JSON mode as silent apart from its JSON stdout result.
- The stderr contract now includes trace-sync progress as an intentional presentation channel.

## Follow-up

None.

## References

- Plan: [`trace-sync-progress`](../plans/trace-sync-progress.md)
- Task: `T01, T02, T03, T04`
- Current-state context: [`sce trace command`](../cli/trace-command.md), [`Agent Trace sync architecture`](../cli/agent-trace-sync-command.md), [`CLI stdout/stderr contract`](../sce/cli-stdout-stderr-contract.md)
- Evidence: [`trace sync orchestration`](../../cli/src/services/trace/sync.rs), [`trace command`](../../cli/src/services/trace/command.rs), [`Validation Report`](../plans/trace-sync-progress.md)
- Related decision: [`Migrate from lexopt to clap for CLI Argument Parsing`](2026-03-09-migrate-lexopt-to-clap.md)
