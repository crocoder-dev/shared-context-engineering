# T01 — Pi mutation lifecycle evidence

Frozen lifecycle evidence for the Pi mutation-scope integration
(`context/plans/pi-mutation-scope-integration.md`). Every load-bearing
assumption behind design decisions **D1–D14** carries a disposition here,
backed by a captured event sequence in `captures/` and/or a citation into the
pinned package's compiled source (`dist/`) and documentation (`docs/`). A
contradictory reading of this evidence is a re-planning gate for T02+, per the
plan's Design preamble and its own version-drift clause.

Dispositions use exactly four values:

- `PROVEN` — observed live in `captures/`.
- `PROVEN-BY-PINNED-SOURCE` — not driven live, but fixed unambiguously by the
  pinned package's compiled source or documentation.
- `DOCUMENTED — NON-LOAD-BEARING` — characterised, but no plan decision rests
  on the exact detail.
- `UNSUPPORTED` — the plan's stated assumption does not hold on the pinned
  version, as literally written.

## Pinned versions (evidence is version-bound)

| Component | Version | Provenance |
|---|---|---|
| `@earendil-works/pi-coding-agent` | **0.80.6** | repo-pinned value in `config/lib/package.json`; confirmed live via `node dist/cli.js --version` |
| Upstream repository | `github.com/earendil-works/pi`, package directory `packages/coding-agent` | `package.json` `repository` field |
| Upstream tag (per plan) | `v0.80.6`, commit `2b3fda9921b5590f285165287bd442a25817f17b` | plan's **Dependency and version policy** |
| Node runtime | `nodejs-24.16.0` (nixpkgs) | invoked via `nix run nixpkgs#nodejs -- <pi>/dist/cli.js` |
| OS | Linux 6.18.37 `x86_64`, NixOS | — |

This machine has no outbound network access to `github.com` from the sandboxed
probe shell, so the upstream Git tag could not be cloned for line-numbered
citations. All source citations below are therefore against the pinned
package's own compiled `dist/*.js` (built from that exact tag per the
package's own build pipeline — no transformation beyond `tsgo` compilation)
and its shipped `docs/*.md`, which describe the same pinned release. This is
the same "pinned build is the source of truth" posture the OpenCode T01 used
for its own citations, one level more direct here because the npm package
ships both compiled source and docs together.

## Probe environment and method

- **Probe repo:** a throwaway `git init` repo (`<PROBE_REPO_ROOT>/repo` in the
  captures), never this SCE checkout.
- **Isolation:** `HOME` was redirected to a scratch `<PROBE_REPO_ROOT>/fakehome`
  for every probe invocation. Only `~/.pi/agent/auth.json` (real provider
  credentials) and `~/.pi/agent/models-store.json` were copied in; no other
  operator state was touched. Verified before and after every batch of runs
  that the operator's real `~/.pi/agent/sessions/` entry count (226) never
  changed, and that the fake-home run created its own
  `fakehome/.pi/agent/sessions/<encoded-cwd>/` tree, confirming Pi's session
  storage is `HOME`-resolved and therefore fully isolable this way (see D11).
- **Invocation:** the pinned binary is `dist/cli.js`; it was always run as
  `nix run nixpkgs#nodejs -- <pinned>/dist/cli.js` (this repo's Bash-tool
  policy requires running `node` through `nix`). One-shot driving via
  `pi -p "<prompt>"`; `--no-session` for throwaway runs, a real session only
  for the D11 session-id/session-file probe. `--tools <allowlist>` constrained
  the tool surface for deterministic probes.
- **Model:** the operator's authenticated default, `openai-codex` /
  `gpt-5.5`/`gpt-5.6-luna` (ChatGPT-backend Codex responses API). It reliably
  drove `bash`, `write`, and `edit` tool calls from a direct natural-language
  instruction, including "call the tool exactly once, do not retry" framing
  needed to keep blocked/failed-tool probes deterministic (this model retries
  tool calls under some failure framings if not told not to).
- **Instrumentation:** `probe-plugins/capture.ts`, loaded via `-e`, subscribes
  every mutation-relevant `pi.on(...)` event and appends one JSON line per
  event to `$PI_PROBE_LOG`, stamped with `mono_us` (`process.hrtime.bigint()`),
  wallclock, `pid`, a per-process `seq`, and the live `ctx.model`. Extensions
  are typed against `ExtensionAPI`; event names not present in this pinned
  version's public event union are cast through `as any` in the probe only
  (the production T03 adapter will use the real typed union).
- **Fault injection:** `capture.ts` throws or blocks in `tool_call` when
  `PI_PROBE_FAULT=throw|block`. `probe-plugins/order-first.ts` /
  `order-last.ts` / `order-last-fault.ts` bracket `capture.ts` in the `-e`
  list to observe multi-extension `tool_call` ordering and fail-closed
  barriers (Probes B/C).
- **Custom tool probe:** `probe-plugins/customtool.ts` registers a
  filesystem-mutating custom tool `probe_mutate` (D2 classification), loaded
  with `NODE_PATH` pointed at `config/lib/node_modules` so its `typebox`
  import resolves without vendoring a copy into the probe tree.
- **Session id probe:** `probe-plugins/sessioninfo.ts` reads
  `ctx.sessionManager.getSessionId()` / `getSessionFile()` on `session_start`.
- `<PROBE_REPO_ROOT>` replaces the absolute scratch path in every committed
  capture.

## Tool vocabulary at 0.80.6 (D2 substrate)

`dist/core/tools/index.js`:

```js
export const allToolNames = new Set(["read", "bash", "edit", "write", "grep", "find", "ls"]);
```

Exactly seven built-ins, unconditionally registered (no model-id-gated
alternate tool set like OpenCode's `apply_patch`/`edit` split — `createCodingTools`
always returns `{read, bash, edit, write}`, `createReadOnlyTools` always
`{read, grep, find, ls}`). This is a strictly simpler substrate than the
OpenCode precedent: **every session sees the same maximal tracked set**
`{bash, edit, write}`, with no per-model exclusivity gate to account for.

---

## D1 — One tool execution is one scope

**Disposition: PROVEN.**

`tool_call` / `tool_execution_start` / `tool_execution_end` / `tool_result`
inputs all carry `toolCallId` + `toolName` (`docs/extensions.md` event
signatures; live in every capture below).

Observed identifier shape (`captures/bash-success.jsonl`):

```
call_nHNT6BxYPqG8Cau0y6adOtMI|fc_01db62cb980e99c0016aa453e0c87c87d2b74d28a903bb7046
```

i.e. `call_<24 base62>|fc_<52 hex>` — a composite of the AI-SDK-level tool-call
id and the provider (`openai-codex-responses`) function-call id, joined by
`|`. Opaque, stable across one invocation's full
`tool_execution_start → tool_call → tool_result → tool_execution_end` bracket
(byte-identical in every capture), and distinct per concurrent call
(`captures/parallel.jsonl`: `call_lsPkqWP10EsCzDm...` and
`call_dDfkKrO7xudGotf...` overlap, see D9-equivalent below). No `sessionID` is
embedded in `toolCallId` itself, but `ctx.sessionManager.getSessionId()`
(D11) is available in the same handler invocation to pair with it.

**Freeze:** `ScopeId` identity is `(sessionId, toolCallId)`, exactly as the
plan's canonical identity template assumes (`s=<len>:<session-id>|c=<len>:<tool-call-id>`).
The literal `toolCallId` string may itself contain `|`, so the plan's
length-prefixed encoding (not a raw delimiter join) is necessary and already
anticipates this — confirmed necessary, not merely convenient, by this exact
observed shape.

---

## D2 — Conservative tool classification

**Disposition: PROVEN.**

- `allToolNames` (above) is a closed, version-pinned set. `bash`/`edit`/`write`
  are the only mutation-capable built-ins; `read`/`grep`/`find`/`ls` are
  read-only, per `dist/core/tools/index.js`'s own
  `createCodingTools` (mutation-capable) vs `createReadOnlyTools` split.
- **Every built-in, including read-only ones, fires the identical
  `tool_execution_start → tool_call → tool_result → tool_execution_end`
  bracket.** `captures/readonly-footprint.jsonl`: `ls`, `grep`, `find`, `read`
  each get the full bracket, none mutate.
- **A custom (plugin-registered) tool fires the same bracket and can mutate.**
  `captures/customtool.jsonl`: `probe_mutate` (via `pi.registerTool`) gets the
  full bracket and its `execute` wrote `ct.txt` to the probe repo.

**Therefore hook presence is not a mutation signal**, exactly as D2 assumes.
Classification must be a closed allowlist keyed on the exact `toolName`
string:

```
bash | edit | write        -> TrackedMutation
read | grep | find | ls    -> Untracked (known read-only)
everything else (custom/plugin tools, future built-ins) -> Untracked (unknown)
```

No intended tracked tool fails the soundness contract. Custom-tool mutation
with no scope (`ct.txt` written, zero footprint) is the intended, safe
`Untracked` outcome — covered by the conservative unscoped fallback, not by
false AI attribution.

**Built-in-name replacement:** `pi.registerTool({ name: "bash", ... })` was
not attempted live (out of T01's time budget), but `docs/extensions.md`
"Overriding Built-in Tools" documents this as a supported, sanctioned
extension capability. **Disposition: DOCUMENTED — NON-LOAD-BEARING for T01,
carried forward as an explicit open risk for T03**: if a project extension
overrides `bash`/`edit`/`write` with different mutation semantics, the
adapter's classification-by-name allowlist would still admit it as tracked
(matching plan D2's stated tolerance: classification is by name, not by
introspecting behavior), so this does not invalidate D2, but T03+ should not
assume the built-in tool's *documented* semantics (e.g. exact `bash` timeout
handling) hold if a project has silently replaced it.

---

## D3 — Start is write-ahead and fail-closed

**Disposition: PROVEN — with a load-bearing ordering correction, see D5.**

`tool_call` is documented as "Fired after `tool_execution_start`, before the
tool executes. **Can block.**" (`docs/extensions.md`). Live proof of the
fail-closed contract:

- **Probe A — block:** `captures/probeA-block.jsonl`. `capture.ts` returns
  `{ block: true, reason }` from `tool_call`. Result: no `tool_result` event,
  `tool_execution_end` still fires but with `isError: true` and
  `result.content` carrying the block reason. The target file
  (`probeA.txt`) was **never created**.
- **Probe A — throw:** `captures/probeA-throw.jsonl`. `capture.ts` throws
  synchronously from `tool_call`. Identical outcome: no `tool_result`,
  `tool_execution_end` fires with `isError: true` and the thrown message as
  content, target file (`probeA2.txt`) **never created**.
- Source: `dist/core/agent-session.js` `_installAgentToolHooks()` wires
  `agent.beforeToolCall` directly to the extension runner's `tool_call`
  emission; a thrown/rejected handler or a `{ block: true }` return is caught
  and converted into an error that the underlying `Agent` treats as the tool
  call's own failure, so `item.execute` (the tool's real body) is never
  invoked in either case. This is a single, uniform code path — a plugin
  `throw` and a plugin `return { block: true }` are handled identically from
  the tool's perspective.

**Freeze:** `tool_call` is a genuine, synchronous, fail-closed pre-execution
gate for every tool. An SCE extension's `tool_call` handler that throws or
returns `block: true` before mutation-scope admission succeeds prevents the
tool from ever running, with zero filesystem side effect — matches D3
exactly. See D5 for why `tool_execution_start` is *not* usable as this gate.

---

## D4 — (Not applicable as a separate design point for Pi)

Pi has no bash-specific pre-spawn hook analogous to OpenCode's `shell.env` —
`tool_call` is the single universal pre-execution boundary for every tool
including `bash` (`docs/extensions.md`'s `tool_call` example uses `bash` as
its primary illustration, mutating `event.input.command` in place). The
plan's D3 (not a separate D4) already reflects this by putting "bash policy"
and "mutation-scope Start" as two handlers on the same `tool_call` gate, in
extension-array order, rather than needing a second Pi-specific hook the way
OpenCode's Bash tool needed `shell.env` in addition to `tool.execute.before`.
**Disposition: DOCUMENTED — NON-LOAD-BEARING** (informational: the plan
already anticipated the simpler Pi shape).

---

## D5 — Execution start is lifecycle evidence, not a mutation boundary

**Disposition: UNSUPPORTED, as literally stated. Load-bearing correction —
see below for the safe substitute.**

The plan's assumed ordering is:

```
PendingStart
    ↓ generic Start (tool_call) succeeds
AwaitingExecution
    ↓ tool_execution_start
Active
```

i.e. `tool_execution_start` was assumed to fire *after* a successful `tool_call`,
proving the tool actually began running. **This is backwards on 0.80.6.**

Documented order (`docs/extensions.md`, Lifecycle Overview, and the `tool_call`
section verbatim: "Fired **after** `tool_execution_start`, before the tool
executes"):

```
tool_execution_start   (unconditional, fires for every registered extension)
tool_call               (can block — the actual gate)
tool_execution_update
tool_result             (only if the tool actually ran)
tool_execution_end
```

Live proof, three ways:

1. **Single-extension bash success** (`captures/bash-success.jsonl`):
   `tool_execution_start` (seq 5) precedes `tool_call` (seq 6) by ~1.3ms, for
   the same `toolCallId`.
2. **Multi-extension ordering, Probe C** (`captures/probeC-order-throw.jsonl`,
   extensions loaded as `order-first, capture, order-last`): **all three**
   extensions' `tool_execution_start` handlers fire, in array order, *before
   any* `tool_call` handler runs at all. Only then does `order-first`'s
   `tool_call` handler fire and throw; `capture`'s and `order-last`'s
   `tool_call` handlers are never reached, and the tool never executes.
3. **Source** — `dist/core/agent-session.js`: `tool_execution_start` /
   `tool_execution_update` / `tool_execution_end` are forwarded verbatim via
   `await this._extensionRunner.emit(extensionEvent)` — a fire-and-forget,
   non-blocking broadcast from the underlying low-level `Agent`'s own event
   stream — entirely separate from `agent.beforeToolCall`/`agent.afterToolCall`,
   which are the only two hooks wired to `tool_call`/`tool_result` and the
   only two whose return value can affect execution. `tool_execution_start` is
   therefore emitted unconditionally as soon as the underlying agent loop
   schedules a tool call, **before** the extension-gated `tool_call` check
   that decides whether it will actually run.

**Consequence:** `tool_execution_start` carries **zero evidentiary value** for
"the tool actually began executing" — it fires identically whether the call
is later allowed, blocked, or thrown on. The plan's `AwaitingExecution` state,
keyed on `tool_execution_start`, cannot do the job D5 assigns it (distinguishing
"Start admitted, tool never executed" from "tool execution actually began").

**Safe substitute (verified below, D6):** `tool_result` — not
`tool_execution_start` and not raw `tool_execution_end` — is the signal that
proves the tool's `execute()` body actually ran. It fires in every capture
where the tool actually executed (success, non-zero exit, custom-tool
mutation) and in **none** of the block/throw captures. This preserves the
design intent behind D5 (distinguish "never ran" from "ran") with a different,
already-available signal, and does not require weakening the soundness
contract — see the Disposition summary's action item.

**This finding is exactly the kind of thing T01 exists to catch** (the plan's
own "T01 is a re-planning gate" clause for ordering divergence from D1–D14).
It requires a design-level correction before T03 encodes any adapter state
machine on the wrong event, but the correction is mechanical (swap
`tool_execution_start` for `tool_result`-gated logic; `tool_execution_start`
becomes pure informational telemetry, useful only for e.g. progress UI, never
for scope-state transitions) and does not touch the soundness properties D3,
D6, or D7 rely on — those remain intact once re-keyed to `tool_result`.

---

## D6 — `tool_execution_end` is the candidate confirming Close

**Disposition: UNSUPPORTED as a standalone signal; PROVEN once gated on
`tool_result`.**

The plan assumed `tool_execution_end` fires exactly for
`{success, isError}` outcomes of an execution that actually happened. Live
evidence shows **`tool_execution_end` fires unconditionally for every
`tool_call`, including one blocked or thrown on before execution**:

| Scenario | `tool_result`? | `tool_execution_end`? | `tool_execution_end.isError` | Capture |
|---|---|---|---|---|
| bash success | yes | yes | `false` | `bash-success.jsonl` |
| bash non-zero exit | yes | yes | `true` (exit code is data) | `bash-nonzero.jsonl` |
| write success | yes | yes | `false` | `write-success.jsonl` |
| edit success (2 ops) | yes (×2) | yes (×2) | `false` | `edit-success.jsonl` |
| custom tool mutation | yes | yes | `false` | `customtool.jsonl` |
| `tool_call` blocked | **no** | yes | `true` (block reason as content) | `probeA-block.jsonl` |
| `tool_call` throws | **no** | yes | `true` (thrown message as content) | `probeA-throw.jsonl` |
| later extension blocks after an earlier one admits (Probe B) | **no** | yes | `true` | `probeB-later-block.jsonl` |

**`tool_result` is present if and only if the tool's `execute()` body actually
ran** (fires for success and for a genuine runtime failure alike — bash exit
7 still fires `tool_result` with `isError: true` and the partial write
persists, matching the plan's "isError must not discard the observation"
requirement) **and is absent whenever `tool_call` prevented execution.**
`tool_execution_end` alone cannot make this distinction; it must be paired
with "did a `tool_result` for this `toolCallId` precede it."

**Freeze (revised from the plan's literal D6):** the confirming Close signal
is `tool_result` (equivalently: `tool_execution_end` *conditioned on* having
observed `tool_result` first for the same `toolCallId` — the adapter may use
either as the trigger as long as it never treats a `tool_execution_end` with
no preceding `tool_result` as a Close). Both success (`isError:false`) and
failed-but-executed (`isError:true`) map to the same Close boundary, exactly
as D6 intends — the correction is only about which raw event proves
"execution happened," not about the success/failure treatment.

No two boundaries are produced per execution: exactly one `tool_result` +
one `tool_execution_end` pair per `toolCallId`, even for a blocked call
(where only `tool_execution_end` appears).

---

## D7 — A Start followed by no execution must be abandoned, never closed

**Disposition: PROVEN, and the correct terminal signal is now precisely
identified (not left open as the plan anticipated).**

The plan explicitly left open "the exact signal proving that an
`AwaitingExecution` attempt can no longer execute," candidate-listing "the
actual blocked-tool result sequence" and `agent_settled`. T01 resolves this:
**the exact signal is the arrival of `tool_execution_end` with no preceding
`tool_result` for that `toolCallId`.** This is not a heuristic or a broad
lifecycle event — it is the same synchronous per-call event pair examined in
D6, deterministically distinguishing "admitted, never executed" (`probeA-*`,
`probeB-later-block`) from "executed" (every success/failure capture). No
reliance on `agent_settled` or any session-wide event is needed for this
specific determination.

**Freeze:** on `tool_execution_end` for an attempt with no observed
`tool_result`, the adapter must abandon that scope (never close it), exactly
per D7's flush/abandon/rebaseline requirement — using the pairing established
here, not an inferred timeout or a broad session-level event.

---

## D8 — Recovery follows the soundness-first flush/abandon/flush pattern

**Disposition: DOCUMENTED — NON-LOAD-BEARING for T01.** This is a T04 adapter
design obligation, not a Pi-lifecycle fact. Nothing observed here contradicts
its feasibility: Probe B/Probe C both prove multiple extensions and multiple
overlapping scopes are independently observable per `toolCallId` (D1, D9-
equivalent below), which is what a flush/abandon/rebaseline sequence needs to
target the correct scope without disturbing siblings.

---

## D9 — Transport failure after tool execution cannot be repaired by pretending the observation is current

**Disposition: DOCUMENTED — NON-LOAD-BEARING for T01.** A T05
(TypeScript-extension) and T04 (Rust adapter) design obligation. The relevant
Pi-side fact — that `tool_result`/`tool_execution_end` fire exactly once,
synchronously, per real execution, with no re-delivery mechanism observed —
is already established in D6 and supports the "no replay" requirement, but
inventing an actual transport failure between the TS extension and the Rust
adapter is outside what a Pi-lifecycle probe can exercise.

---

## D10 — Process death is positive staleness evidence; elapsed time is not

**Disposition: DOCUMENTED — NON-LOAD-BEARING for T01, with one supporting
live fact.** `captures/sigint.jsonl`: a SIGINT delivered to the underlying
`sh -c 'sleep 20; ...'` child's process group while a `bash` tool call was
in flight killed the **pi/node process itself** (it disappeared from `ps`
immediately), while the spawned `sh`/`sleep` descendants **kept running
and completed their mutation (`sig.txt` written) after `pi` was already
dead** — the same "orphan survives parent death" shape the OpenCode T01
found. This directly supports D10's premise that a durable attempt can
outlive its owning Pi process with no terminal event ever arriving, and that
no timeout can distinguish "orphan still mutating" from "cleanly abandoned."
Exact cross-platform process-instance identity (PID-reuse-safe ownership
proof) was not probed — that is a T04 implementation detail, not a Pi
lifecycle fact, and the plan already treats it as an open implementation
question ("the implementation should use the strongest process-instance
evidence available").

---

## D11 — Pi session/model provenance is admission-time metadata

**Disposition: PROVEN, and simpler than the plan's OpenCode-derived
assumption suggested.**

- **Session id:** `ctx.sessionManager.getSessionId()` returns a UUIDv7, e.g.
  `01a091f4-3d57-7132-91a9-57218a3564f1` (`captures/sessioninfo.jsonl`).
  Canonical `pi_<sessionId>` prefixing per D11 applies cleanly; no observed
  characters would need escaping.
- **Session storage is checkout-scoped, not global-user-scoped** (unlike
  OpenCode): `ctx.sessionManager.getSessionFile()` resolves under
  `~/.pi/agent/sessions/<url-safe-encoded-cwd>/<timestamp>_<sessionId>.jsonl`
  — one directory per project working directory, encoded from the cwd path
  itself (`--tmp-...-pi-t01-probe-repo--` for
  `/tmp/.../pi-t01-probe/repo`). This directly supports D12 (multiple Pi
  processes on one checkout share the same session-directory namespace with
  no cross-checkout leakage) and made the isolation strategy above
  straightforward (`HOME` redirection alone fully isolates a probe run from
  the operator's real Pi state — confirmed: the operator's real
  `~/.pi/agent/sessions/` entry count was unchanged, 226, before and after
  every probe batch).
- **Model provenance is simpler than OpenCode's `chat.params`-tracking
  design:** every extension event handler receives `ctx.model` **directly**
  (`{ provider, id }`, e.g. `{"provider":"openai-codex","id":"gpt-5.5"}` —
  observed identically on `session_start`, `tool_call`, `tool_execution_end`,
  etc. in every capture). There is no need for an adapter-side
  session-id-to-model map built from a separate pre-turn event — `ctx.model`
  at the exact moment of `tool_call` **is** the admission-time model
  observation the plan wants, with no risk of it lagging behind a `chat.params`
  race. A session with no resolvable model would presumably have failed
  before any `tool_call` could fire at all (a `tool_call` implies a
  successful LLM response was already parsed into a tool-call message), so
  "missing model at `tool_call`" is expected to be unreachable in practice
  rather than a real per-call `NULL` case — this was not falsified live but
  follows directly from `ctx.model`'s presence in the `ExtensionContext`
  contract (`docs/extensions.md` `### ctx.modelRegistry / ctx.model`).
  `model_select` (fires on `/model`, cycling, or session restore) is the only
  documented way the active model changes mid-session; each subsequent
  `tool_call`'s own `ctx.model` reflects the change automatically, so no
  explicit switch-tracking is needed.

**Freeze:** `provenance.session_id = pi_<uuid>` from
`ctx.sessionManager.getSessionId()`; `provenance.model_id =
normalized(ctx.model.provider + "/" + ctx.model.id)` read directly at the
`tool_call` handler invocation, else `NULL` if `ctx.model` is ever absent
(not observed, but the plan's "never guess" rule applies unconditionally).

---

## D12 — Multi-process Pi is normal concurrency

**Disposition: PROVEN (single-process parallelism) / PROVEN-BY-PINNED-SOURCE
(cross-process).**

`captures/parallel.jsonl` (one assistant turn issuing two `bash` calls
explicitly in parallel): both calls' `tool_execution_start`/`tool_call` fire
with distinct `toolCallId`s while the first (3s sleep) is still in flight
when the second (1s sleep) starts and finishes first — genuine overlap,
independently identified, no forced serialization, no accidental scope
collapse. This proves the single-process half of D12 (D1's identity scheme
is sufficient for real overlap).

Two genuinely separate Pi **processes** on one checkout were not driven live
(out of T01's time budget; would need two full agent turns run concurrently
under the same isolated `HOME`/cwd). This is supported by source instead:
D11 already establishes that Pi session storage is a per-session file inside
a per-cwd directory (`<encoded-cwd>/<timestamp>_<uuid>.jsonl>`), with the
session id itself (a UUIDv7) as the sole per-session key — nothing in the
observed session/tool-call identity scheme is process-global or requires a
single writer. Two processes in the same checkout would each get their own
session file and their own `toolCallId` namespace (each `toolCallId` is
already provider/call-specific, not checkout- or process-derived), so nothing
in D1's `(sessionId, toolCallId)` identity scheme could collide across
processes. **Disposition for the cross-process half specifically:
PROVEN-BY-PINNED-SOURCE**, not live-witnessed.

---

## D13 — `!` / `!!` user Bash is not AI attribution

**Disposition: PROVEN-BY-PINNED-SOURCE — and this triggers the plan's own
stop condition. Flagged as a required T02+ design item, not a whole-plan
re-planning gate.**

`user_bash` (`!`/`!!`) is a TUI-only, keystroke-driven feature
(`dist/modes/interactive/interactive-mode.js`) with no reachable path from
`-p`/one-shot mode, so it could not be exercised through this probe harness's
non-interactive driving method within T01's time budget. Source inspection is
unambiguous and directly answers the plan's explicit open question ("T01 must
explicitly determine whether `user_bash` can execute concurrently with an
active agent tool"):

```js
// interactive-mode.js, handleBashCommand()
const isDeferred = this.session.isStreaming;
this.bashComponent = new BashExecutionComponent(command, this.ui, excludeFromContext);
if (isDeferred) {
    // Show in pending area when agent is streaming
    this.pendingMessagesContainer.addChild(this.bashComponent);
    ...
} else {
    this.chatContainer.addChild(this.bashComponent);
}
...
const result = await this.session.executeBash(command, ...);
```

`this.session.isStreaming` (i.e., an agent turn, and therefore any in-flight
tool call, is active) affects **only where the bash output is displayed** —
`pendingMessagesContainer` (deferred visual placement) vs immediate chat
placement. **`session.executeBash(command, ...)` is called unconditionally,
regardless of `isStreaming`.** The only guard that prevents launching a user
bash command is `session.isBashRunning` (a second `!` while one user bash is
already running), which has nothing to do with agent-tool activity. **`!`/`!!`
user Bash can therefore execute concurrently with an active agent
`bash`/`edit`/`write` tool call** on 0.80.6.

Per the plan's own text: *"If it can, the plan must stop and add a sound
explicit unscoped/taint boundary before T02."* This condition is met. This is
reported here as the required action for T02, not attempted in T01 (T01 is
evidence-only): T02+ must ensure a `user_bash` mutation occurring while a
tracked Pi scope is live is never attributable to that scope merely because
it overlapped in time — e.g. by having the TS extension's `user_bash` handler
explicitly notify the Rust adapter (a taint/fence boundary), or by relying on
the existing unscoped-interval fallback plus verifying no code path lets a
`user_bash`-caused mutation land inside an *open* AI-scope's confirmed
interval. This does not invalidate D1–D12; it adds one required new
integration point.

---

## D14 — Detached descendants remain an explicit limitation

**Disposition: PROVEN.**

`captures/bash-detached.jsonl`: `nohup sh -c 'sleep 5; echo done > detached.txt' >/dev/null 2>&1 &`
inside one `bash` tool call. `tool_execution_end` fires immediately (the
foreground `bash` tool call returns once the backgrounding shell built-in
returns), but `detached.txt` does not exist yet at that point and only
appears ~5s later, well after `tool_execution_end`/Close and after
`session_shutdown`. Exactly the plan's documented limitation: `tool_execution_end`
(paired with `tool_result`, per D6) proves the foreground tool call finished,
never that its full process tree stopped mutating. No shell-parsing or
static background-process detection was attempted, per the plan's explicit
non-goal.

---

## Fail-closed execution-barrier probes

### Probe A — `tool_call` failure (block and throw) · PROVEN

See D3/D6 above. Both `{ block: true }` and a synchronous `throw` in
`tool_call` produce: no `tool_result`, `tool_execution_end` with
`isError: true` and the block/throw reason as content, and **zero filesystem
side effect** (`probeA.txt` / `probeA2.txt` never created).
Captures: `probeA-block.jsonl`, `probeA-throw.jsonl`.

### Probe B — later-extension rejection after an earlier extension's silent admission · PROVEN

`captures/probeB-later-block.jsonl` (extensions loaded `capture, order-last-fault`,
a `bash` call): `capture`'s `tool_call` handler runs first and returns
`undefined` (silent admission — the shape an SCE mutation-scope Start success
would have), then `order-last-fault`'s `tool_call` handler runs and returns
`{ block: true }`. Result: `probeB.txt` **never created**, no `tool_result`
observed by `capture`, `tool_execution_end` fires with `isError: true`.
**Proves D4's premise directly: a successful (non-blocking) `tool_call`
handler from an earlier extension does not itself prove the tool will
execute — a later extension can still reject it after that point.** This is
exactly why D4 (Pi becomes confirmation-required) is necessary, and confirms
the mechanism is real on this pinned version, not merely theoretical.

### Probe C — earlier-extension synchronous throw blocks every later extension and the tool · PROVEN

`captures/probeC-order-throw.jsonl` (extensions loaded
`order-first, capture, order-last`, a `bash` call, `order-first` configured
to throw in `tool_call`): all three extensions' `tool_execution_start`
handlers fire (array order) — see D5 — then only `order-first`'s `tool_call`
handler fires and throws; `capture`'s and `order-last`'s `tool_call` handlers
are **never reached**, and the tool never executes (`probeC.txt` not
created). **Proves the plan's required handler ordering is enforceable**:
placing SCE bash-policy before the mutation-scope extension in the `-e`/
extension-array order means a bash-policy rejection prevents the
mutation-scope extension's `tool_call` handler from running at all — creating
no scope, exactly per D3's requirement — and this is a hard array-order
guarantee, not a race.

---

## Disposition summary

| # | Decision | Disposition | Primary evidence |
|---|---|---|---|
| D1 | Scope identity = one tool execution `(sessionId, toolCallId)` | **PROVEN** | `bash-success`, `parallel`; `docs/extensions.md`, `agent-session.js` |
| D2 | Explicit tool-name allowlist; `Untracked` ≠ read-only | **PROVEN** | `customtool`, `readonly-footprint`; `core/tools/index.js` |
| D3 | `tool_call` is a synchronous fail-closed pre-execution gate | **PROVEN** | `probeA-block`, `probeA-throw`; `agent-session.js` `_installAgentToolHooks` |
| D4 | (folded into D3 for Pi — no separate pre-spawn hook) | **DOCUMENTED — NON-LOAD-BEARING** | `docs/extensions.md` `tool_call` |
| D5 | `tool_execution_start` proves execution began, after a successful Start | **UNSUPPORTED as stated** — fires unconditionally *before* `tool_call`, for every extension | `bash-success`, `probeC-order-throw`; `agent-session.js` |
| D6 | `tool_execution_end` is the confirming Close for success/isError | **UNSUPPORTED standalone; PROVEN once gated on `tool_result`** | `probeA-*`, `probeB-later-block`, `bash-nonzero` |
| D7 | Start-without-execution must be abandoned, terminal signal owned by T01 | **PROVEN — signal identified: `tool_execution_end` with no preceding `tool_result`** | `probeA-*`, `probeB-later-block` |
| D8 | Recovery flush/abandon/flush pattern | **DOCUMENTED — NON-LOAD-BEARING (T04)** | `probeB`, `probeC` (multi-scope isolation feasibility) |
| D9 | No replay of a lost terminal boundary | **DOCUMENTED — NON-LOAD-BEARING (T04/T05)** | D6 (single-fire guarantee) |
| D10 | Process death is positive staleness evidence; no TTL | **PROVEN (orphan-survives-parent fact); DOCUMENTED for PID-reuse mechanics** | `sigint.jsonl` |
| D11 | Session id = `pi_<uuid>`; model observed via live `ctx.model`, else `NULL` | **PROVEN** | `sessioninfo.jsonl`; `docs/extensions.md` `ctx.model` |
| D12 | Legitimate parallelism preserved, single- and cross-process | **PROVEN (single-process); PROVEN-BY-PINNED-SOURCE (cross-process)** | `parallel.jsonl`; session-file-per-cwd scheme |
| D13 | `user_bash` never creates an `ActorKind::Pi` scope; overlap must be checked | **PROVEN-BY-PINNED-SOURCE — overlap IS possible, plan's stop condition triggered** | `interactive-mode.js` `handleBashCommand` |
| D14 | Detached descendants: explicit, documented limitation | **PROVEN** | `bash-detached.jsonl` |
| Probe A | `tool_call` block/throw is fail-closed, zero side effect | **PROVEN** | `probeA-block`, `probeA-throw` |
| Probe B | Later-extension rejection after earlier silent admission blocks execution | **PROVEN** | `probeB-later-block` |
| Probe C | Earlier-extension throw blocks later extensions + the tool | **PROVEN** | `probeC-order-throw` |

**Re-planning-gate assessment.** Per the plan's stop conditions
("`tool_call` cannot reliably block before mutation execution," "no sound
confirming post-execution boundary exists," "later-extension rejection
invalidates the confirmation-required design," "Pi user Bash can overlap AI
execution in a way the current protocol cannot soundly distinguish," or
"process/recovery semantics cannot conservatively preserve false-positive
safety") — **none of these hold as stated**: `tool_call` blocks reliably
(Probes A/B/C), a sound confirming boundary exists (`tool_result`, once D5/D6
are corrected as above), later-extension rejection is exactly what motivates
and validates the confirmation-required design (D4/Probe B), and
process/recovery semantics remain conservative (D7's signal is now exact,
D10's orphan-survival fact is accounted for by staying confirmation-required).

**However, two findings are load-bearing corrections that T02+ must adopt
before implementation, not treat as already-settled by the plan text as
written:**

1. **D5/D6 correction (mechanical, does not weaken soundness):** the adapter's
   `PendingStart → AwaitingExecution → Active` bookkeeping must key
   `AwaitingExecution → Active` (and D7's abandon decision) on **`tool_result`**
   arriving for the `toolCallId`, never on `tool_execution_start`, which
   carries no evidentiary value on this pinned version. `tool_execution_end`
   is safe to treat as Close **only** when a `tool_result` for the same
   `toolCallId` was already observed; a `tool_execution_end` with none is an
   abandon signal, not a Close.
2. **D13 requires an explicit new T02+ design item**, not present in the
   plan's current task bodies: a sound way to ensure a `user_bash` mutation
   that overlaps a live, unconfirmed Pi AI scope is never later folded into
   that scope's positive attribution once it confirms. This is additive (a
   new required correctness property to design and test in T02–T04), not a
   contradiction of anything already scoped — but it is not yet written down
   as a task deliverable anywhere in T02–T06's "Done when" bullets, and
   should be before those tasks are treated as complete.

Neither finding requires broadening positive attribution, weakening the
soundness contract, or abandoning the overall design — both are refinements
discovered by doing exactly what T01 was scoped to do. Whether this rises to
a formal plan revision before T02 begins, versus folding the corrections into
T02/T04's existing scope language, is a decision for whoever reviews this
task's completion, per the plan's stop-condition text ("any such finding
requires revising this plan rather than weakening attribution").

## Capture index

| File | Scenario | Key result |
|---|---|---|
| `captures/bash-success.jsonl` | `printf > file` via `bash` | `tool_execution_start → tool_call → tool_result → tool_execution_end`, ~1.3ms Start-to-gate |
| `captures/bash-nonzero.jsonl` | `sh -c '...; exit 7'` | `tool_result`/`tool_execution_end` still fire, `isError:true`; partial write persists |
| `captures/bash-detached.jsonl` | `nohup sh -c 'sleep 5; echo done' &` | Close fires before the descendant's mutation; descendant survives session shutdown |
| `captures/write-success.jsonl` | `write` new file | Same bracket shape as `bash` |
| `captures/edit-success.jsonl` | `write` then `edit` | Two independent brackets, two distinct `toolCallId`s |
| `captures/readonly-footprint.jsonl` | `ls`, `grep`, `find`, `read` | Identical bracket shape to mutating tools; zero mutation |
| `captures/customtool.jsonl` | plugin tool `probe_mutate` mutates a file | Identical bracket shape; must be `Untracked` per D2 |
| `captures/parallel.jsonl` | one turn, two `bash` calls forced parallel | Overlapping live scopes, distinct `toolCallId`s, out-of-order Close |
| `captures/probeA-block.jsonl` | `tool_call` returns `{block:true}` | No `tool_result`; `tool_execution_end` `isError:true`; no file created |
| `captures/probeA-throw.jsonl` | `tool_call` throws | Identical shape to block |
| `captures/probeB-later-block.jsonl` | earlier ext admits silently, later ext blocks | No execution despite earlier "successful" `tool_call` |
| `captures/probeC-order-throw.jsonl` | earlier ext throws in `tool_call` (3-ext array) | Later extensions' `tool_call` never reached; all 3 exts' `tool_execution_start` still fire first |
| `captures/sigint.jsonl` | SIGINT to process group mid-`sleep` `bash` | pi process dies immediately; orphaned `sh`/`sleep` survive and complete their mutation afterward |
| `captures/sessioninfo.jsonl` | real (non-`--no-session`) session start | Session id = UUIDv7; session file path reveals per-cwd-encoded storage |

## probe-plugins/

- `capture.ts` — the instrumentation extension (also Probe A fault injection
  via `PI_PROBE_FAULT=throw|block`).
- `order-first.ts` / `order-last.ts` — ordering brackets for Probe C.
- `order-last-fault.ts` — later-extension fault injection for Probe B
  (`PI_PROBE_LAST_FAULT=block|throw`).
- `customtool.ts` — `probe_mutate` custom mutating tool for D2.
- `sessioninfo.ts` — reads `ctx.sessionManager` identity for D11.

Comment-free per repository convention. Reference material for T02–T05, not
production code, and not on any build/workspace include path.

## Scenarios not exercised live (time/environment limits, not credential limits)

Unlike the OpenCode T01 (which hit a real credential wall for `apply_patch`),
every tool and lifecycle path here was reachable with the operator's existing
Pi credentials. The scenarios below were skipped for probe-harness/time
reasons and are flagged for T02–T06 to close before `/validate`, not because
the pinned version lacks the capability:

- **Session resume/fork/reload/switch, `agent_end` vs `agent_settled`
  distinctness under retry/compaction, `SIGKILL` (vs the `SIGINT` proven
  above), and two genuinely separate Pi processes racing one checkout.** All
  are documented mechanisms in `docs/extensions.md`'s Lifecycle Overview and
  Session Events sections and are structurally consistent with everything
  proven above (in particular, D11's per-session-file, per-cwd-directory
  storage scheme and D1's per-call identity scheme give no reason to expect
  different behavior), but were not independently captured.
- **Built-in tool-name replacement** (D2) — documented as supported by
  `docs/extensions.md` but not driven live.
- **Model switch mid-session** (`model_select` event) — documented, not
  captured live; D11's live-`ctx.model`-per-call design makes this low-risk
  by construction (each `tool_call` reads the model current at that instant),
  but the mechanism itself deserves a live regression before `/validate`'s
  AC13.

None of these are Probes A/B/C (all three of which are proven above), and
none currently contradict any D1–D14 disposition; they are recorded here so
T03–T06 do not silently assume live coverage that wasn't actually collected.
