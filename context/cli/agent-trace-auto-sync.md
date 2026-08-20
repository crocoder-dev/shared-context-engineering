# Automatic Agent Trace synchronization

## Purpose

Automatic synchronization is a default-enabled convenience layered on the existing
`sce sync` command. It does not replace explicit synchronization or introduce a
second synchronization engine.

## Configuration

`agent_trace.auto_sync` is a config-file-only boolean resolved through the normal
global-then-local config merge. It defaults to `true`, and `sce config show`
reports the resolved value and its source. Set it explicitly to `false` to opt
out. There is no environment variable or CLI flag for this setting.

## Trigger boundary

The post-commit hook first completes its existing Agent Trace validation and
repository-scoped database persistence. Only after both succeed does it inspect
`agent_trace.auto_sync`. When enabled, the hook asks the sync-owned launcher to
start the current `sce` executable with exactly:

```text
sync --format json
```

The child runs with the repository root as its working directory and null
stdin, stdout, and stderr. `Command::spawn()` is used without waiting for a
status; the hook returns its normal successful result immediately. A failure to
resolve the current executable or spawn the child is ignored, so launcher
failures cannot turn a successful post-commit operation into a failure.

Automatic synchronization is not invoked by `pre-commit`, `diff-trace`, or
`conversation-trace`. It is one post-commit launch, not a high-frequency hook,
watcher, polling loop, scheduler, daemon, retry queue, persistent service, or
second synchronization database.

## Doctor readiness

`sce doctor` reports the capability without invoking it. The post-commit hook's
canonical managed block is the hook-side readiness proof, using the same
managed-block currency semantics as setup. The report exposes
`post_commit_auto_sync` in JSON with `state`, `enabled`, `source`, and
`config_source` fields. Its states are `ready` for enabled/current,
`disabled` for an explicit false opt-out, `not_ready` for enabled but missing,
stale, unreadable, or otherwise non-current hook content, and `not_applicable`
outside repository scope. The disabled state is healthy and does not alter the
existing problem/remediation or overall readiness rules for unrelated hook
issues. Text uses `[PASS] Post-commit Agent Trace auto-sync`, the explicit
disabled label, or `[FAIL] Post-commit Agent Trace auto-sync` accordingly.
Doctor never launches `sce sync` or a background process.

## Manual synchronization and retryability

The explicit operator flow remains:

```text
sce auth login
cd <repository>
sce sync
```

Automatic execution uses the same command and therefore the same repository
Agent Trace database, control-plane protocol, authentication, and
control-plane cursor authority. A child startup, completion, or network failure
is fail-open to the commit. Rows that remain local are available to a later
manual `sce sync` or a later successful automatic invocation; no local cursor
or background retry machinery is required.

See [the sync command contract](sync-command.md), [the config precedence
contract](config-precedence-contract.md), and [the hook routing contract](../sce/agent-trace-hooks-command-routing.md).
