# Commit output layouts

Use only the applicable layout. Values come from staged truth and internal
workflow state.

## Regular-mode staging gate

```markdown
Please run `git add <files>` for all changes you want included in this commit.
Atomic commits should only include intentionally staged changes.
Confirm once staging is complete.
```

## No staged changes

```markdown
No staged changes. Stage changes before commit.
```

## Regular proposal

For each proposal, present the complete commit message and covered files. When
more than one commit is proposed, also present the split rationale. Do not claim a
commit was created.

## Blocked

Present every issue's problem, impact, and required decision. Do not commit.

## Bypass success

```markdown
Committed {commit-hash}
```

## Bypass Git failure

Present Git's failure unchanged and stop without retrying.
