# SCE Atomic Commit Result Contract

Return exactly one Markdown document using one layout below. `Status` is the
branch value consumed by `/commit`. Use every required heading and label
exactly as written, omit optional sections that do not apply, and do not add
prose outside the selected layout.

Report paths exactly as `git diff --cached --name-only` reports them. Every
staged file belongs to exactly one proposed commit. Keep each message's body
in the same fenced block as its subject, separated by one blank line. Never
report a commit hash or guidance forbidden by the mode.

## Status: `proposal`

Use only in regular mode. Nothing is committed.

````markdown
# Atomic Commit Result

Status: proposal
Mode: regular

## Commits

### {commit.id}

#### Message

```text
{subject}

{body}
```

#### Files

- {file}

#### Cites plan

{true|false}

## Split rationale

{split_rationale}

## Scope classification

{context_only|mixed}

## Notes

- {note}
````

`Commits` is required. Repeat the commit block as needed. Include `Split
rationale` only for multiple commits. `Scope classification` and `Notes` are
optional.

## Status: `bypass_message`

Use only in bypass mode. Exactly one message covers every staged file.

````markdown
# Atomic Commit Result

Status: bypass_message
Mode: bypass

## Message

```text
{subject}

{body}
```

## Files

- {file}

## Cites plan

{true|false}

## Notes

- {note}
````

`Message` and `Files` are required. `Cites plan` and `Notes` are optional.
Never include commits, split rationale, or scope classification.

## Status: `blocked`

````markdown
# Atomic Commit Result

Status: blocked
Mode: {regular|bypass}

## Files

- {file}

## Issues

### {issue.id}

- Category: {no_staged_changes|plan_citation_ambiguity|unreadable_diff|contradictory_context}
- Problem: {problem}
- Impact: {impact}
- Decision required: {decision_required}
````

`Issues` is required. Include `Mode` when supplied and `Files` when staged
files were read. Plan-citation ambiguity blocks only in regular mode; bypass
mode omits the citation instead.
