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
- The validation status is the `Status:` line (usually `**Status:**`) in a top-level paragraph of the report.
- For context synchronization and validation status, only the first word of the value counts, so `synced — root pass confirmed` and `validated (revalidated after T05)` keep their state. Task status markers are exact, because values such as `done; reopened in_progress` are ambiguous.

## Errors

The first problem fails the command with a line number. Nothing is written to stdout. A missing or unreadable file is `SCE-ERR-RUNTIME` (exit 4). Invalid UTF-8 and uninterpretable plan state are `SCE-ERR-VALIDATION` (exit 3). Plan-state errors cover a missing, empty, or duplicate task stack; malformed task items (including non-task items in the stack); duplicate task IDs; a missing or unsupported status marker; checkbox/status disagreement; an unsupported or duplicate context synchronization field; a duplicate validation report; and a missing, duplicate, or unsupported validation status.

## Legacy plan compatibility

When this command was added, 99 of the 174 plans in `context/plans/` parsed. The rest are older formats that the command rejects on purpose: validation reports without a `**Status:**` field, `(status:complete)` or annotated status markers, `## Tasks` / `## 4) Task stack` headings, and non-task items or repeated sync fields in the task stack.
