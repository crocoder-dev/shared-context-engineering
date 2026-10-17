# Audit output layouts

Use only the applicable layout. Values come from workflow state. Omit empty detail
lists, but keep all status and count fields.

## Missing context bootstrap gate

```markdown
-------------------------------------

# SCE context audit blocked.

`context/` does not exist in {repository}.

Required action: `sce setup --bootstrap-context`

No repository files were changed.
```

## Unsupported scoped audit

```markdown
-------------------------------------

# SCE context audit not started.

`sce-audit` audits the complete current-state `context/` surface.
Path-, file-, domain-, and glob-scoped audits are not supported.

Repository: {repository}
No repository files were changed.
```

## Blocked

```markdown
-------------------------------------

# SCE context audit blocked.

Repository: {repository}
Reason: {reason}

Preserved files:
- {preserved-path}: {why-preserved}

Pending repairs:
- {target-path}: {repair-summary}

Retry condition: {retry-condition}

No Git commit was created.
```

## Completed audit

```markdown
-------------------------------------

# SCE context audit complete.

Repository: {repository}
Status: {clean|repaired}

Findings before repair:
- verified: {verified-count}
- drifted: {drifted-count}
- missing: {missing-count}
- orphaned: {orphaned-count}
- unverifiable: {unverifiable-count}

Context changes:
- {context-path}: {repair-summary}

Unresolved:
- {classification}: {summary} — {evidence-gap}

Protected areas: unchanged
Implementation files: unchanged
Post-write verification: passed
No Git commit was created.
```
