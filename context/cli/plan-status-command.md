# `sce plan status`

`sce plan status <path> [--format text|json]` reads one SCE Markdown plan file and prints its recorded lifecycle state. SCE owns the interpretation of its plan format, so consumers such as SCE Factory call this instead of keeping their own Markdown parser.

The command is hidden from `sce`, `sce help`, and `sce --help`, but it is parser-valid and has command-local `--help`.

It is read-only. It reads only the given path: no plan-name resolution, repository discovery, Git, database, authentication, network, or `sce setup` is needed. It reports recorded facts and does not select tasks, judge readiness, or run any workflow.

## JSON contract

```json
{
  "tasks": [
    { "id": "T01", "title": "Create database schema", "status": "done", "context_sync": "synced" }
  ],
  "validation": { "status": "validated", "report_sha256": "<lowercase hex>" }
}
```

- `tasks` keeps plan order. `status` is `todo` or `done`. `context_sync` is `pending`, `synced`, `blocked`, or `null` when a legacy task has no `Context synchronization:` field. A missing field is never read as synced.
- `validation` is `null` when the plan has no `## Validation Report` section. `status` is `validated` or `failed`.
- `report_sha256` is the SHA-256 of the report section's source bytes, from its heading to the next heading of depth 2 or less (or the end of the file), including its subsections. It lets a consumer detect a changed report. It is not evidence that validation ran.
- No aggregate, count, next-task, timestamp, or schema-version fields are emitted.

## Interpretation rules

Code: `cli/src/services/plan/{mod,command,parse}.rs`. `parse::parse(&str)` is pure and parses the document once with the `markdown` crate's GFM AST.

- Tasks come only from list items in top-level lists under the single `## Task stack` heading (until the next heading of depth 2 or less). Fenced code, blockquotes, nested lists, and other sections are ignored.
- Each task item needs a GFM checkbox and a lead line `T<2+ digits>: <title> (status:todo|done)`. The title is the plain text with inline formatting removed, so `` `Title` `` and `**T01: Title**` forms both work. Checked requires `status:done` and unchecked requires `status:todo`.
- `Context synchronization:` is read only from the task's own direct sub-list. `Context synchronization blocker:` and all other fields are ignored.
- The validation status is the `Status:` line (usually `**Status:**`) in a top-level paragraph of the report header: the content between `## Validation Report` and its first subsection heading. A `Status:` line inside a subsection such as `### Failed checks and follow-ups` is ignored, so a report whose header lacks one fails as missing. This does not change `report_sha256`, which still covers the whole section.
- All lifecycle values are exact. After inline formatting is removed and surrounding whitespace is trimmed, context synchronization must be exactly `pending`, `synced`, or `blocked`, validation status exactly `validated` or `failed`, and task status markers exactly `todo` or `done`. Annotated or ambiguous values such as `synced but verification failed`, `pending, previously synced`, `validated (revalidated after T05)`, or `validated.` are rejected, so a contradictory note can never be read as a successful state.

## Errors

The first problem fails the command with a line number. Nothing is written to stdout. A missing or unreadable file is `SCE-ERR-RUNTIME` (exit 4). Invalid UTF-8 and uninterpretable plan state are `SCE-ERR-VALIDATION` (exit 3). Plan-state errors cover a missing, empty, or duplicate task stack; malformed task items (including non-task items in the stack); duplicate task IDs; a missing or unsupported status marker; checkbox/status disagreement; an unsupported or duplicate context synchronization field; a duplicate validation report; and a missing, duplicate, or unsupported validation status.

## Legacy plan compatibility

With exact lifecycle values, 97 of the 174 plans in `context/plans/` parse. The rest are older formats that the command rejects on purpose: validation reports without a `**Status:**` field in the report header, `(status:complete)` or annotated status markers, annotated context synchronization or validation status values, `## Tasks` / `## 4) Task stack` headings, and non-task items or repeated sync fields in the task stack. Historical plans are not rewritten; only the current canonical plan format is supported.
