# Plan: claude-background-shell-mutation-scope

## Change summary

Add first-class mutation-lifecycle safety support for Claude Code's managed
background shell execution (`Bash` with `tool_input.run_in_background = true`)
in the Claude adapter at `cli/src/services/hooks/claude_mutation_scope/`. Today
`handle_pre_tool_use` denies that call outright with
`EXPLICIT_BACKGROUND_SHELL_DENY_REASON`, so a session in which Claude chooses
background execution (the motivating case is
`nix flake check 2>&1 | tail -40; echo "EXIT=${PIPESTATUS[0]}"`) cannot run the
command at all. This plan replaces that denial with a lifecycle that keeps the
SCE mutation scope open from `PreToolUse` until a boundary proven to occur after
the background process can no longer mutate on the normal path, with a durable
worktree inhibitor as the fail-closed fallback, so the invariant holds:

> Any repository mutation performed while a Claude-managed background command
> may still run is covered by its live SCE mutation suppressor or by the
> durable worktree safety inhibitor; neither state permits positive
> mutation-scope attribution from that background interval.

Keeping a scope open is not the same as attributing to it. A background scope
can stay live for minutes or hours, so the first implementation treats it as a
per-scope `ManagedBackgroundSuppressor`: every interval on its worktree is
`IneligibleUnscoped` while it may still run, and its termination path is
non-attributing. It can never produce `AiExclusive(background_scope)` or a
`mutation_ai_patch`/AI-lineage record. Ordinary Claude foreground scopes keep
today's mutation-scope semantics. Independently established direct tool
evidence, such as a Claude `Edit`/`Write` `diff_trace`, remains authoritative
and is not revoked by the suppressor. The protocol also gets a durable
worktree-level
`background_safety_taint` latch for the impossible state in which a possibly
live background execution has lost its live suppressor. The latch is global to
the worktree and therefore protects every harness and every boundary. That
makes `cli/src/services/mutation_trace/`, `spec/mutation_cursor.qnt`, the
Quint Connect harness, the generic ingress `start` payload, persistence, and
doctor/health part of this change.

The central safety invariant is:

> While SCE believes a Claude-managed background process may still mutate the
> worktree, SCE has either a live required-confirmation suppressor for that
> execution or a durable `background_safety_taint` on the worktree. While
> either condition holds, mutation-scope/worktree-observation evidence cannot
> produce positive AI attribution from that background interval.
> Independently established direct tool evidence, such as a Claude
> `Edit`/`Write` `diff_trace`, remains authoritative and is not revoked by the
> background suppressor.

The core safety property is:

```text
No mutation may become AI-attributed merely because
a managed-background scope is live or terminates.
```

This does not mean that no mutation can be AI-attributed while the scope is
live. A foreground Claude `Edit` with its own direct `diff_trace` may remain
AI-attributed even though the same filesystem interval is
`IneligibleUnscoped` under the mutation protocol.

Normal protocol transitions preserve the live suppressor. If a runtime
inconsistency nevertheless finds a possibly-live execution with a missing or
terminal suppressor, it must establish the durable safety latch before
processing another boundary. The latch is not ordinary `external_taint` or
`needs_rebaseline`: restart, ordinary recovery, sibling cleanup, and
`sce doctor --fix` cannot clear it. Only G5-proven process-termination
evidence followed by deterministic protocol cleanup can return the worktree to
normal attribution.

The change stays Claude-only on the adapter side: no OpenCode, Codex, or Pi
adapter change and no harness-neutral background abstraction. Self-detaching
descendants of a foreground command (`command &`, `nohup`, `setsid`,
double-fork, `start_new_session=True`) remain the separate, explicitly
unsupported D20 boundary. The plan begins with an empirical probe of the real
Claude Code lifecycle. If that probe does not show a reliable terminal event,
a stable correlation identifier, and the required ordering, the denial stays and
the plan is revised toward SCE-owned process supervision.

## Design

### Current-state findings

Verified against the code on `main` at `5dc8e2eb`.

- **Denial.** `lifecycle.rs::handle_pre_tool_use` returns
  `pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON)` when
  `events.rs::is_explicit_background_shell(tool_name, run_in_background)` is
  true (`Bash` or `PowerShell` with `run_in_background == true`). The check runs
  before `git_dir` resolution, the recovery barrier, and `establish_start`.
- **Event parsing discards the evidence a background lifecycle needs.**
  `parse_claude_hook_event` maps `PostToolUse` / `PostToolUseFailure` to a bare
  `ClaudeToolIdentity`. `tool_input` and `tool_response` are dropped, so
  `tool_response.backgroundTaskId` is unreachable. Any `hook_event_name` outside
  the twelve known names is rejected.
- **Attempt state.** `state.rs` persists
  `{version: 1, next_attempt_seq, recovery_pending, attempts[]}`; each
  `AdapterAttempt` has a `phase` of `pending_start | active | pending_abandon`.
  `parse_adapter_state` rejects any `version` other than
  `ADAPTER_STATE_VERSION`, but deserialization does not use
  `deny_unknown_fields`: a version-1 file carrying extra fields is accepted and
  the extra fields are silently dropped.
- **`PostToolUse` always terminates the attempt** (`handle_close`), and the
  `Stop`, `StopFailure`, `UserPromptSubmit`, `SubagentStop`, `SessionEnd`, and
  `WorktreeRemove` sweeps abandon every matching attempt regardless of whether
  its process is still running.
- **The recovery barrier and health classifier assume attempts are
  short-lived.** `apply_recovery_barrier` denies while
  `recovery_pending && !attempts.is_empty()`; `classify_health` reports that
  shape as `blocked`; `assess_repairability` is `AutoFixable` only when every
  attempt is `pending_abandon`.
- **Confirmation is keyed by harness, not by scope.**
  `protocol::requires_boundary_confirmation(actor_kind)` is `true` for Codex,
  OpenCode, and Pi and `false` for `ClaudeCode`;
  `spec/mutation_cursor.qnt::requiresBoundaryConfirmation` mirrors it.
  `ScopeState` holds only `status`, `actor_kind`, and `worktree_id`, and
  `mutation_trace_scopes` has no per-scope mode column. Under these rules a sole
  live Claude scope makes every interval `AiExclusive(scope)` at any boundary.
- **Taint recovery abandons live scopes.** `protocol::recover` marks every live
  scope on a tainted or externally tainted worktree `Abandoned`. A live
  background scope can therefore be retired at the protocol level, by a
  recovery some other invocation triggered, while its process is still running.
- **There is no durable worktree-wide background safety latch.** A missing or
  terminal background scope currently has no protocol representation that can
  suppress mutation-scope attribution for other harnesses after the Claude
  process has lost correlation. Ordinary `external_taint` is not suitable
  because it is transient, and `needs_rebaseline` is deliberately clearable
  by recovery.
- **Registered hooks.** `config/pkl/renderers/claude-content.pkl` registers the
  adapter, unmatched, for ten events. `Notification` and any task-completion
  event are not registered.
- **Existing evidence.** `fixtures/probe14-run-in-background-true.*` (Claude
  Code `2.1.258`) shows `PostToolUse` after `duration_ms: 8` with
  `tool_response.backgroundTaskId` and empty output. Nothing after that
  acknowledgement was captured. The installed version is now `2.1.284`.
- **Context drift to repair.** `context/cli/claude-mutation-scope-integration.md`
  says the adapter lives in `mod.rs` and `state.rs`; the code has split into
  `events.rs`, `lifecycle.rs`, `payload.rs`, `health.rs`, and `state.rs`.

### Observed Claude lifecycle contract and decision gate

Unknown until T01 runs. T01 answers each determination from captured payloads
and externally observable timestamps only.

| ID | Determination |
| --- | --- |
| G1 | `PostToolUse` for a background call fires before the process finishes and carries an identifier for the background task |
| G2 | Claude emits a later hook event when the background task ends, for success, non-zero exit, and cancellation/kill |
| G3 | A stable identifier joins the initial invocation to that terminal event |
| G4 | Ordering holds for the terminal event: `last possible child mutation < process termination <= terminal event delivery` |
| G5 | For each of `Stop`, `StopFailure`, `UserPromptSubmit`, `SubagentStop`, `SessionEnd`, `WorktreeRemove`, and Claude process exit: whether the event proves the process cannot still mutate after it (`yes` / `no` / `unknown`), with at least one boundary reachable in normal operation answering `yes` |
| G6 | The terminal event is delivered when the task ends even if the session is otherwise idle, not deferred to the next turn |
| G7 | Whether the terminal event needs a hook registration SCE does not install today, and whether a session's hook registrations come from one settings snapshot |

`unknown` is treated as `no` everywhere.

```text
T01 probe
 |
 +-- G1, G2, G3, G4 hold and G5 has at least one proven boundary
 |      -> implement the hook-driven managed background lifecycle (T02 onward)
 |
 +-- otherwise
        -> STOP after T01
        -> leave the current denial in place
        -> revise this plan through /change-to-plan toward SCE-owned
           process supervision (see Fallback direction)
```

G6 does not stop the plan; it selects the terminal action (BG-D4). G7 selects
whether the capability gate needs a registration marker (BG-D5).

### Two separate concerns

**Adapter lifecycle** (`state.rs`, `lifecycle.rs`) tracks facts about one tool
execution: foreground, background awaiting its identifier, background
correlated and running, background unresolved, pending abandon, terminal. It
decides which seam operation to send and when.

**Mutation protocol semantics** (`protocol.rs`, `spec/mutation_cursor.qnt`)
decide whether a worktree delta observed at a boundary is `AiExclusive`,
`AiContended`, or `IneligibleUnscoped`.

A scope being open in the adapter never by itself entitles it to positive
attribution. The adapter keeps the scope open to satisfy the invariant; the
protocol decides what, if anything, is attributed.

### Evidence channels

SCE keeps direct tool evidence distinct from filesystem/mutation-scope
evidence. Direct coverage is resolved first, and mutation-history attribution
only processes lines not already covered by direct evidence.

```text
Direct evidence
    Claude Edit / Write / structured diff
            ↓
        diff_trace
            ↓
independent positive evidence
            ↓
remains authoritative


Filesystem/mutation-scope evidence
    observed Git tree delta
            ↓
managed background suppressor live
            ↓
    IneligibleUnscoped
            ↓
no mutation_ai_patch / mutation-lineage AI
```

The managed-background suppressor affects mutation-scope attribution only. It
does not delete, downgrade, mask, or rewrite independently established direct
`diff_trace` evidence, and it does not change Claude `Edit`/`Write` collection
or direct-evidence precedence.

### Design decisions

**BG-D1 — Adapter state version 2.** Background-aware state is written as
`ADAPTER_STATE_VERSION = 2`. The new binary reads version 1 and version 2,
interprets every version-1 attempt as a foreground attempt, and always writes
version 2. No background attempt is ever written under version 1. An older
binary rejects a version-2 file through its existing version check. Version 2
must represent background-awaiting-identifier, correlated-running,
`background_unresolved`, and `pending_terminal_recovery` distinctly; the
protocol's durable `background_safety_taint` remains authoritative when the
adapter and protocol disagree.

**BG-D2 — Unresolved background has two explicitly different cases.** A
managed background execution that SCE can no longer correlate with a terminal
event is classified against the protocol suppressor before the adapter chooses
an outcome.

**Case A: correlation lost, suppressor still live.** Enter persisted
`background_unresolved` (final name chosen in T05). The protocol scope remains
`Active` with `ManagedBackgroundSuppressor`; it is never closed or abandoned
without termination evidence. The attempt remains persisted, every new
mutation-capable Claude `PreToolUse` in that checkout is denied, the recovery
barrier cannot flush past it, and `sce doctor --fix` does not touch it. All
boundaries from every harness remain `IneligibleUnscoped` because the live
suppressor is worktree-wide. A G5-proven lifecycle boundary is the only way
to leave this case.

**Case B: correlation lost or inconsistency found, suppressor not live.** If a
possibly-live managed background attempt is paired with a missing, `Closed`, or
`Abandoned` required scope, this is not ordinary `background_unresolved` and is
an invariant violation in the normal protocol path, not a valid lifecycle
transition. It is not repaired by the Claude adapter. Atomically establish the durable
worktree-level `background_safety_taint` with the attempt/scope identity and
reason, then force `IneligibleUnscoped` for every harness and every boundary.
Admission, mutation-scope attribution, ordinary `recover`, rebaseline, flush,
sibling abandonment, and `sce doctor --fix` remain blocked. The latch survives
process restart and can clear only after a G5-proven lifecycle boundary
demonstrates
that the process can no longer mutate, followed by deterministic protocol
cleanup. If establishing the latch fails, the operation fails closed and emits
no boundary that could produce positive mutation-scope attribution.

Entry conditions include a background `PostToolUse` without a usable
identifier, conflicting identifiers, a `pending_start` whose idempotent
`start` cannot be durably replayed, any correlation loss, and the impossible
terminal-scope observation above. The implementation must not use the same
`background_unresolved` phase for Case B.

**BG-D3 — Managed background is a non-attributing protocol mode.** The
protocol gains a durable per-scope mode fixed at registration and never
weakened: the existing harness default for ordinary scopes, or
`ManagedBackgroundSuppressor` for a Claude managed background. The latter
implies required confirmation and an execution lease that is potentially live.
The Quint model, Rust refinement, and MBT must prove:

- for mutation-scope attribution, while the lease is potentially live, every
  interval on that worktree is `IneligibleUnscoped`, including intervals
  observed at `Start`, `Advance`, `Close`, `Flush`, recovery, and boundaries
  from another harness;
- a managed-background scope cannot be closed or abandoned by a generic
  boundary, `recover`, external-taint recovery, sibling cleanup, or replay;
- only a dedicated non-attributing termination transition carrying G4/G5 proof
  can consume the lease and end the scope;
- `background_safety_taint` has the same attribution result even if the scope
  is missing or terminal;
- no mutation event processed by the mutation protocol while either condition
  holds can carry `AiExclusive`, `mutation_ai_patch`, or positive
  mutation-lineage coverage;
- these protocol properties make no claim about independently established
  direct evidence. A direct Claude `Edit`/`Write` `diff_trace` remains
  authoritative and is not changed by this mode.

The mode's guarantee is therefore:

```text
For mutation-scope attribution:
    every interval while A may run -> IneligibleUnscoped

For direct attribution:
    independently established direct evidence is unchanged
```

The existing Claude foreground default remains unchanged. Codex, OpenCode, and
Pi defaults remain unchanged. If the protocol cannot preserve the lease across
all recovery and replay paths, T02 stops and the plan is revised before any
background admission work.

**BG-D4 — Terminal action follows proof and never attributes.** Success,
non-zero exit, and cancellation are execution outcomes, not attribution
failures. Once G4 or a G5 boundary proves the process cannot mutate, use the
dedicated non-attributing terminal operation (named `terminate_background` in
this plan; final API name chosen in T03). It may implement the state change as
`Abandon`/`consume` rather than `Close`, but it must atomically consume the
background lease, preserve `IneligibleUnscoped` for the final interval, and
never create `AiExclusive(background_scope)`.

- Terminal event delivered promptly: `terminate_background` with its outcome.
- Terminal event delayed until later: keep the suppressor live and
  non-attributing until the event arrives or a G5-proven boundary retires it;
  a late duplicate is a no-op.
- G5-proven session/process boundary without a terminal event:
  `terminate_background` with that proof; `Close` is not used.
- Terminal transition failure: retain the live suppressor or the durable safety
  latch, persist `pending_terminal_recovery`, and retry only deterministic
  cleanup. The failure is fail-closed and cannot re-enable mutation-scope
  attribution.
- Correlation loss with a live suppressor follows BG-D2 Case A. Correlation
  loss with a missing/terminal suppressor follows Case B.
- Never use `PostToolUse` acknowledgement, timeouts, TTLs, duration polling, or
  unproven process-exit assumptions as termination evidence.

**BG-D5 — Admission requires a verified completion capability.** If G7 shows
the terminal event needs a registration SCE does not install today,
`PreToolUse(Bash, run_in_background=true)` stays denied unless both hold:

1. the `PreToolUse` invocation itself carries a registration-contract marker
   that the generated settings emit only together with the terminal-event
   registration, in a form older binaries ignore (an environment assignment in
   the generated hook command, not a new CLI argument). Because a session's
   hooks come from one settings snapshot (to be confirmed by G7), the marker on
   the invocation proves the same snapshot holds the terminal registration;
2. the project `.claude/settings.json` contains an SCE-owned
   `claude-mutation-scope` handler for the terminal event, by the same
   ownership predicate setup and doctor already use.

```text
background capability unavailable  -> deny background Bash (stable reason naming `sce setup`)
background capability verified     -> admit managed background Bash
```

An upgraded CLI with ungenerated settings therefore keeps denying. After
`sce setup` and a new Claude session, admission succeeds. If G7 shows no new
registration is needed, the capability is unconditionally available and no
marker is added. If G7 shows a session's hooks do not come from one snapshot,
the marker proof is unsound and T11 stops for a plan revision.

**BG-D6 — Recovery barrier and safety-latch recovery.** Decided after BG-D3
and dependent on its proof.

| Persisted state at a mutation-capable `PreToolUse` | Outcome |
| --- | --- |
| `background_unresolved` with live suppressor | deny; no flush; nothing cleared |
| `background_safety_taint` | deny; no flush, rebaseline, or ordinary recovery |
| `recovery_pending` with any live managed background, including awaiting identifier | deny or perform only a guarded non-attributing rebaseline that preserves the live lease |
| `recovery_pending` after G5 proof with `pending_terminal_recovery` | retry deterministic non-attributing cleanup; never use `Close` |
| no background safety condition and no `recovery_pending` | proceed with existing semantics |

Recovery may abandon ordinary sibling scopes only when it leaves every possibly
live managed background lease active. Implement this by changing `recover` or
adding a guarded `recover_managed_background` variant; do not retain the
current unconditional live-scope abandonment. If preserving that lease is
impossible, the whole recovery is a guarded no-op and the worktree remains tainted. A
`background_safety_taint` is never cleared by `recover`, external-taint
recovery, `Flush`, `repair_blocked`, or doctor. Only the G5 proof plus the
dedicated termination/cleanup transition may clear it. Process restart reads
the same persisted lease/latch state before any boundary is accepted.

The named cases are therefore:

- correlated A + foreground B: B's interval and every interval under A are
  `IneligibleUnscoped`; B may retain today's lifecycle only if A's suppressor
  survives it;
- unresolved A + foreground B: B cannot bypass the live suppressor and the
  barrier remains blocked until A has proof;
- A's terminal transition fails after proof: A remains non-attributing and
  pending cleanup; the latch/suppressor stays in force until retry succeeds;
- A is alive across separate CLI invocations: persisted state and protocol
  lease survive restart, so mutation-scope attribution stays ineligible;
- missing/terminal suppressor while A may live: establish the durable latch,
  and reject ordinary recovery until G5 evidence exists.

**BG-D7 — Health and repairability.** Health keeps the shared
`healthy | recovering | blocked | invalid` statuses. Repairability gains a
third fact beside `AutoFixable` and `ManualOnly`: blocked awaiting lifecycle
evidence.

| Condition | Status | Repairability |
| --- | --- | --- |
| correlated managed background suppressor live | `healthy` | none |
| background awaiting its identifier with live suppressor | `healthy` | none |
| unresolved background with live suppressor | `blocked` | blocked awaiting lifecycle evidence |
| background safety-tainted because its suppressor is missing/terminal | `blocked` | blocked awaiting lifecycle evidence; never auto-fixable |
| process termination proven, non-attributing terminal cleanup pending | `blocked` | `AutoFixable` only for deterministic retry, never to clear an unproven latch |
| unreadable, malformed, or unsupported-version state | `invalid` | `ManualOnly` |

`sce doctor --fix` never closes, abandons, flushes past, rebaselines, or
clears a possibly-live background attempt or `background_safety_taint`. It may
retry deterministic cleanup only when G5 termination evidence is already
persisted and the safety latch can be cleared in the same guarded protocol
transition.

**BG-D8 — Mid-flight backgrounding.** If T01 captures a foreground-started
command that Claude moves to the background (a `PostToolUse` carrying a
background identifier for an attempt whose `PreToolUse` did not set
`run_in_background`), the adapter first atomically strengthens that live scope
to `ManagedBackgroundSuppressor` before processing the acknowledgement. It
does not close the foreground scope or produce positive mutation-scope
attribution for the interval. The same attempt is then correlated with the
observed identifier. If the protocol
cannot strengthen it atomically, the adapter establishes
`background_safety_taint` and fails closed. If T01 cannot capture the path, it
is documented beside D20 as an unsupported boundary whose behavior this plan
does not change.

### Required protocol invariants

The Quint model and Rust refinement must state these as safety properties, not
as adapter assumptions:

1. For every managed-background execution whose lifecycle is not G5-proven
   terminated, either its protocol scope is `Active` with
   `ManagedBackgroundSuppressor`, or its worktree is durably marked
   `background_safety_taint`.
2. `recover`, external-taint recovery, adapter recovery, abandonment of
   sibling scopes, duplicate/replayed boundaries, and ordinary flush/rebaseline
   cannot terminate the last live suppressor or clear the safety latch while
   that execution may still mutate. They either preserve the suppressor, leave
   the worktree otherwise tainted, or establish the latch.
3. If either the live suppressor or the safety latch holds, every mutation
   event processed by the mutation protocol for that worktree has
   `IneligibleUnscoped` attribution. No `AiExclusive` result,
   `mutation_ai_patch`, or mutation-lineage AI coverage is possible from the
   managed-background interval.
4. Only a dedicated termination transition carrying G4/G5 evidence may
   consume the lease. It must be non-attributing. Only that proven transition
   may clear `background_safety_taint`, and only after deterministic cleanup
   succeeds.

These invariants cover only the mutation-scope/worktree-observation evidence
channel. They do not model `diff_traces` or revoke independent direct tool
evidence; direct coverage is resolved before mutation-history attribution.

The refinement tests must exercise both the normal live-suppressor branch and
the missing/terminal-suppressor fallback branch, including restart between
every state-changing step. If any protocol action can violate one of these
properties, implementation stops and this plan is revised before Claude
background admission.

### State transitions

Foreground (unchanged):

```text
PreToolUse -> pending_start -> seam start -> active
PostToolUse | PostToolUseFailure -> seam close -> attempt removed
```

Managed background:

```text
PreToolUse(run_in_background=true), capability verified
    -> pending_start -> seam start(ManagedBackgroundSuppressor) -> background awaiting identifier
PostToolUse with identifier
    -> background running(identifier); no seam call
PostToolUse without usable identifier, or correlation otherwise lost
    -> Case A background unresolved (live suppressor, admission blocked)
    -> Case B background safety-tainted (missing/terminal suppressor, global inhibitor)
terminal event(identifier), G4+G6
    -> pending_terminal_recovery -> non-attributing terminate_background -> attempt removed
terminal event(identifier), G6 not holding; or G5-proven boundary
    -> pending_terminal_recovery -> non-attributing terminate_background -> attempt removed
background unresolved + G5-proven boundary
    -> pending_terminal_recovery -> non-attributing terminate_background -> attempt removed
background safety-tainted + G5-proven boundary
    -> guarded cleanup clears inhibitor and consumes terminal protocol record -> attempt removed
```

`ScopeId` and `EventId` derivation are unchanged.

### Overlapping scopes

The protocol does not assume serialized scopes; mutation-scope attribution is
computed from the set of live scopes at each boundary. A managed-background
scope is a worktree suppressor, not a positive mutation-scope attribution
source. Use:

```text
A = managed-background Bash
B = foreground Claude Edit with direct diff_trace
H = unrelated filesystem/human/process mutation
```

For these scopes and mutations:

| Boundary | Live before | Interval attribution | Why |
| --- | --- | --- | --- |
| `Start(A)` | none | `IneligibleUnscoped` | no live scope |
| `Start(B)` | `{A}` | `IneligibleUnscoped` | A's live suppressor covers every boundary |
| `Close(B)` (B mutated) | `{A, B}` | `IneligibleUnscoped` | A suppresses the whole worktree |
| terminal proof for A | `{A}` until guarded transition | `IneligibleUnscoped` | termination is non-attributing |
| A removed after cleanup | none | normal future attribution | no A interval is attributed |

- A's mutation follows the mutation protocol only:

  ```text
  background Bash A writes file
          ↓
  filesystem observation
          ↓
  ManagedBackgroundSuppressor live
          ↓
  IneligibleUnscoped
          ↓
  no mutation_ai_patch
  no positive mutation-lineage coverage from A
  ```

- B's mutation follows both channels:

  ```text
  Claude Edit B while A is running
          ↓
  direct diff_trace for B
          ↓
  B remains directly AI-attributed

  same filesystem interval observed by mutation protocol
          ↓
  IneligibleUnscoped
          ↓
  no mutation-scope AI attribution
  ```

  B's directly covered line remains AI-attributed with B's direct
  session/model/tool provenance. The suppressor does not erase B's
  `diff_trace`, and mutation history does not use A's observation to
  supplement B's direct evidence.
- H follows the mutation protocol without direct evidence:

  ```text
  human/editor/unrelated process writes file
          ↓
  no direct evidence
          ↓
  mutation scope sees IneligibleUnscoped
          ↓
  not AI-attributed
  ```

- A's mutations before, during, and after B are all `IneligibleUnscoped` in
  the mutation protocol. A's terminal proof does not convert the final
  interval into mutation-scope attribution:

  ```text
  G4/G5 termination proof
          ↓
  non-attributing terminate_background
          ↓
  no AiExclusive(A)
  no mutation_ai_patch from A
  ```

- After A has been safely cleaned up, future ordinary foreground tools resume
  normal direct and mutation-scope attribution semantics. The safety property
  is that background execution cannot create false-positive AI attribution,
  while stronger independent direct evidence remains usable.

Mutation-scope coverage is deliberately given up in favor of avoiding false
positives; independently proven direct coverage is preserved.

### Fallback direction if the lifecycle signal is insufficient

Not implemented by this plan. If the decision gate stops the plan, the next
investigation is an SCE-supervised process lifetime:

```text
Claude Bash(run_in_background=true)
    -> PreToolUse
    -> SCE rewrites the tool input so the command runs under an SCE-owned supervisor
    -> supervisor launches the command and observes the real process exit
    -> SCE closes the mutation scope at that exit
```

That investigation would need to establish whether the tested Claude Code
version honors modified tool input from a `PreToolUse` hook for `Bash`, how the
rewrite interacts with Claude's permission system and the SCE bash-policy hook,
and whether the existing `external-mutation-guard` supervisor precedent can be
reused. Timeouts, TTLs, duration polling, closing on `PostToolUse`, closing on
`Stop` without proof, and abandoning a possibly-running process are excluded
from the fallback as well.

### Affected files and modules

- `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/{types,protocol,store}.rs`,
  `cli/src/services/mutation_trace/mbt/`, a new
  `cli/migrations/agent-trace-repository/006_*.sql` — per-scope mode,
  non-attributing termination, durable background-safety latch, and migration.
- `cli/src/services/mutation_trace/runtime/coordinator.rs`,
  `cli/src/services/hooks/mutation_scope.rs` — carry the mode on `Start`.
- `cli/src/services/hooks/claude_mutation_scope/{events,state,lifecycle,health,payload}.rs`,
  `tests.rs`, `fixtures/`.
- `cli/src/services/hooks/mutation_scope_health.rs` and doctor rendering.
- `config/pkl/renderers/claude-content.pkl`, setup merge, doctor registration
  checks — only if G7 requires a registration.
- `cli/src/services/hooks/mod.rs` (`mutation_provenance_e2e`).

### Migration and compatibility

- **Adapter state upgrade.** The new binary reads a version-1 file, treats
  every attempt as foreground, and rewrites it as version 2 on its next write.
  No background attempt can exist in a version-1 file, because background
  admission is denied by every binary that writes version 1.
- **Adapter state downgrade.** An older binary rejects a version-2 file
  ("unsupported version"), so every mutation-capable `PreToolUse` is denied and
  health is `invalid` / `manual_only` until the newer binary is restored. It
  never reads a background attempt as a foreground `active` attempt. This is
  the reason additive fields under version 1 are not used: the older binary
  would accept them, drop them, and apply foreground cleanup to a running
  background process.
- **Agent Trace DB.** The per-scope mode and durable worktree safety latch are
  additive migration state. Existing scope rows resolve to the harness
  default, and existing worktrees have no latch. The migration must persist
  the mode and a worktree-keyed `background_safety_taint` record (including
  reason, scope/attempt identity, and lifecycle-proof status) in the same
  transaction as protocol transitions. A binary older than the migration sees
  an unexpected applied migration and must fail closed before loading or
  accepting mutation-scope boundaries; it must never ignore the latch and
  produce positive mutation-scope attribution. T03 records the observed
  downgrade behavior.
- **Ingress.** The `start` payload gains one optional key. Payloads without it
  behave exactly as today, so Codex, OpenCode, and Pi are unaffected.
- **Hook registration.** Covered by BG-D5: stale settings keep background Bash
  denied rather than admitted without a terminal event.
- **PowerShell.** Not probeable in this Linux environment; stays denied.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Raw, unmodified hook payloads with capture timestamps and
  externally observable write timestamps exist for every T01 probe case (or
  the case is recorded as not capturable with its reason), and
  `fixtures/NOTES.md` states the tested Claude Code version, answers G1–G7,
  gives the per-event "process cannot still mutate after this event" table,
  and records the decision-gate outcome.
  - Validate: inspect `cli/src/services/hooks/claude_mutation_scope/fixtures/`
    and the `NOTES.md` background-lifecycle addendum; every G-row cites its
    fixtures and timestamps, and the G4 ordering is shown by timestamps written
    by the child process itself.
- [ ] AC2: `context/cli/claude-mutation-scope-background-execution.md`
  describes the managed-background lifecycle using only observed behavior,
  names the tested version, and still documents self-detaching descendants as
  unsupported.
  - Validate: read the file; every lifecycle claim traces to a fixture named in
    `NOTES.md`.
- [ ] AC3: A scope registered as `ManagedBackgroundSuppressor` suppresses
  positive mutation-scope attribution on its worktree at every boundary,
  including its terminal boundary; it cannot be closed or abandoned while
  potentially live; its mode cannot be weakened; and no mutation-scope AI
  attribution occurs while the execution is unterminated, including across
  `recover`, external-taint recovery, sibling cleanup, duplicate boundaries,
  and process restart. Independently established direct evidence remains
  authoritative. A required-confirmation scope that is not managed background
  retains its existing semantics, and Claude foreground behavior is
  unchanged.
  - Validate: `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC4: A `start` ingress payload can request the managed-background
  suppressor mode, a payload without the key behaves as today, the key is
  rejected on every other operation, a weaker replay is rejected, and the mode
  is durable in `mutation_trace_scopes`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope`
- [ ] AC5: A version-1 adapter state file loads with every attempt treated as
  foreground; every write by the new binary is version 2; a file with an
  unsupported version is rejected and `PreToolUse` denies; the version-1 parser
  rule (reject any other version) rejects a version-2 file.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope::state`
- [ ] AC6: `PreToolUse` for `Bash` with `run_in_background = true` is admitted
  only when the completion capability is verified: with stale Claude settings
  it is denied with the documented reason, and after setup makes the
  registration current it returns empty stdout, commits a `start` requesting
  `ManagedBackgroundSuppressor` before the command runs, and persists a
  background attempt.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`;
    `nix run .#pkl-check-generated`
- [ ] AC7: The background `PostToolUse` persists the stable identifier, sends
  no seam call, and leaves the attempt; a later separate invocation reading
  only the state file still correlates the terminal event.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC8: The terminal event proves lifecycle completion for success,
  non-zero exit, and cancellation, then uses the BG-D4 non-attributing terminal
  action; a duplicate terminal event is a no-op; an unknown identifier leaves
  the state file byte-identical; and a failed terminal transition leaves
  `pending_terminal_recovery` with the suppressor or safety latch still
  active. No case uses `Close` to attribute the final interval.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC9: A background `PostToolUse` with no usable identifier enters Case A
  only when the required protocol suppressor is still live: it sends neither
  `close` nor `abandon`, persists `background_unresolved`, denies later
  mutation-capable admission, and leaves recovery/doctor unable to clear it.
  If the scope is missing, `Closed`, or `Abandoned`, the same input enters Case
  B instead and durably establishes `background_safety_taint`; it is not called
  ordinary unresolved and cannot be cleared by recovery or doctor. Only a
  G5-proven boundary can retire either case.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC10: For each of `Stop`, `StopFailure`, `UserPromptSubmit`,
  `SubagentStop`, `SessionEnd`, and `WorktreeRemove`, a test fixes whether the
  event is terminal for a managed background attempt, matching the T01 table:
  unproven events leave running and unresolved background attempts untouched,
  proven events abandon them, and foreground attempts are swept exactly as
  before.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC11: Every row of the BG-D6 barrier table and each named recovery case
  is pinned by a test. `recover`, external-taint recovery, sibling abandonment,
  replay, flush, and rebaseline cannot remove the last live suppressor or clear
  `background_safety_taint` while the process may still run. A failed terminal
  transition remains fail-closed.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC12: `sce doctor` distinguishes a healthy correlated background
  suppressor, background awaiting its identifier, unresolved with a live
  suppressor, safety-tainted because the suppressor is missing/terminal,
  terminal awaiting protocol recovery, and invalid persisted state; reports
  blocked safety-taint as not auto-fixable; names each background attempt's
  scope, session, agent, `tool_use_id`, identifier, and latch reason; and
  `sce doctor --fix` does not alter an unresolved/running attempt or clear the
  safety latch.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope::health`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
- [ ] AC13: Structured log events exist for background started, identifier
  bound, background completed, and background unresolved/recovery required,
  carrying `session_id`, `agent_id`, `tool_use_id`, `scope_id`, the background
  identifier, and the terminal outcome where each applies.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
- [ ] AC14: Against a real Git repository and Agent Trace DB, use
  `A = managed-background Bash`, `B = foreground Claude Edit with direct
  diff_trace`, and `H = unrelated filesystem/human/process mutation`. A's
  mutation protocol attribution is `IneligibleUnscoped`, with no A lines in
  `mutation_ai_patch` and no positive mutation-lineage provenance. B's
  mutation protocol attribution for the same interval is also
  `IneligibleUnscoped`, but B's direct `diff_trace` survives, its directly
  covered line remains AI-attributed, and its direct session/model/tool
  provenance remains authoritative. No mutation evidence from A supplements
  B's direct evidence. H has no direct evidence and is not AI-attributed.
  A's terminal proof uses the non-attributing terminal transition, with no
  `AiExclusive(A)`, `mutation_ai_patch`, or mutation-lineage coverage from A;
  the recorded order is `scope START` → `PostToolUse` → delayed mutation →
  terminal proof → non-attributing cleanup. After A is safely removed, a new
  ordinary foreground scope resumes the pre-existing direct and mutation-scope
  attribution behavior.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_provenance_e2e`
- [ ] AC15: The foreground lifecycle is unchanged, `PowerShell` with
  `run_in_background = true` is still denied, and a foreground command with a
  self-detaching descendant still closes at `PostToolUse` with D20 documented
  as unsupported.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`;
    pre-existing foreground and `probe17` assertions pass unmodified.
- [ ] AC16: In a real Claude Code session, a backgrounded
  `nix flake check 2>&1 | tail -40; echo "EXIT=${PIPESTATUS[0]}"` is not
  denied, its managed-background suppressor is live while it runs, the
  terminal event ends it through the non-attributing path, and afterwards the
  adapter state shows `"attempts": []` and `"recovery_pending": false`; no
  `mutation_ai_patch` or AI-lineage row is emitted for that background scope.
  - Validate: inspect the live-acceptance fixtures and `NOTES.md` section
    recorded by T13; re-run with
    `nix run .#turso -- --experimental-multiprocess-wal --readonly "<agent-trace-db-path>" "SELECT actor_kind, status, COUNT(*) FROM mutation_trace_scopes GROUP BY actor_kind, status;"`.
- [ ] AC17: In a real Claude Code session, `sleep 3` followed by
  `echo test >> <tracked-file>` run with `run_in_background = true` yields the
  AC14 ordering in the persisted trace and logs, with the appended line
  recorded as `IneligibleUnscoped` and no `mutation_ai_patch`/AI-lineage record.
  - Validate: inspect the T13 fixtures; compare the adapter log timestamps, the
    file's write timestamp, and the `mutation_trace_*` rows read through the
    repository Turso CLI.
- [ ] AC18: Against a real Git repository and Agent Trace DB, seed a possibly
  live background attempt whose required protocol scope is unexpectedly
  `Closed` or `Abandoned`. The next boundary durably establishes
  `background_safety_taint`; every subsequent boundary from Claude, Codex,
  OpenCode, and Pi is `IneligibleUnscoped`; doctor reports
  `blocked-awaiting-lifecycle-evidence`; and ordinary `recover`, rebaseline,
  and `sce doctor --fix` cannot clear it. After a G5-proven termination
  boundary, deterministic cleanup clears the latch and a new ordinary
  foreground scope is eligible again. Repeat after SCE process restart.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_provenance_e2e`; inspect the DB with the repository Turso CLI.
- [ ] AC19: MBT/refinement covers live suppressor, recover, external-taint
  recovery, sibling abandonment, duplicate/replayed boundaries, terminal
  transition failure, safety-latch establishment, restart, and G5 cleanup; the
  Quint invariant is stated precisely: if a managed background is not proven
  terminated, either its live suppressor or the durable safety latch exists,
  and while either exists mutation-scope evidence cannot produce
  `AiExclusive`, `mutation_ai_patch`, or positive mutation-lineage coverage.
  The protocol makes no claim about independent direct evidence.
  - Validate: `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/claude-mutation-scope-background-execution.md` — observed
  lifecycle, tested version, retained D20 boundary, mid-flight disposition.
- `context/cli/claude-mutation-scope-integration.md` — state version 2,
  phases, `PostToolUse` behavior, cleanup table, barrier, capability gate, and
  the module-layout drift noted above.
- `context/cli/claude-mutation-scope-health.md`,
  `context/sce/mutation-scope-health-status.md`,
  `context/sce/doctor-human-text-contract.md` — statuses and the third
  repairability fact.
- `context/cli/mutation-trace-protocol.md`,
  `context/cli/mutation-scope-runtime.md`,
  `context/cli/mutation-trace-store.md`,
  `context/cli/mutation-trace-quint-connect.md`,
  `context/cli/mutation-scope-hook-ingress.md` — per-scope confirmation.
- `context/cli/doctor-recovery-safety-model.md` — mappings affected by the
  barrier and repair changes.
- `context/sce/claude-raw-hook-capture.md`,
  `context/sce/agent-trace-hooks-command-routing.md` — only if a registration
  is added.
- `context/overview.md`, `context/glossary.md`, `context/context-map.md`, and
  a decision record for conservative managed-background suppression and the
  durable safety latch.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/claude_mutation_scope/**`;
  `spec/mutation_cursor.qnt`; `cli/src/services/mutation_trace/` protocol,
  types, store, MBT harness, and the coordinator's `Start` and guarded
  non-attributing termination boundaries; one additive Agent Trace DB
  migration for per-scope mode and the durable worktree safety latch; the
  generic ingress `start` payload; the shared mutation-scope health types and
  doctor rendering;
  `config/pkl/renderers/claude-content.pkl` and its setup/doctor consumers when
  a registration is required; the Claude cases in `cli/src/services/hooks/mod.rs`;
  the context files listed under Context sync.
- **Out of scope:** OpenCode, Codex, and Pi adapters and their confirmation
  behavior; changing the default confirmation rule for any harness; background
  subagents (already supported through their own tool scopes); `PowerShell`
  background execution; the supervised-process fallback.
- **Constraints:** every `PreToolUse` failure path stays fail-closed with
  Claude's deny decision; the adapter reaches the runtime only through
  `run_mutation_scope_from_payload`; the adapter-state lock is never held
  across a seam call; success, non-zero exit, and cancellation are execution
  outcomes, not attribution failures; a scope is closed or abandoned only at a
  boundary proven to occur after its process can no longer mutate; a managed
  background scope uses a non-attributing terminal path even after such proof;
  the Bash
  denial is removed only by T12, after protocol semantics, lifecycle,
  unresolved blocking, recovery, health, and the capability gate exist;
  attribution choices prefer a false negative over a false positive; context
  documents record observed behavior only; all commands run through Nix per
  `AGENTS.md`; context files respect the repository's per-file line budget.
- **Non-goal:** making all Claude scopes confirmation-required.
- **Non-goal:** detecting, supervising, or statically scanning for
  self-detaching descendants (D20).
- **Non-goal:** a harness-neutral background execution model.
- **Non-goal:** timeouts, TTLs, duration polling, or PID tracking as a
  substitute for an observed lifecycle boundary.
- **Non-goal:** automatically resolving a background attempt left behind by a
  Claude process that died without emitting any proven boundary. It stays
  persisted, keeps its worktree non-attributing (with the live suppressor or
  safety latch), and is reported by doctor.

## Assumptions

- The probe follows the T01 methodology already recorded in
  `fixtures/NOTES.md`: a scratch dump hook registered alongside the existing
  SCE entries in `.claude/settings.json`, removed before the task finishes.
- New fixtures continue the existing `probeNN-<slug>.<event>.json` naming,
  starting at `probe18`.
- `PowerShell` with `run_in_background = true` keeps its current denial and
  reason text.
- Managed-background mutation-scope attribution is intentionally conservative
  in v1: no interval, including the final interval at termination, may be
  positively attributed to the background scope. Independently established
  direct evidence is not affected; mutation-scope false negatives are
  accepted.
- New log events follow the existing `sce.hooks.claude_mutation_scope.<name>`
  convention.
- Plan `Verify` commands use the repository Cargo wrapper form from
  `AGENTS.md`; `nix flake check` remains the canonical verification.

## Task stack

- [x] T01: `Probe and record the Claude managed-background shell lifecycle` (status:done)
  - Task ID: T01
  - Scope: In — on the installed Claude Code version, capture raw hook payloads
    with wall-clock capture timestamps, and make the background command write
    its own timestamps to a tracked, non-ignored path (including a write as its
    last action and a repeating write for kill cases) so ordering is observed,
    for: (1) successful background Bash; (2) non-zero exit; (3)
    cancellation/kill; (4) another tool running while it remains active,
    including a foreground `Edit`/`Bash` that opens and closes during it; (5)
    `Stop`; (6) `StopFailure`; (7) `UserPromptSubmit`; (8) `SubagentStop`,
    with a background Bash started by that subagent; (9) `SessionEnd`; (10) a
    repository mutation after the initial `PostToolUse`; (11) process behavior
    when Claude itself exits; (12) whether the terminal event arrives while the
    session is otherwise idle; (13) a foreground command moved to the
    background mid-flight, if producible; (14) whether hook registrations
    edited mid-session take effect and whether a session's hooks come from one
    settings snapshot. Register the capture hook for `PreToolUse`,
    `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop`, `StopFailure`,
    `SubagentStop`, `SessionEnd`, `UserPromptSubmit`, `PostToolBatch`, and every
    other hook event the installed version exposes that could report
    background-task completion. Preserve per event: timestamp, `session_id`,
    `agent_id`, `tool_use_id`, `tool_name`, `tool_input`, `tool_response`, any
    background/task/process identifier, and the raw payload. Store fixtures
    under `fixtures/`, add a `NOTES.md` addendum answering G1–G7 with the
    per-event table and the decision-gate outcome, and update
    `context/cli/claude-mutation-scope-background-execution.md` with observed
    behavior and the tested version. Out — any Rust change; any change to the
    denial; committing the scratch hook or marker files.
  - Dependencies: none
  - Done when: every probe case is captured or recorded as not capturable with
    its concrete reason; G4 is shown as
    `last child write < process termination <= terminal event` from timestamps
    the child wrote; each listed lifecycle event has a `yes`/`no`/`unknown`
    answer for "process cannot still mutate after this event"; case 10 is shown
    Git-observable the way `probe17` was; the decision-gate outcome (proceed,
    or stop and revise toward supervision) is written in `NOTES.md`;
    `.claude/settings.json` is back to its pre-task content.
  - Verify: `git status --short` shows only new fixture files, `NOTES.md`, and
    the context document; `git diff -- .claude/settings.json` is empty;
    `claude --version` matches the version recorded in `NOTES.md`.
  - Completed: 2026-10-01
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json`, `cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md`, `context/cli/claude-mutation-scope-background-execution.md`
  - Result: Captured the current Claude Code version and the isolated probe attempt. Claude 2.1.284 reached SessionStart but produced no model response or tool/lifecycle event within the probe timeout; all 14 cases were recorded as not capturable for this run, with prior 2.1.258 fixtures retained as prior-version evidence. G1-G7 remain unknown, so the decision gate is stop and revise toward SCE-owned process supervision; the existing background denial was unchanged.
  - Verify: Passed before the lifecycle write: `git status --short --untracked-files=all` listed only the new fixture, `fixtures/NOTES.md`, and the context document; `git diff -- .claude/settings.json` was empty; `claude --version` reported `2.1.284`, matching `NOTES.md`. The evidence fixture also passed JSON parsing and `git diff --check` passed.
  - Context impact: repository-wide behavior and Claude adapter lifecycle boundary; updated the background-execution context document and fixture notes with the current-version probe limitation and the stop decision so T02 cannot assume an unproven terminal contract.
  - Context synchronization: synced

- [ ] T02: `Define and prove conservative managed-background protocol safety` (status:todo)
  - Task ID: T02
  - Scope: In — specify the smallest protocol extension: a durable per-scope
    `ManagedBackgroundSuppressor` mode/lease, a guarded non-attributing
    `terminate_background` transition, and a durable worktree-level
    `background_safety_taint` fallback. In `spec/mutation_cursor.qnt`, state
    the invariant that an unproven background execution has either a live
    suppressor or the latch, and that either condition makes all
    mutation-scope attribution `IneligibleUnscoped`. Model `recover`,
    external-taint recovery, sibling
    abandonment, duplicate/replayed boundaries, missing/terminal suppressors,
    terminal failure, and G5 cleanup. Define witness runs for A/B/H, a Claude
    foreground scope unchanged, and all other harnesses unchanged. The Quint
    model remains responsible only for the mutation-protocol channel; it does
    not model `diff_traces` merely to express the direct-evidence distinction.
    Out — DB migration, store persistence, coordinator/ingress, and Claude
    adapter changes.
  - Dependencies: T01, with its decision gate recorded as proceed
  - Done when: the Quint safety invariants and witnesses pass; the model
    explicitly forbids generic `recover`/abandon/close from terminating a
    potentially-live managed background; the non-attributing terminal path is
    the only lease-consuming path; the latch fallback forces global
    `IneligibleUnscoped`; and the task stops for plan revision if any path can
    produce positive mutation-scope attribution or remove both protections.
  - Verify: `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
  - Context synchronization: pending

- [ ] T03: `Persist the protocol mode, lease, and safety latch` (status:todo)
  - Task ID: T03
  - Scope: In — refine T02 in `types.rs`, `protocol.rs`, `store.rs`, the MBT
    driver, and an additive `006` migration. Persist per-scope mode and
    potentially-live lease state, the worktree-keyed `background_safety_taint`
    reason/identity/proof state, guarded transitions, restart loading, and
    migration compatibility. Record older-binary behavior and make any
    schema-version mismatch fail closed before a boundary is accepted. Out —
    runtime ingress and Claude adapter behavior.
  - Dependencies: T02
  - Done when: all protocol fields round-trip through a real Agent Trace DB;
    migration/restart tests preserve live suppressors and safety latches;
    ordinary recover cannot clear the latch; only G5 proof plus deterministic
    cleanup can clear it; real Git/DB tests cover a missing/terminal scope;
    and the Rust refinement/MBT still matches the Quint model.
  - Verify: `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
  - Context synchronization: pending

- [ ] T04: `Carry the background-safe mode through runtime ingress` (status:todo)
  - Task ID: T04
  - Scope: In — `RuntimeBoundary::Start` and `coordinate()` register the
    requested managed-background mode; `hooks/mutation_scope.rs` accepts one
    optional `start` key, rejects it on every other operation, and forwards it
    verbatim. Add the guarded non-attributing termination payload/boundary.
    Out — any adapter sending the key.
  - Dependencies: T03
  - Done when: a `start` payload requesting the suppressor persists the mode;
    a payload without the key behaves exactly as today; a weaker replay is an
    identity conflict; termination requires its dedicated operation/proof;
    and exact per-operation key-set tests cover both additions.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope`
  - Context synchronization: pending

- [ ] T05: `Introduce version 2 Claude adapter state with version 1 migration` (status:todo)
  - Task ID: T05
  - Scope: In — `state.rs`: `ADAPTER_STATE_VERSION = 2`; a reader that accepts
    version 1 and version 2, maps every version-1 attempt to foreground, and
    rejects any other version; writes always version 2; phases for background
    awaiting identifier, background running with its identifier, background
    unresolved, and `pending_terminal_recovery`; the transitions between them
    and their invariants against ordinary abandonment; lookup by identifier.
    Out — wiring into `lifecycle.rs`;
    health.
  - Dependencies: T04
  - Done when: a file written by the current binary loads as all-foreground and
    is rewritten as version 2 on the next write; no code path can serialize a
    background phase under version 1; a test holding the version-1 parser rule
    shows a version-2 file is rejected; binding is idempotent for the same
    identifier and a different identifier moves the attempt to unresolved;
    every transition is one durable write under the adapter-state lock.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope::state`
  - Context synchronization: pending

- [ ] T06: `Parse Claude background lifecycle and completion evidence` (status:todo)
  - Task ID: T06
  - Scope: In — `events.rs`: carry the background identifier and
    `run_in_background` on the parsed `PostToolUse`/`PostToolUseFailure`; parse
    the terminal event T01 identified into a typed variant with session, `cwd`,
    optional `agent_id`, the joining identifier, and the outcome; unrelated
    events of that hook name parse to a no-op. Fixture-driven tests. Out —
    lifecycle behavior; the denial stays.
  - Dependencies: T05
  - Done when: each T01 fixture parses to the expected typed event; a
    `PostToolUse` without background evidence parses exactly as before; strict
    rejection of malformed required fields is preserved; dispatch treats the
    new variant as an empty-stdout no-op.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T07: `Implement the correlated managed-background lifecycle` (status:todo)
  - Task ID: T07
  - Scope: In — `lifecycle.rs`: background `PostToolUse` binds the identifier
    with no seam call; the terminal event looks the attempt up and applies the
    BG-D4 non-attributing terminal action for success, non-zero exit, and
    cancellation; a failed transition enters `pending_terminal_recovery` with
    the suppressor/latch retained; log events for identifier bound and
    background completed. Background attempts are created through the state API
    in tests because the denial still stands. Out — unresolved handling,
    cleanup sweeps, the barrier, removing the denial.
  - Dependencies: T06
  - Done when: tests prove background `PostToolUse` does not terminate; each
    outcome uses the non-attributing terminal transition and removes the
    attempt only after protocol cleanup; a duplicate terminal event is a
    no-op; an unknown identifier leaves the state file byte-identical; a
    separate invocation reading only the file correlates the terminal event;
    foreground tests pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T08: `Hold unresolved executions and establish the safety inhibitor` (status:todo)
  - Task ID: T08
  - Scope: In — `lifecycle.rs`: every BG-D2 entry condition first checks the
    protocol scope. A live suppressor produces Case A without `close` or
    `abandon`; a missing/terminal suppressor atomically establishes Case B's
    `background_safety_taint` before any other boundary; a `pending_start`
    replays idempotent `start` only if it can make the suppressor live;
    mutation-capable `PreToolUse` is denied in both cases; and logs identify
    unresolved versus safety-tainted recovery. Out — proven-boundary cleanup
    and recovery rules (T09); health (T10).
  - Dependencies: T07
  - Done when: tests prove the two cases are distinct; no code path can send
    `close`/`abandon` or flush past Case A; Case B is durable, global,
    restart-safe, and cannot be cleared by `repair_blocked`, the barrier, or
    ordinary recovery; and failure to establish the latch fails closed.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T09: `Apply proven-boundary cleanup and guarded recovery rules` (status:todo)
  - Task ID: T09
  - Scope: In — `lifecycle.rs`: `Stop`, `StopFailure`, `UserPromptSubmit`,
    `SubagentStop`, `SessionEnd`, and `WorktreeRemove` retire a running or
    unresolved background attempt only where T01 answered `yes`, through the
    non-attributing termination path; foreground sweeps stay unchanged. Apply
    the BG-D6 barrier, preserve live suppressors during taint/external-taint
    recovery, and make latch cleanup require persisted G5 evidence. Out —
    health classification; removing the denial.
  - Dependencies: T08
  - Done when: one test per event fixes its outcome for running and unresolved
    background attempts and the unchanged outcome for a foreground attempt;
    each BG-D6 row and recovery case has a test, including a real Git/DB test
    that flush/recovery under a live correlated background scope emits
    `IneligibleUnscoped`; missing-suppressor fallback remains latched; and
    terminal-transition failure remains fail-closed.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T10: `Report background attempts and safety taint in health and doctor` (status:todo)
  - Task ID: T10
  - Scope: In — `health.rs` `classify_health` and `assess_repairability`,
    `repair_blocked`, the shared `mutation_scope_health.rs` repairability fact
    for "blocked awaiting lifecycle evidence", and doctor text/JSON rendering
    and fixability mapping per BG-D7, with per-attempt detail. Out — other
    adapters' classifiers.
  - Dependencies: T09
  - Done when: the classification table test covers every `recovery_pending` ×
    phase shape and the durable safety-latch state; all six BG-D7 conditions
    render distinctly; safety-taint is explicitly not auto-fixable;
    `sce doctor --fix` leaves unresolved/running attempts and the latch
    untouched and re-proves its condition under the lock; and the Rust
    regressions tied to `spec/doctor_recovery.qnt` invariants pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope::health`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
  - Context synchronization: pending

- [ ] T11: `Gate background admission on a verified terminal-hook capability` (status:todo)
  - Task ID: T11
  - Scope: In — per BG-D5: the terminal-event registration and the
    registration-contract marker in `config/pkl/renderers/claude-content.pkl`,
    setup merge and doctor expectations for them, and an adapter function that
    returns capability verified or unavailable from the invocation marker plus
    the project settings check, with a stable deny reason. If G7 shows no new
    registration is needed, complete with that evidence and a capability that
    is always verified. If G7 shows hooks do not come from one snapshot, stop
    for a plan revision. Out — calling the gate from `PreToolUse` (T12).
  - Dependencies: T10
  - Done when: generated settings contain the registration and marker; setup's
    merge stays idempotent and preserves user-owned hooks; the capability
    function returns unavailable for settings generated before this change and
    verified after `sce setup`; an older binary invoked with the marker present
    behaves as before.
  - Verify: `nix run .#pkl-check-generated`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T12: `Admit capability-verified Claude background Bash instead of denying it` (status:todo)
  - Task ID: T12
  - Scope: In — `lifecycle.rs` / `events.rs` / `payload.rs`: background Bash
    `PreToolUse` checks the capability, then runs the barrier and a write-ahead
    `start` requesting `ManagedBackgroundSuppressor`, and persists a background
    attempt; the unconditional Bash denial is removed while `PowerShell` keeps
    it; BG-D8 handling if T01 captured mid-flight backgrounding; log event for
    background started; tests replaying T01 fixture sequences through the whole
    adapter. Out — real Git/DB attribution regressions (T13).
  - Dependencies: T09, T10, T11
  - Done when: with capability unavailable the call is denied with the
    capability reason; with it verified the call returns empty stdout and never
    `allow`; every failure in that path returns the fail-closed deny; the T01
    success, failure, and cancellation sequences each end with no attempt and
    `recovery_pending == false`, as does the sequence for
    `nix flake check 2>&1 | tail -40; echo "EXIT=${PIPESTATUS[0]}"`; foreground
    tests and the `probe17` test pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T13: `Prove conservative background safety against real Git and the Agent Trace DB` (status:todo)
  - Task ID: T13
  - Scope: In — regressions beside the existing Claude cases in
    `services::hooks::tests::mutation_provenance_e2e`, driven through
    `run_claude_mutation_scope_from_payload_at_state_root`: the A/B/H sequence
    from Design, mutation-scope checks for every A interval, B's surviving
    direct `diff_trace` and direct session/model/tool provenance, proof that A's
    mutation evidence does not supplement B, H's lack of direct evidence,
    terminal non-attributing cleanup, the missing/terminal-scope safety-latch
    scenario, all-harness boundaries, process restart, and post-cleanup
    ordinary attribution. Out — changes to protocol or lineage logic; a
    failure here stops the task for a plan revision.
  - Dependencies: T12
  - Done when: A, B, and H have `IneligibleUnscoped` mutation-protocol
    attribution while A may run; no mutation-AI line or mutation-lineage AI
    coverage is emitted for A; B's direct `diff_trace` remains AI-attributed
    with its direct provenance and is not supplemented by A; H has no direct
    evidence and is not AI-attributed; missing suppressor creates a durable
    latch that recovery cannot clear; G5 cleanup clears it; restart preserves
    it; and future ordinary foreground attribution is unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_provenance_e2e`
  - Context synchronization: pending

- [ ] T14: `Record live Claude acceptance evidence for background execution` (status:todo)
  - Task ID: T14
  - Scope: In — in a real Claude Code session with the built CLI and current
    generated settings, run the backgrounded
    `nix flake check 2>&1 | tail -40; echo "EXIT=${PIPESTATUS[0]}"` scenario and
    the `sleep 3` / `echo test >> <tracked-file>` scenario; store the captured
    payloads, adapter-state snapshots during and after, log events, and
    `mutation_trace_*` rows as fixtures with a `NOTES.md` section; update the
    background-execution context document to describe the supported behavior.
    Out — any source change; a failure is recorded and stops the task.
  - Dependencies: T13
  - Done when: both scenarios show no `PreToolUse` deny, a live suppressor while
    the command runs, the terminal event ending it through non-attributing
    cleanup, `"attempts": []`, and `"recovery_pending": false`; neither
    scenario emits `mutation_ai_patch`/AI lineage for the background scope; the
    missing-suppressor restart scenario and doctor output are recorded; and the
    test edit to the tracked file is reverted.
  - Verify: `nix run .#sce -- doctor --format json` reports Claude
    mutation-scope health `healthy`; `git status --short` shows only the new
    fixtures, `NOTES.md`, and the context document.
  - Context synchronization: pending

## Final safety statement

While SCE believes a Claude-managed background process may still mutate the
worktree, that execution cannot provide positive mutation-scope attribution.
A managed-background scope never produces `AiExclusive`,
`mutation_ai_patch`, or positive mutation-lineage coverage, including at
termination. Independently established direct tool evidence such as Claude
`Edit`/`Write` `diff_traces` remains authoritative and is unaffected. SCE
cannot return the mutation protocol to ordinary attribution until lifecycle
evidence proves the background process can no longer mutate and guarded
cleanup completes. The important property is:

```text
background execution cannot create false-positive AI attribution
```

while stronger independent direct evidence remains usable.

## Open questions

None. The questions the earlier draft left open are now decisions or gates:
the first implementation never positively mutation-scope attributes a managed
background;
BG-D2 Case A is unresolved with a live suppressor, Case B is the durable
worktree safety latch, recovery is guarded by the protocol invariant, stale
hook registration is the BG-D5 admission gate, and mid-flight backgrounding is
BG-D8. There is no accepted residual false-positive mutation-scope attribution
case. If T02
cannot prove the lease-or-latch invariant across all protocol transitions, the
plan stops before background admission and is revised.
