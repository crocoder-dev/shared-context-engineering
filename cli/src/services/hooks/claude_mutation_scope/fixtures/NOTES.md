# T01/T04 fixture capture notes

Raw Claude Code hook-event payloads captured live, in-session, by temporarily
wiring a throwaway dump hook into this checkout's own `.claude/settings.json`
(reverted immediately after capture; not shipped). Every file in this
directory is an unmodified byte-for-byte copy of what the real `claude` binary
wrote to the hook script's STDIN.

- **Tested Claude Code version:** `2.1.258` (`claude --version`), the version
  installed in this environment. No other version is pinned anywhere in the
  plan or repo, so this is "the version SCE chooses to support" per T01's
  scope.
- **Session:** `f8e78276-48a2-45d8-a421-b47b7aad4768`, captured 2026-09-04.
- **Capture method:** `cli/src/services/hooks/claude_mutation_scope/fixtures/`
  did not exist before this task. A scratch script
  (`.../scratchpad/hook-capture/capture.sh`) was registered as an *additional*
  hook entry (alongside, not replacing, the existing SCE hook entries) for
  every lifecycle event named in the plan, dumping raw STDIN JSON to a
  timestamped file. A second scratch script
  (`.../scratchpad/hook-capture/deny-marker.sh`) was registered on
  `PreToolUse` to synthetically `deny` any tool call whose payload contained a
  unique marker string, for probe 6. Both were removed from
  `.claude/settings.json` before this task finished; see the task's `Files
  changed` record for the diff.

## Per-probe outcome

| # | Probe | Status | Fixture files |
|---|---|---|---|
| 1 | `Write` success | captured | `probe01-write-success.*` |
| 2 | `Bash` success | captured | `probe02-bash-success.*` |
| 3 | `Bash` writes then exits non-zero | captured | `probe03-bash-partial-write-then-nonzero-exit.*` |
| 4 | Two parallel mutation tools | captured | `probe04-two-parallel-mutation-tools.*` |
| 5 | Manual permission denial | **not captured** | — see below |
| 6 | Another `PreToolUse` hook denies the tool | captured | `probe06-another-pretooluse-hook-denies.*` |
| 7 | Auto-mode `PermissionDenied` | captured | `probe07-auto-mode-permission-denied.*` |
| 8 | User interrupt before `Stop` | **captured via analog, not literally** | `probe08-forced-stop-analog-no-terminal-signal.*` — see below |
| 9 | Next main-thread `UserPromptSubmit` after interruption | **not captured** | — see below |
| 10 | Subagent tool call with `agent_id` | captured | `probe10-subagent-tool-call.*` |
| 11 | `SubagentStop` then resumed same `agent_id` | captured | `probe11-resumed-subagent-same-agent-id.*` |
| 12 | `isolation: worktree` tool `cwd` | captured | `probe12-isolation-worktree-tool-cwd.*` |
| 13 | `WorktreeRemove` payload | **attempted, not observed** | — see below |
| 14 | Explicit `run_in_background=true` Bash | captured | `probe14-run-in-background-true.*` |
| 15 | `run_in_background=false` long-running Bash (**hard gate**) | captured — **PASS** | `probe15-run-in-background-false-hard-gate.*` |
| 16 | (optional) `PostToolBatch` | captured | `probe16-post-tool-batch-optional.*` |
| 17 | T04 follow-up: foreground `Bash` starts a self-detaching descendant | captured | `probe17-detached-child-after-post-tool-use.*` — see T04 addendum below |

12 of 15 required probes captured with real, live payloads. Probes 5 and 9
could not be produced at all in this session (see below); probe 8 is answered
by a directly relevant self-triggerable analog rather than a literal user
interrupt; probe 13 was attempted twice and the event was not observed to
fire.

## D-decision findings (the actual gate this task exists for)

### Hard gate — D20 (`run_in_background=false`): **PASS**

`probe15-run-in-background-false-hard-gate.pre_tool_use.json` /
`.post_tool_use.json`: a `Bash` call with `tool_input.run_in_background:
false` and a `sleep 4` body produced `PostToolUse` only after
`duration_ms: 4018` — the call genuinely blocked in the foreground for the
full sleep duration. Claude Code 2.1.258 cannot silently detach a foreground
shell call whose incoming payload said `false`. D20 as written is sound; no
plan revision required.

Contrast with `probe14-run-in-background-true.*`: the same shape of command
with `run_in_background: true` produced `PostToolUse` after `duration_ms: 8`
with a `tool_response.backgroundTaskId` field and no output — confirming
`PostToolUse` for a backgrounded call is a stub acknowledging the detach, not
a completion signal. This is exactly the risk D20 describes and validates
denying explicit `run_in_background=true` at `PreToolUse`.

### D10 (`PostToolUseFailure`): **PASS**

`probe03-bash-partial-write-then-nonzero-exit.*`: a `Bash` call that wrote a
file then exited 7 produced **only** `PostToolUseFailure`
(`error: "Exit code 7"`, `is_interrupt: false`) — no `PostToolUse` fired at
all for the same `tool_use_id`. This resolves the plan's open question ("does
`PostToolUse` also fire, or only `PostToolUseFailure`?"): on 2.1.258, exactly
one of the two fires per attempt, never both. The event carries `session_id`,
`cwd`, `tool_name`, `tool_use_id` — everything D10's mapping reads. D9/D10 as
written (map `PostToolUseFailure` to the same `Close` operation as `PostToolUse`)
need no revision.

### D13 (`PermissionDenied`): **PASS**, and confirms the design's own caveat

`probe07-auto-mode-permission-denied.permission_denied.json`: a `Bash` call
denied by this harness's own automatic classifier fired `PermissionDenied`
with `tool_name`, `tool_input`, `tool_use_id`, `session_id`, `cwd`, and
`reason: "Blocked by classifier"` — everything D13 reads.

`probe06-another-pretooluse-hook-denies.*`: a `Bash` call denied by a
*second, independent* `PreToolUse` hook (synthetic `permissionDecision: deny`)
produced **no** `PermissionDenied` event at all — only the original
`PreToolUse`, then a `PostToolBatch` entry whose `tool_response` carries the
deny reason. This is exactly what D13 already assumes: `PermissionDenied` is
an auto-mode-classifier-shaped signal, and "another parallel `PreToolUse`
hook blocking the tool" is *not* covered by it and must rely on lifecycle
cleanup (`Stop`/`UserPromptSubmit`/`SessionEnd`) instead. Manual denial
(probe 5) was not independently observed (see below), but structurally it is
the same family as probe 6 (a decision made outside the auto-classifier
path), so this evidence is consistent with, though does not independently
prove, D13's assumption that manual denial also does not fire
`PermissionDenied`. No plan revision required; D13 as written already
accounts for this.

### D22 (`WorktreeRemove`): **inconclusive / needs-revision assumption confirmed as the safer default**

Two attempts, neither produced a `WorktreeRemove` capture:

1. An `isolation: worktree` subagent that wrote a file (uncommitted) left its
   worktree on disk for the remainder of the session (visible under
   `.claude/worktrees/agent-<id>/`); per the harness's own "auto-cleaned if
   unchanged" contract this worktree has changes, so it is not eligible for
   auto-cleanup, and no `WorktreeRemove` fired within the session.
2. A second `isolation: worktree` subagent that made zero tool calls left no
   worktree directory to clean up at all (nothing was ever materialized), so
   there was nothing for a `WorktreeRemove` event to report.

`WorktreeRemove` was not observed to fire in this Claude Code version for
either isolated-worktree-subagent pathway reachable from this session. This
does not prove the event never fires (worktree cleanup may only happen at
session end, outside what this task could observe), but it is consistent with
the plan's own stated fallback: **do not build T05 around a dedicated
`WorktreeRemove` registration actually firing during a session**; rely on
`SubagentStop` (D17) and `SessionEnd` (D18) to retire isolated-worktree
attempts instead, and treat `WorktreeRemove` as a best-effort registration
only. Recommend T02+ implement D22's stated fallback path rather than
depending on `WorktreeRemove` as load-bearing.

### D15 (`StopFailure`): **not captured — untested**

No probe in this task's list is dedicated to `StopFailure` specifically (it is
only reachable by making the main assistant thread's own turn end in
failure, which this session cannot self-trigger without actually failing the
task performing the probe). No `StopFailure` fixture exists in this
directory. Per the plan's own Open Questions, D14 (`Stop`) plus D16
(`UserPromptSubmit`) already cover the failed-turn case as a fallback if
`StopFailure` proves unreliable; T02+ should not depend on `StopFailure` as
load-bearing until a real fixture confirms it fires. This is a genuine gap,
not a pass — flagged here explicitly as the task requires.

### D11/D17/D18 (orphaned tool attempt with no terminal signal): supporting evidence from a forced-stop analog

`probe08-forced-stop-analog-no-terminal-signal.pre_tool_use.json`: a
subagent's in-flight `Bash sleep 20` was forcibly killed
(`TaskStop`) shortly after its `PreToolUse` fired. No `PostToolUse`, no
`PostToolUseFailure`, and — notably — **no `SubagentStop`** ever fired for
that subagent afterward. This is not a literal main-thread user interrupt
(probe 8 as specified), which this session cannot self-trigger, but it is a
directly analogous scenario: an attempt with a durably-recorded `pending_start`
/`active` boundary and *no* terminal signal from any lifecycle event this
adapter listens to, including the one (`SubagentStop`) the design nominates
as that attempt's primary cleanup trigger. This is strong supporting evidence
that D18's `SessionEnd` sweep must be treated as the true backstop, not an
edge case — `SubagentStop`-only cleanup (D17) is not sufficient for every
abrupt termination path, exactly the posture D11/D12/D19 already take
(prefer lost attribution over false attribution, and gate new mutation-capable
`PreToolUse` on `recovery_pending`). No plan revision required; this
reinforces the existing design rather than contradicting it.

## Probes not captured, and why

- **Probe 5 (manual permission denial):** this session's hook payloads all
  carry `"permission_mode":"auto"`. In this mode there is no human-interactive
  permission prompt for the assistant driving this task to be denied through —
  denials come either from the automatic classifier (probe 7, captured) or
  from another hook (probe 6, captured). Producing a literal human-clicked
  "deny" requires a session running in a mode that actually prompts a human
  and a human available to click deny at the right moment; neither is
  available to a single automated task-execution turn. See the D13 finding
  above for why this gap is low-risk: the design already does not treat
  manual denial as a `PermissionDenied`-signal case.
- **Probe 9 (next main-thread `UserPromptSubmit` after interruption):**
  `UserPromptSubmit` only fires when the user submits a new prompt. This
  entire task ran inside one continuous turn with no further user prompt
  after the one that started it (and the capture hook was not yet wired for
  that earlier prompt), so no `UserPromptSubmit` occurred for the hook to
  capture. Every other captured event in this set carries `session_id` and
  `cwd` identically shaped, so there is no structural reason to expect
  `UserPromptSubmit` differs; this is a session-timing gap, not a design
  concern.

Both gaps require either a differently-configured session (a prompting
permission mode) or genuine subsequent user turns, which a single
`/next-task` invocation does not have access to. They should be captured
opportunistically in a future session (a real permission denial and a real
follow-up prompt/interrupt) and appended to this directory, or accepted as
uncaptured since D13 and D14/D16 do not depend on their exact payload shape
for correctness — only on `session_id`/`tool_use_id` presence, which every
other captured event already confirms is standard across this Claude Code
version's hook payloads.

## T04 addendum: detached-descendant Bash lifecycle probe

**Revised 2026-09-05.** The original T04 capture wrote its marker to
`context/tmp/`, which `context/tmp/.gitignore` ignores wholesale. SCE's
`GitSnapshotService::capture_tree()` observes repository state through
`git read-tree HEAD` / `git add -A -- .` / `git write-tree`, which never sees
an ignored path. That evidence therefore only proved *a detached descendant
survives `PostToolUse` and writes an ignored file later* — not the stronger,
required claim that the descendant's mutation is one SCE's Git snapshot
would actually observe, and therefore one that falls outside the tool's
closed scope. This section replaces the original addendum with a corrected
capture using a non-ignored marker path and direct proof of Git-observability.
The two raw hook-payload fixtures below are real re-captured payloads from
the corrected rerun, not hand edits of the originals.

Captured live, in-session, using the exact same methodology as T01: a scratch
dump hook (`.../scratchpad/hook-capture/capture17.sh`) was registered as an
*additional* `PreToolUse`/`PostToolUse` entry for the `Bash` matcher
(alongside, not replacing, the existing SCE hook entries) in
`.claude/settings.json`, dumping raw STDIN JSON plus a wall-clock capture
timestamp to per-event files. The scratch hook was removed from
`.claude/settings.json` before this task finished; see the task's `Files
changed` record.

### Claude lifecycle evidence

- **Tested Claude Code version:** `2.1.258` (`claude --version`), same as T01
  — still the version installed in this environment.
- **Session:** `3cd16464-cf03-44f5-825b-27296fd55c34`, captured 2026-09-05
  (UTC timestamps below fall on 2026-09-04, the session's UTC day).
- **Process pattern tested:** a foreground `Bash` call with **explicit**
  `"run_in_background":false` present in the real captured `tool_input`
  (not merely omitted and defaulted) running:

  ```bash
  setsid bash -c 'sleep 3; date -u +%Y-%m-%dT%H:%M:%S.%6NZ > probe17-detached-child-write.marker' < /dev/null > /dev/null 2>&1 &
  disown
  echo "parent exiting at $(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)"
  ```

  `setsid` starts the inner `bash -c '...'` in a new session, detached from
  the invoking shell's session/controlling terminal — the shell-level
  equivalent of Python `subprocess.Popen(..., start_new_session=True)` named
  as an option in the plan. The backgrounding `&` plus `disown` and the
  redirected stdio mean the invoked (parent) shell returns immediately
  without waiting for the detached descendant; the descendant itself sleeps
  3 seconds, then writes its own wall-clock write-timestamp to
  `probe17-detached-child-write.marker` at the repository root — a
  deliberately **non-ignored** test-only path, verified below, not committed
  as a file (removed immediately after evidence capture).
- Both the `PreToolUse` and `PostToolUse` fixtures carry the identical
  `tool_use_id` `toolu_011DiMMHcxCZr6HzXWZhWzmD`, confirming they describe
  the same tool-execution attempt.

- **Observed timestamps** (all UTC):
  - `t1` — `PreToolUse` (hook capture time): `2026-09-04T22:18:09.195Z`
    (`probe17-detached-child-after-post-tool-use.pre_tool_use.json`).
  - `t2` — `PostToolUse` (hook capture time): `2026-09-04T22:18:21.678Z`
    (`probe17-detached-child-after-post-tool-use.post_tool_use.json`);
    the payload's own `tool_response.stdout` independently confirms
    `"parent exiting at 2026-09-04T22:18:21.668321Z"` with `duration_ms: 13`
    — the invoked shell itself returned in 13ms, never waiting on the
    detached descendant.
  - `t3` — descendant's actual write, timestamped by the descendant process
    itself (not the hook capture wrapper): `2026-09-04T22:18:24.674140Z`
    (~3.00s after `t2`, matching the child's own `sleep 3`).

- **Derived ordering:** `t1` (22:18:09.195) < `t2` (22:18:21.678) < `t3`
  (22:18:24.674). **`PostToolUse` fired roughly three seconds before the
  detached descendant actually wrote to the repository.**

### Git-observable mutation evidence

- **Marker path:** `probe17-detached-child-write.marker` (repository root).
- **Not gitignored:** `git check-ignore -v probe17-detached-child-write.marker`
  exited `1` (no match) both before the probe ran and again after the
  descendant's write; `git status --short -- probe17-detached-child-write.marker`
  reported `?? probe17-detached-child-write.marker` — an untracked path Git
  actually reports, not one silently swallowed by `.gitignore`.
- **Tree-hash proof, reproducing `GitSnapshotService::capture_tree()`
  exactly:** using a `GIT_INDEX_FILE`-scoped temporary index (never the real
  `.git/index`), `git read-tree HEAD` → `git add -A -- .` → `git write-tree`
  was run twice — once with the marker present (post-child-write state) and
  once with it removed (pre-child-write state), restoring the marker
  immediately after:
  - Tree **before** the child's write: `596fcceafa2ebf70a087f606d7e16645f18ee17e`
  - Tree **after** the child's write: `b24d653632e478298b625e86c99f51f4016f9f57`
  - The two tree hashes differ: **T1 ≠ T2**. The descendant's mutation is
    Git-observable and would change the tree an SCE snapshot captures.
- The temporary marker file was deleted immediately after capture and does
  not appear in the committed fixture set; see
  `probe17-detached-child-after-post-tool-use.evidence.json` for the full
  machine-readable capture metadata (versions, timestamps, tree hashes,
  ignore-check result).

### Derived SCE attribution consequence

Chaining the two evidence sections: the adapter's mutation scope for this
`Bash` tool execution closes at `PostToolUse` (`t2`), observing the tree as
it stood at that moment. The detached descendant then mutates a
non-ignored, Git-tracked-by-`add -A` repository path at `t3`, changing the
tree an SCE snapshot would capture (`T1 ≠ T2` above) — strictly after the
scope already closed. **A foreground Claude `Bash` tool call can return
`PostToolUse` while a descendant it spawned remains alive and later performs
a mutation that changes SCE's observable Git tree; `PostToolUse` is
therefore not proof that all descendant mutation activity has ended.**

### D20 disposition (T04)

This matches the plan's first anticipated disposition exactly: the
descendant's mutation is observed to land *after* `PostToolUse`, and is now
directly proven Git-observable rather than merely surviving past
`PostToolUse`. Per the plan's own instruction, D20 is updated (not left as a
theoretical boundary) to record this as a **confirmed, observed** finding
for this specific process pattern (`setsid`-based shell detachment) and this
specific Claude Code version (`2.1.258`): a foreground
(`run_in_background=false`, explicitly captured as such) `Bash` call's
`PostToolUse` closes the adapter's mutation scope before a self-detaching
descendant it spawned goes on to mutate the repository in a way SCE's own
Git snapshot would observe, so that later mutation is **not** observed
inside the tool's own scope boundary and would be misattributed (or silently
dropped) if the adapter ever treated `PostToolUse` as proof no descendant
process is still running. This does **not** generalize to `nohup`,
double-fork, or daemonizing patterns this probe did not exercise — those
remain unproven, and D20's unsupported-boundary wording stays in place
regardless, since this PR implements no detection or supervision either way,
and no static shell-command inspection is added.
