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

The child runs with the repository root as its working directory, null stdin and
stdout, and piped stderr. An internal `SCE_INTERNAL_AUTO_SYNC=1` process
marker lets the child classify this invocation as automatic without adding a
user-facing option or configuration layer. The launcher waits for the child to
reach terminal completion before returning, draining the pipe while waiting and
forwarding the captured bytes through the parent's stderr. This makes the child
lifetime and pipe-drain completion deterministic. This intentionally adds the
child synchronization duration to post-commit latency; no timeout policy is part
of the boundary. A non-zero child exit remains fail-open after the child has
rendered its own single typed `SCE-ERR-RUNTIME` diagnostic; the launcher forwards
the captured bytes and does not duplicate it. If the current executable cannot be
resolved, the child cannot be spawned, or waiting fails, the launcher emits the
same typed automatic-sync diagnostic with the startup or wait reason on stderr.
All launcher and child failures remain fail-open, so they cannot turn a
successful post-commit operation into a failure.

Automatic synchronization is not invoked by `pre-commit`, `diff-trace`, or
`conversation-trace`. It is one post-commit launch, not a high-frequency hook,
watcher, polling loop, scheduler, daemon, retry queue, persistent service, or
second synchronization database.

### Failure diagnostics

The automatic child classifies terminal sync failures into the closed
`AutomaticSyncFailureKind` set: `Authentication`, `ControlPlane`, `Stream`, or
`Runtime`. The app renders exactly one `Error [SCE-ERR-RUNTIME]` diagnostic
whose message begins `Automatic synchronization failed:`. Authentication uses
the reviewed `sce auth login`, then manual `sce sync` recovery instruction and
keeps the technical reason for observability; non-authentication failures
include their preserved display reason and actionable recovery guidance before
the manual `sce sync` retry. Automatic user-error rendering does not append the
generic runtime `Try:` sentence or render the technical source as a second
diagnostic. Launcher executable-resolution and spawn failures use the same
`Runtime` payload and preserve their startup reason.

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
control-plane cursor authority. A child startup, wait, or network failure is
fail-open to the commit. Rows that remain local are available to a later
manual `sce sync` or a later successful automatic invocation; no local cursor
or background retry machinery is required.

See [the sync command contract](sync-command.md), [the config precedence
contract](config-precedence-contract.md), [the hook routing contract](../sce/agent-trace-hooks-command-routing.md),
and [the captured-stderr decision](../decisions/2026-09-01-parent-owned-automatic-sync-stderr.md).
