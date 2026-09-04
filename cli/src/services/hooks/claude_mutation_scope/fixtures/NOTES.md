# T01 fixture capture notes

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
