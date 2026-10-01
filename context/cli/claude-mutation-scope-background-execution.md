# Claude mutation-scope background and detached execution boundaries

Detail split out of
[claude-mutation-scope-integration.md](claude-mutation-scope-integration.md)
for the repository's per-file line budget.

## Background shell is unsupported

An explicit `Bash.run_in_background = true` / `PowerShell.run_in_background =
true` is denied in `PreToolUse` (fail-closed shape) with:

```text
SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.
```

A detached shell can keep mutating the repository after `PostToolUse` returns and
can outlive a session; the generic contract has no process supervisor or stable
background-execution terminal signal. This is a deliberate correctness boundary,
not a Bash security policy. Background **subagents** are not excluded — their
internal mutation-capable tool calls still establish their own scopes.

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
`2.1.284`; per the plan, the decision gate stops here and the denial remains
in force until a later probe can establish a reliable completion signal and a
proven no-more-mutation boundary. The fallback direction is SCE-owned process
supervision, not admission based on the acknowledgement event alone. This
constraint is recorded in the
[Claude background admission decision](../decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md).

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
