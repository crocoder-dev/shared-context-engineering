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
