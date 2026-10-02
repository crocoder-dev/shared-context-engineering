# Claude mutation-scope background and detached execution boundaries

Detail split out of
[claude-mutation-scope-integration.md](claude-mutation-scope-integration.md)
for the repository's per-file line budget.

## Background shell execution

Native Claude background Bash is operationally supported but unsupported for
mutation attribution. The effective decision is
[Claude background Bash runs natively and untracked](../decisions/2026-10-02-claude-background-bash-untracked.md).

### Background Bash: allowed natively, untracked

`Bash` with `tool_input.run_in_background = true` returns the ordinary empty
`PreToolUse` allow output. `handle_pre_tool_use` takes that return before
repository resolution, the recovery barrier, model-state resolution, and
`establish_start`, so the call:

- creates no SCE mutation scope and calls no generic mutation-scope `start`;
- creates no Claude adapter attempt and performs no adapter-state I/O;
- receives no lifecycle tracking;
- is admitted even while `recovery_pending` is armed, because it never reaches
  the barrier;
- may mutate the repository in ways that contaminate the attribution of later
  or concurrent tracked scopes.

A later `PostToolUse` or `PostToolUseFailure` for that tool finds no matching
attempt and is a no-op through the existing missing-attempt branch of
`handle_close`. It does not close, abandon, or otherwise transition an
unrelated live attempt.

Claude lifecycle completion remains unproven, so the adapter deliberately
avoids lifecycle tracking rather than attempting to recover it. SCE makes no
mutation-attribution guarantee for mutations produced by a native background
Bash process. This is a compatibility tradeoff, not an attribution-safety or
fail-closed feature.

The accepted consequence, stated as a tradeoff outside SCE's guarantees rather
than a bug:

```text
background A starts            -> untracked
foreground B starts            -> tracked
A mutates the repository while B is live
B closes
SCE observes A's mutation at B's boundary and attributes it to B
```

Under the existing protocol that sequence produces one `ai_exclusive` `close`
event attributed to B's scope, with the cursor advanced to the tree containing
A's write. No test or protocol machinery distinguishes A from B.

### Background PowerShell: denied

`PowerShell` with `run_in_background = true` is denied in `PreToolUse`
(fail-closed shape), before any repository or adapter-state access, with:

```text
SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.
```

Background **subagents** are not excluded — their internal foreground
mutation-capable tool calls still establish their own scopes.

## T01 current-version probe result

On 2026-10-01, the installed Claude Code version was `2.1.284`. A T01 probe
using an isolated temporary settings file and an additive capture hook reached
`SessionStart`, but the non-interactive session produced no model response or
tool/lifecycle event after more than 120 seconds. A no-hook control behaved the
same way. The result is recorded in
`cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json`
and the detailed G1-G7 table is in `fixtures/NOTES.md`.

The existing `probe14-*` and `probe17-*` evidence was captured on Claude Code
`2.1.258`, so it is retained as prior-version evidence and does not establish
the current-version terminal contract. G1-G7 therefore remain unknown for
`2.1.284`. At the time, the T01 decision gate stopped there and kept the
denial in force until a later probe could establish a reliable completion
signal and a proven no-more-mutation boundary, with SCE-owned process
supervision as the fallback direction rather than admission based on the
acknowledgement event alone. That constraint is recorded in the
[Claude background admission decision](../decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md).

The T01 technical conclusion stays true: G1-G7 are unproven, so
attribution-safe background tracking cannot be built from the observed Claude
lifecycle. The admission policy it carried is superseded by
[Claude background Bash runs natively and untracked](../decisions/2026-10-02-claude-background-bash-untracked.md),
which accepts untracked execution anyway and is what the adapter implements.
The 2026-10-01 record is retained unmodified as history.

**Self-detaching descendants are a separate, explicit unsupported boundary
(D20).** A `run_in_background = false` call can still leave a repository-mutating
descendant running after `PostToolUse` returns when the invoked command detaches
a child (`command &`, `nohup`, `setsid`, double-fork, `start_new_session=True`).
T04 proved this live against Claude Code `2.1.258`: a foreground `setsid`
command returned `PostToolUse` in `duration_ms: 13` and its descendant's write
landed ~3s later, changing the Git tree an SCE snapshot would capture — outside
the tool's closed scope. This is not solvable by inspecting the command string;
the integration adds no detection, supervision, or static scan, and simply does
not treat `PostToolUse` as proof that every descendant has stopped mutating. See
the T04 addendum and `probe17-*` fixtures under
`cli/src/services/hooks/claude_mutation_scope/fixtures/`.

## Related context

- [Claude mutation-scope integration](claude-mutation-scope-integration.md)
