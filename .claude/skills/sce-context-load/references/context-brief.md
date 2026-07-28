# SCE Context Load Result Contract

Return exactly one Markdown document using one layout below. The first line
after the title is the status source consumed by the invoking command. Use
the headings and labels exactly as written, omit optional sections that do
not apply, and do not add prose outside the selected layout.

List every file read and no file that was not read. Key facts are durable
statements, not planning or implementation recommendations. Empty required
lists must contain `- None.`. This skill never reports context edits.

## Status: `loaded`

Use when the context root exists and relevant context was read.

```markdown
# Context Load Result

Status: loaded

## Context root

{context_root}

## Entry points

### {path}

- Read: {true|false}
- Reason: {reason; omit only when read is true and no reason is needed}

## Domain context

### {path}

**Relevance:** {relevance}

**Key facts:**

- {key_fact}

## Gaps

### {area}

{detail}

## Drift

### {path}

- Context says: {context_says}
- Code says: {code_says}
- Repair needed: {repair_needed}
```

`Context root`, `Entry points`, `Domain context`, and `Gaps` are required.
Repeat the entry-point, domain-context, gap, and drift blocks as needed.
Include `Drift` only when recorded context contradicts code. Record gaps for
absent entry points and focus areas without durable context.

## Status: `bootstrap_required`

Use when the context root does not exist. Read and create nothing.

```markdown
# Context Load Result

Status: bootstrap_required

## Context root

{context_root}

## Reason

{reason}
```
