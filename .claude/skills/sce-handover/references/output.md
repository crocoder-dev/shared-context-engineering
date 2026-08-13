# Handover output layouts

Use only the applicable layout. Values come from the resolved mode and
internal workflow state.

## Invalid usage

```markdown
# Handover: invalid arguments

`/handover` takes no arguments (writer mode) or exactly one handover path
(loader mode): `/handover context/handovers/<file>.md`.

Received: `{$ARGUMENTS}`
```

## Writer blocked

```markdown
# Handover not written

**Problem:** {specific blocker, e.g. missing context root}
**Required action:** `{command or decision}`

No file was written.
```

## Writer success

```markdown
# Handover written

**Path:** `{written path}`

To continue from this handover in another session:

`/handover {written path}`
```

## Loader blocked

```markdown
# Handover not loaded

**Problem:** {path outside context/handovers/, non-Markdown, missing file, or
missing required section}
**Path:** `{argument as given}`

Loading stopped. No file was read past the point of failure, and nothing was
changed.
```

## Loader success

```markdown
# Handover loaded

**Path:** `{loaded path}`

## Current Task State

{content read from the file}

## Decisions Made

- {content read from the file}

## Open Questions / Blockers

- {content read from the file}

## Next Recommended Step

{content read from the file}

## Assumptions

- {content read from the file, or `None.`}

---

This handover has been presented for continuation only. No file was edited, no
plan task was marked complete, and the recommended next step was not started.
```

# Report rules

- Writer success must report the exact written path so
  `/handover {written path}` is directly runnable.
- Never claim a handover was written or loaded unless the corresponding file
  operation actually completed.
- Loader success must not include any statement implying repository state
  changed; loading is read-only.
- Omit `Plan` and `Task` framing in the loaded output only when the source
  document itself omits them.
- Never fabricate section content; render exactly what was written or read.
