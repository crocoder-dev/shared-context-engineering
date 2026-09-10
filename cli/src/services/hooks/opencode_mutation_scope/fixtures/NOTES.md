# T01 — OpenCode mutation lifecycle evidence

Frozen lifecycle evidence for the OpenCode mutation-scope integration
(`context/plans/opencode-mutation-scope-integration.md`). Every load-bearing
assumption behind design decisions **D1–D11** carries a disposition here, backed
by a captured event sequence in `captures/` and/or a citation into pinned
upstream `sst/opencode` source. A contradictory reading of this evidence is a
re-planning gate for T02+, per the plan's Design preamble.

Dispositions use exactly three values:

- `PROVEN` — observed live in `captures/` and/or fixed by pinned upstream source.
- `DOCUMENTED — NON-LOAD-BEARING` — characterised, but no plan decision rests on it.
- `UNSUPPORTED` — the intended behavior does not hold on the pinned versions.

## Pinned versions (evidence is version-bound)

| Component | Version | Provenance |
|---|---|---|
| OpenCode CLI (`opencode-ai`) | **1.15.4** | selected and recorded before the first load-bearing probe, per the plan's *OpenCode/plugin version policy*; installed locally into the probe runtime, never globally |
| `@opencode-ai/plugin` | **1.15.4** | the exact repo-pinned value inherited from PR #275 (`config/lib/package.json`); resolved transitively `@opencode-ai/sdk@1.15.4`, `effect@4.0.0-beta.65`, `zod@4.1.8` |
| Bundled Bun runtime | **1.15.4** build | reported by `opencode --version` internals; `opencode.exe` is a Bun single-file executable (`linux-x64`) |
| Upstream source | tag **`v1.15.4`**, commit **`2b92c5677e830e95d34fc3d5664a69297d2d0b51`** | `github.com/sst/opencode`; CLI and `@opencode-ai/plugin` share one release tag in that monorepo |
| OS | Linux 6.18.37 `x86_64`, NixOS 26.05 | — |

The CLI version and the plugin version are **identical (1.15.4)** here, so there
is no cross-version equivalence gap to argue: the plugin API surface, the CLI
runtime, and the cited source all correspond to `v1.15.4`. Neither version may
change for T02+ without re-running this probe matrix (plan policy).

All load-bearing source citations are `packages/...` paths **at tag `v1.15.4`**.

## Probe environment and method

- **Probe repo:** a throwaway `git init` repo (`<PROBE_REPO>` in the captures),
  never the SCE checkout. Two commits of seed content.
- **Isolation:** `XDG_DATA_HOME` / `XDG_CONFIG_HOME` / `XDG_STATE_HOME` /
  `XDG_CACHE_HOME` redirected to a scratch tree so the probe CLI got a fresh
  `opencode.db` and never shared state with the operator's installed OpenCode
  1.18.x. `auth.json` was copied into the isolated data dir (provider auth lives
  in the data dir, not config).
  - Sharing the operator DB first produced
    `NOT NULL constraint failed: session_message.seq` — a 1.15.4 binary against a
    DB already migrated by 1.18.x. Isolation fixed it. This confirms **OpenCode
    persistence is global-user-scoped and schema-version-coupled**, not
    checkout-local (relevant to D11 / T04).
- **Instrumentation:** `probe-plugins/capture.ts` is a plugin registered through
  the project `.opencode/opencode.json` `plugin` array. It records every hook
  invocation (`tool.execute.before`, `shell.env`, `tool.execute.after`,
  `chat.params`, `chat.message`, `permission.ask`, `config`, `tool.definition`)
  and every `event(...)` payload to a JSONL log, each line stamped with a
  monotonic `mono_us` (`process.hrtime.bigint()`), wallclock, `pid`, and a
  per-process `seq`. OpenCode plugins receive **typed function arguments**, not
  a raw STDIN payload (unlike Codex hooks), so the captures are the
  instrumentation's structured record of those arguments, not byte-for-byte
  stdin. `message.part.delta` (token-stream) events were filtered out of the
  committed captures; `<PROBE_REPO>` replaces the absolute scratch path.
- **Ordering probes:** `probe-plugins/order-first.ts` and `order-last.ts` bracket
  `capture.ts` in the `plugin` array to observe multi-plugin hook ordering and
  fail-closed barriers. `order-first.ts` can throw synchronously in
  `tool.execute.before` (Probe C).
- **Fault injection:** `capture.ts` throws in `tool.execute.before` when
  `OC_PROBE_FAULT=before` (Probe A) or in `shell.env` when
  `OC_PROBE_FAULT=shellenv` (Probe B).
- **Custom tool probe:** `probe-plugins/customtool.ts` registers a
  filesystem-mutating plugin tool `probe_mutate` (D2 classification).
- **Model:** `opencode/big-pickle` (OpenCode Zen, free, `providerID:"opencode"`,
  `api.id:"big-pickle"`) drove every model-dependent probe. It reliably emits
  `bash`, `write`, `edit`, and `task` tool calls. See **apply_patch** below for
  why that tool could not be driven live on this credential set.
- **Signals:** `opencode run` was launched under `setsid`; `SIGINT` / `SIGKILL`
  were delivered to its process group once `shell.env` for a long `sleep`
  command had been observed in the capture.

## Tool vocabulary at v1.15.4 (D2 substrate)

Registry: `packages/opencode/src/tool/registry.ts`.

| Tool id | Constant | Registered when |
|---|---|---|
| `bash` | `ShellID.ToolID` (`tool/shell/id.ts:16` — literally `"bash"`) | always |
| `write` | `WriteTool.id` (`tool/write.ts:28`) | model id does **not** match the patch gate |
| `edit` | `EditTool.id` (`tool/edit.ts:59`) | model id does **not** match the patch gate |
| `apply_patch` | `ApplyPatchTool.id` (`tool/apply_patch.ts:23`) | model id **does** match the patch gate |
| `task` | `TaskTool.id` (`tool/task.ts` `id = "task"`) | always |
| `read`,`glob`,`grep`,`webfetch`,`websearch`,`todo`,`skill`,… | — | always / flag-gated |
| plugin `tool: {}` entries, MCP tools | — | always, when configured |

**The patch gate** (`registry.ts` `tools()` filter):

```
const usePatch = input.modelID.includes("gpt-") && !input.modelID.includes("oss") && !input.modelID.includes("gpt-4")
if (tool.id === ApplyPatchTool.id) return usePatch
if (tool.id === EditTool.id || tool.id === WriteTool.id) return !usePatch
```

Consequence for the plan: **`apply_patch` and `edit`/`write` are mutually
exclusive within a single session**, decided by the executing model id. A
GPT‑5‑class session exposes `{bash, apply_patch}`; every other session exposes
`{bash, write, edit}`. `task` is always present. So the *four intended tracked
tool names* are all real and trackable, but **no single session ever sees all
four** — the maximal tracked set per session is three. This does not weaken the
soundness contract (see D2).

---

## D1 — Scope identity is one OpenCode tool execution

**Disposition: PROVEN.**

`tool.execute.before` / `tool.execute.after` inputs carry exactly
`{ tool, sessionID, callID }` (plus `args` on `after`).
Source: `packages/plugin/src/index.ts` `Hooks["tool.execute.before"]` /
`["tool.execute.after"]`; call sites `packages/opencode/src/session/prompt.ts`
(registry-tool path ~L575–L600, task path ~L755 / ~L834).

Observed identifiers (v1.15.4):

- `sessionID` — `ses_` + 24 base62 chars, e.g. `ses_f74901593ffeCRyKXZhIbXYsC4`
  (`captures/bash-success.jsonl`). Monotonic-ish (time-prefixed); globally unique.
- `callID` — `call_` + 24 hex, e.g. `call_13ebe30069304f5aa6eaeb74`
  (`captures/shape` era; every `bash-*` / `write-*` / `edit-*` capture). One
  `callID` per tool invocation; **stable across that invocation's
  `before` → `shell.env` → `after`** (verified byte-identical in
  `captures/bash-success.jsonl`, `captures/parallel-forced.jsonl`).
- Concurrent calls in one session get **distinct** `callID`s and remain
  distinguishable throughout — see D9 (`captures/parallel-forced.jsonl`:
  `call_03dd117ed0e24aa9bf7df5cd` and `call_279a576aed6c47c3b9c60a1a` overlap).
- The **`task`** tool call, in the parent-loop path, uses `callID: part.id`
  (an ascending `prt_...` id), not a `call_...` id
  (`session/prompt.ts` ~L755; `captures/subagent.jsonl` shows
  `call_efe5ddbc02d54cac9a9e6ef3` for the task — the registry path — so both
  forms occur depending on how the model emits the call).

**Freeze:** `ScopeId` identity is `(sessionID, callID)`. `callID` alone is
already collision-safe in practice, but `sessionID` is required to bracket
subagent sessions (D9) and to build session provenance (D8). No turn/agent/step
identity is needed for uniqueness — nothing in the captures shows `callID` reuse
across concurrent or sequential calls. The plan's intended encoding
`oc-tool-v1|s=<len>:<sessionID>|c=<len>:<callID>` is consistent with the observed
identifiers (both are opaque `[A-Za-z0-9_]` strings; length-prefixing is
sufficient, no hashing needed).

---

## D2 — Tool classification is explicit

**Disposition: PROVEN.**

- `tool.execute.before` / `after` fire for **every registry tool**, including
  read-only ones: `captures/edit-success.jsonl` shows `read`
  (`call_c0ab65f5082d4a35b8713027`) bracketed by `before`/`after` exactly like
  the tracked `edit` call that follows.
- **Plugin-defined tools fire the same hooks and can mutate.**
  `captures/customtool.jsonl`: `probe_mutate` (`call_87a39ff955614cf58a13f206`)
  is bracketed by `before`/`after`, and its `execute` wrote `ct.txt`
  (`customtool.execute` record). Registry wrapping: `registry.ts` `fromPlugin(...)`.
- MCP tools: same `before`/`after` bracket, plus an unconditional
  `ctx.ask({ patterns:["*"] })` inside `execute`
  (`session/prompt.ts` MCP branch ~L605–L620). Not driven live (no MCP server
  configured in the probe repo); source is unambiguous.

**Therefore hook presence is not a mutation signal.** Classification must be a
closed allowlist keyed on the exact `tool` string:

```
bash | write | edit | apply_patch   -> TrackedMutation
task                                -> Delegation
everything else (read/glob/grep/... , MCP, plugin tools, unknown/future) -> Untracked
```

`Untracked` = tool executes, may mutate, **no scope created, no positive
individual attribution claimed** — exactly the plan's D2 text. `probe_mutate`
mutating `ct.txt` with no scope is the intended, safe outcome (the interval is
covered by the conservative unscoped fallback, not by a false AI attribution).

No tool in the intended tracked set fails the soundness contract, so the tracked
set is **not reduced**. It is, per session, at most `{bash, write, edit}` or
`{bash, apply_patch}` plus always-present `task` as Delegation (see the patch
gate above).

---

## D3 — Confirmation-required attribution becomes generic

**Disposition: PROVEN (evidence basis for the generalization; the protocol edit itself is T02).**

The load-bearing fact D3 rests on is: **an OpenCode tracked tool can reach its
Start boundary and then terminate with no successful `Close`.** Proven three ways:

1. **Permission rejection after Start** — `captures/edit-perm-ask.jsonl`
   (`OPENCODE_PERMISSION={"edit":"ask"}`):
   `tool.execute.before` (`edit`, `call_08913c9ba6734a16937486b9`) fires →
   `permission.asked` (`permission:"edit"`) → `permission.replied`
   `reply:"reject"` (headless `opencode run` auto-rejects any `ask`) →
   **no `tool.execute.after`**, file unchanged.
   Source: `ctx.ask` is `permission.ask(...).pipe(Effect.orDie)`
   (`session/prompt.ts` `resolveTools.context`); a `PermissionRejectedError` /
   `PermissionDeniedError` (`packages/opencode/src/permission/index.ts` L75/L89)
   becomes a defect, so `item.execute` dies **before** the `after` trigger
   (`session/prompt.ts`: `after` is a separate `yield*` after
   `yield* item.execute(...)`).
2. **Interrupt during execution** — `captures/sigint.jsonl` /
   `captures/sigkill.jsonl`: `before` + `shell.env` recorded, then the capture
   **stops** — no `after`, no `session.idle`, no disposal event (see D11).
3. **Internal validation failure** — `apply_patch` `Effect.fail(...)` on a bad
   patch, before `ctx.ask` and before any write
   (`tool/apply_patch.ts` L36–L52); same "no `after`" outcome. (Source only;
   see apply_patch section.)

Contrast: `bash` **non-zero exit**, **exit 127**, and **timeout** all still fire
`tool.execute.after` — they are *successful tool results* (see D4). So `after` is
a precise "the tool ran to a normal completion" signal, and its absence is
genuinely ambiguous (rejected / crashed / validation-failed / mutated-then-threw).

This is exactly the Codex situation. The generalization
`requiresBoundaryConfirmation(actor_kind)` with `Codex -> true`,
`OpenCode -> true`, `Claude -> false`, `Pi -> unchanged` is justified: an
OpenCode scope must be treated as unconfirmed until its own successful
`Close(scope)`. Implementing that in `spec/mutation_cursor.qnt` /
`protocol.rs` / `mbt/` is **T02's** scope; T01 only freezes the evidence that the
"Start without Close" state is reachable for OpenCode.

---

## D4 — Bash uses the strongest available pre-execution boundary

**Disposition: PROVEN.**

Source-observed lifecycle for the `bash` tool
(`packages/opencode/src/tool/shell.ts`, `ShellTool.execute` ~L610–L642):

```
tool.execute.before                         (session/prompt.ts, before item.execute)
  -> parse command, collect path scan
  -> yield* ask(ctx, scan)                   (shell.ts L628 -> ctx.ask -> Effect.orDie)   [permission]
  -> run({ ..., env: yield* shellEnv(ctx, cwd), ... }, ctx)
       shellEnv: plugin.trigger("shell.env", { cwd, sessionID, callID }, { env:{} })     (shell.ts L412)
  -> spawner.spawn(cmd(shell, command, cwd, env))                                        (shell.ts L482)
  -> stream output ...
tool.execute.after                          (session/prompt.ts, after item.execute)
```

Live confirmation of the ordering (`captures/bash-success.jsonl`,
`captures/parallel-forced.jsonl`, `captures/order-observe.jsonl`):
`tool.execute.before` → `shell.env` (same `callID`, ~30–40 ms later) →
`tool.execute.after`. `shell.env` input is `{ cwd, sessionID, callID }` with
`callID` matching the tool call.

**`shell.env` fires after OpenCode's permission evaluation and before process
spawn** — proven by:

- **Rejected bash never reaches `shell.env`.** `captures/bash-perm-ask.jsonl`
  (`{"bash":"ask"}` → headless reject): `tool.execute.before` fires,
  `permission.asked` / `permission.replied reject`, then **no `shell.env`, no
  `after`**. A `shell.env`-anchored Start therefore creates **zero scope** for a
  rejected bash — the plan's D4 requirement.
- **Config-level `deny` removes `bash` from the registry entirely.**
  `captures/bash-perm-deny.jsonl` (`{"bash":"deny"}`): the model is told
  `unavailable tool 'bash'. Available tools: edit, glob, grep, invalid, read,
  skill, task, todowrite, webfetch, websearch, write` — no `bash`, no
  `tool.execute.before` for bash at all. Source: `registry.ts` ~L299
  (`rule.pattern === "*" && rule.action === "deny"` → tool excluded).
- **Probe B** proves a failed `shell.env` blocks the spawn (see Probe B).

`shell.env` also fires for the interactive `!`-prefixed shell / bang-command path
(`session/prompt.ts` ~L1015, `callID: part.callID`) — a separate real shell
spawn in the session directory. The adapter sees `{cwd, sessionID, callID}` in
both cases; distinguishing "bash tool" from "bang command" is not possible from
the `shell.env` payload alone, but both represent a genuine AI-initiated shell
execution in the worktree, so treating both as a tracked bash Start is sound
(the bang path is rare and out of the plan's core scope; **DOCUMENTED**).

**Freeze:** `bash` Start = `shell.env`, keyed `(sessionID, callID)`. Terminal
cases and their `after` behavior:

| bash outcome | `tool.execute.after`? | file side effects | capture |
|---|---|---|---|
| success (exit 0) | **yes** | applied | `bash-success.jsonl` |
| non-zero exit (`exit 7`) | **yes** (exit code is data) | partial writes persist | `bash-nonzero.jsonl` |
| command not found (127) | **yes** | — | `session-error.jsonl` |
| timeout (tool-enforced) | **yes**, output carries `<shell_metadata>… exceeded timeout …</shell_metadata>` | whatever ran before kill | `bash-timeout.jsonl` |
| detached/background descendant (`nohup … &`) | **yes** (parent returns; descendant keeps running) | descendant may mutate after `after` | `bash-detached.jsonl` |
| permission `ask` → reject | **no** `shell.env`, **no** `after` | none | `bash-perm-ask.jsonl` |
| permission `deny` (config) | tool absent; nothing | none | `bash-perm-deny.jsonl` |
| SIGINT / SIGKILL mid-run | **no** `after`, no terminal event | orphaned child may still mutate | `sigint.jsonl`, `sigkill.jsonl` |

So a bash Start (`shell.env`) followed by `after` = the tool ran to a normal
end, **but background descendants and post-`after` orphans mean `after` is not
proof that all mutation by that scope has ceased** (D11). D3 remains the
correctness boundary.

---

## D5 — File mutation tools use write-ahead Start

**Disposition: PROVEN for `write` and `edit` (live); PROVEN-by-source for `apply_patch`.**

`write` / `edit` (`captures/write-success.jsonl`, `captures/edit-success.jsonl`):

```
tool.execute.before  (tool: "write"|"edit", callID)      <- Start (write-ahead)
  -> inside item.execute: assertExternalDirectory, compute diff
  -> ctx.ask({ permission: "edit", ... })                <- may reject (orDie -> defect)
  -> fs.writeWithDirs(...) / apply edit                  <- the mutation
  -> format, bus events, LSP diagnostics
tool.execute.after   (tool: "write"|"edit", callID, args, output)   <- Close, only on full success
```

Source: `tool/write.ts` L38–L88 (ask at L54, write at L70);
`tool/edit.ts` L69+ (ask at L98 / L141).

**Start (`tool.execute.before`) carries no permission or validation guarantee.**
`captures/edit-perm-ask.jsonl` proves the write-ahead orphan: `before` (Start)
fires, permission rejected, **no `after`**, no mutation. The scope is live in the
adapter with no confirming Close — handled by D3, never by inferring execution
from the missing `after`.

`apply_patch` (`tool/apply_patch.ts`): identical shape on the registry path —
`tool.execute.before` → `run(params, ctx)` [ `patchText` present check →
`Patch.parsePatch` (throws → `Effect.fail`, L36–L52) → build file changes →
`ctx.ask({permission:"edit"})` (L206) → `afs.writeWithDirs` / update / delete
(L228+) ] → `tool.execute.after` only on full success. `callID` is
`options.toolCallId` (registry path), same as `write`/`edit`. **A bad patch fails
before `ctx.ask` and before any write; a permission rejection fails after
`before`; both yield no `after`.**

**`config`-level `{"edit":"deny"}` removes `write`, `edit`, and `apply_patch`
together** (all use `permission: "edit"`). `captures/edit-perm-deny.jsonl` /
`captures/write-perm-deny.jsonl`: the model falls back to `bash`. So the
write-ahead-then-rejected case for file tools is only reachable with
`{"edit":"ask"}`, which `edit-perm-ask.jsonl` captures.

---

## D6 — Generated plugin ordering is load-bearing

**Disposition: PROVEN.**

Plugin hook dispatch: `packages/opencode/src/plugin/index.ts`.

- Plugins load **sequentially, in array order**; hooks are pushed in that order
  ("Keep plugin execution sequential so hook registration and execution order
  remains deterministic"). `trigger(name, input, output)` iterates
  `s.hooks` in order: `for (const hook of s.hooks) { ... yield* Effect.promise(async () => fn(input, output)) }`.
- INTERNAL auth plugins are prepended; external plugins follow, in the order of
  the merged `plugin` config array.

**The explicit `plugin` array order is honored end-to-end.**
`captures/order-observe.jsonl` — array
`["./probe/order-first.ts","./probe/capture.ts","./probe/order-last.ts"]` — every
hook (`plugin.init`, `tool.execute.before`, `shell.env`, `tool.execute.after`)
fires `order-first` → `capture` → `order-last`, for the same `callID`.

**No double-registration when an auto-discovered file is also listed explicitly.**
`captures/dup.jsonl`: `dup.ts` placed in the auto-discovered `.opencode/plugin/`
**and** listed as `./plugin/dup.ts` → `plugin.init` fires **once**. The `config`
hook's `pluginList` shows every entry normalized to a single `file://` URL.
Source: `deduplicatePluginOrigins` (`config/plugin.ts` L69) dedupes on the
resolved `file://` spec; relative specs are normalized to `file://` before the
merge (`plugin/shared.ts` `resolvePathPluginTarget`, `pathToFileURL`).

**Caveat (T05, not T01):** ordering among *purely* auto-discovered plugins
(`{plugin,plugins}/*.{ts,js}`, `config/plugin.ts` `load()`) is `glob` package
order — **not sorted** (`packages/core/src/util/glob.ts` wraps `glob` with no
`sort`). SCE must keep its plugins as **explicit `plugin` array entries** in the
generated `opencode.json` (as it does today for `sce-bash-policy` /
`sce-agent-trace`) and append `sce-mutation-scope` as the final array entry; it
must not rely on filename glob order. Setup-merge/doctor must assert the mutation
scope plugin is the last array entry after arbitrary user plugins are merged in.

**Fail-closed barrier for the ordering contract: see Probe C.**

---

## D7 — The TypeScript plugin is a thin transport adapter

**Disposition: PROVEN (feasibility) / architectural.**

Everything the plan wants the TS plugin to own is available synchronously in the
hook payloads:

- **Identity:** `sessionID` + `callID` on `tool.execute.before` / `shell.env` /
  `tool.execute.after`.
- **Model observation:** `chat.params` input `{ sessionID, agent, model, provider, message }`
  — see D8.
- **Fail-closed synchronous Start:** hooks are `async` and **awaited** in the
  trigger loop (`plugin/index.ts`: `yield* Effect.promise(async () => fn(...))`);
  a hook that throws/rejects deterministically blocks the tool (Probes A/B/C).
  So the TS plugin can do a synchronous, fail-closed call into
  `sce hooks opencode-mutation-scope` and, on transport failure, throw to prevent
  the mutation.
- **Best-effort terminal forwarding:** `tool.execute.after` + the async
  `event(...)` stream (`message.part.updated` with `part.state.status`,
  `session.idle`, `session.error`, `server.instance.disposed`).

No mutation-protocol state is exposed to the plugin — `active_scopes`,
attribution, durable state all stay in Rust. The captures contain nothing that
would require protocol logic in TS.

---

## D8 — Model provenance is observed, not inferred

**Disposition: PROVEN.**

`chat.params` (`packages/opencode/src/session/llm.ts` ~L162) fires **before every
LLM call**, i.e. before the assistant turn that emits tool calls. Input
(`packages/plugin/src/index.ts`):

```
{ sessionID, agent, model: Provider.Model, provider: ProviderContext, message: UserMessage }
```

Observed `model` object (`captures/bash-success.jsonl` seq 13):

```json
{ "id":"big-pickle", "providerID":"opencode", "api": { "id":"big-pickle", "url":"https://opencode.ai/zen/v1", "npm":"@ai-sdk/openai-compatible" }, ... }
```

so `provenance.model_id` can be built as `providerID/api.id` (normalized), and
`provider.source` (`"custom"` here, also `"env"|"config"|"api"`) is available.

Ordering per turn (`captures/bash-success.jsonl`): `chat.params(agent:title)` →
`chat.params(agent:build)` → … → `tool.execute.before`. It fires **once per
turn** (three times in `bash-success` — title turn, the tool turn, the final
turn). An ephemeral `sessionID -> model` map updated on `chat.params` is
therefore always populated before that session's next `tool.execute.before`.

**Nuances the adapter must respect:**

- `chat.params` also fires for the internal **`title`** agent (small-model
  summary). The map should track the **primary/build** agent's model, keyed by
  `sessionID` + filtered by `agent` (or by ignoring `agent:"title"`), or it will
  occasionally record the title model.
- **Subagents get their own `chat.params`** with the **child `sessionID`** and
  their own agent (`captures/subagent.jsonl`: parent
  `ses_f7485fb0fffeGGPxaJ037TfPpB` agent `build`; child
  `ses_f7485d3e6ffepjZy8I8T0UNpaa` agent `general`). Keying the map by
  `sessionID` handles this automatically — the child's tool calls carry the
  child `sessionID`.
- **Model switch:** `chat.params` re-fires per call with the then-current
  `input.model`; a `session.next.model.switched` event
  (`captures/*` seq ~6) also announces it. So the map self-heals on the next
  turn. A tool call that somehow precedes any `chat.params` for its session →
  **no model evidence → persist `NULL`** (never guess, never copy another
  session's model — plan D8).
- The child session's model, if the subagent definition pins none, is inherited
  from the parent's assistant message (`tool/task.ts` L172–L176), and the
  child's own `chat.params` still reports it explicitly.

**Freeze:** at Start, `provenance.session_id = oc_<sessionID>`,
`provenance.model_id = normalized(providerID + "/" + api.id)` from the live
per-session `chat.params` observation, else `NULL`.

---

## D9 — Legitimate OpenCode parallelism is preserved

**Disposition: PROVEN.**

`captures/parallel-forced.jsonl` (one model response, two `bash` calls):

```
mono_us     hook                   callID
4315308     tool.execute.before    call_03dd117ed0e24aa9bf7df5cd   (A)
4351979     shell.env              call_03dd117ed0e24aa9bf7df5cd   (A)
4638370     tool.execute.before    call_279a576aed6c47c3b9c60a1a   (B)   <- B starts while A live
4643768     shell.env              call_279a576aed6c47c3b9c60a1a   (B)
8362235     tool.execute.after     call_03dd117ed0e24aa9bf7df5cd   (A)   <- ~4s later (sleep 4)
8651783     tool.execute.after     call_279a576aed6c47c3b9c60a1a   (B)
```

Both commands were `sleep 4; echo …`; B's Start (`before` + `shell.env`) lands
~3.7 s before A's Close. **Two live tracked bash scopes, same `sessionID`,
different `callID`, genuinely concurrent.** Starting B must not retire A.

(Note: a naive "run two commands" prompt — `captures/parallel.jsonl` — was
executed *sequentially* by this model; the explicit single-response prompt was
needed to force overlap. Both are committed.)

Hooks themselves are still dispatched atomically (the trigger loop is
sequential); only the spans **between** `before` and `after` overlap. There is no
"same-session predecessor" signal that could justify retiring A when B starts —
`sessionID` equality is not evidence of anything. **Do not port Codex's
same-lane predecessor sweep.** Any stale-attempt recovery must key on the exact
`(sessionID, callID)` and needs stronger evidence than "a later call started"
(D11).

Subagent nesting (`captures/subagent.jsonl`): parent `task` scope
(`call_efe5ddbc02d54cac9a9e6ef3`, parent session) is live across the child
session's `bash` scope (`call_43265d7f533a4973bc0b0ee9`, child session);
`tool.execute.after` for `task` fires only after the child finishes. Different
`sessionID`s keep them separate with no special handling.

---

## D10 — Asynchronous events may clean up but do not establish attribution

**Disposition: PROVEN.**

The `event(...)` hook is **fire-and-forget**: `plugin/index.ts` subscribes the
bus and calls `void hook["event"]?.(...)` — **not awaited**, on a forked fiber.
So event delivery is not ordered w.r.t. the synchronous tool trigger path,
even though in this single-process CLI they often *appear* interleaved in
`mono_us` order.

Event union at v1.15.4 (`packages/sdk/js/src/gen/types.gen.ts` `Event`): includes
`message.part.updated`, `message.part.removed`, `permission.updated`,
`permission.replied`, `session.status`, `session.idle`, `session.error`,
`session.updated`, `session.created`, `session.deleted`,
`server.instance.disposed`, `file.edited`, `file.watcher.updated`, `pty.*`, …

Useful for **exact-attempt cleanup**, keyed by `callID`:

- `message.part.updated` with `part.type:"tool"` carries
  `{ callID, tool, state.status }` transitioning
  `pending → running → completed | error`
  (`captures/bash-success.jsonl`: pending→running→running→completed).
  A `state.status:"error"` for a known live `callID` is a legitimate
  `Abandon(scope)` trigger.
- `session.idle` + `server.instance.disposed` mark a clean end of turn / clean
  shutdown (every non-signal capture ends with both).

**But these events do NOT arrive on interrupt.** `captures/sigint.jsonl` and
`captures/sigkill.jsonl` end after `shell.env` with a single stray
`session.updated` and then **nothing** — no tool `error` part, no
`session.error`, no `session.idle`, no `server.instance.disposed`. So positive
attribution must never depend on an async cleanup event arriving before the next
mutation boundary. An exact terminal failure event may drive `Abandon(scope)`,
but D3 remains the correctness boundary while any cleanup is pending or lost.

`session.error` was not observed in any probe (the model recovered from
command-not-found within the same session — `captures/session-error.jsonl`);
its payload shape is taken from source (`EventSessionError`,
`{ sessionID, error: NamedError.toObject() }`).

---

## D11 — No timeout-based correctness

**Disposition: PROVEN (uncertainty is real and unavoidable; no safe TTL exists).**

- **Graceful:** `session.idle` then `server.instance.disposed` end every clean
  run. `opencode run` is one-shot: it disposes the instance immediately after
  the turn.
- **SIGINT** (`captures/sigint.jsonl`): single Ctrl-C in `opencode run` →
  `footer.requestExit()` (`cli/cmd/run/runtime.lifecycle.ts` L243/L248) → the
  process exits **fast**. Capture stops right after `shell.env` — **no `after`,
  no idle, no disposal event**. The spawned `bash -c "sleep 25; echo done > sig.txt"`
  child was **orphaned and ran to completion**: `sig.txt` appeared with `done`
  **after `opencode.exe` was already gone**.
- **SIGKILL** (`captures/sigkill.jsonl`): even more abrupt — capture ends at a
  `part.state.status:"running"` event; the orphaned `bash -c` and its `sleep 90`
  child were confirmed **still alive after the OpenCode process group was
  killed**, reparented to init, and would have written `sigk.txt` ~90 s later.
- **Detached descendant** (`captures/bash-detached.jsonl`): `nohup … &` inside a
  normally-completing bash call — `tool.execute.after` fires while the descendant
  keeps running. `after` (Close) does **not** imply the scope's process tree has
  stopped mutating.
- **Persistence is global-user-scoped** (`~/.local/share/opencode/opencode.db` +
  `storage/`, keyed by a `projectID` hash of the directory), schema-coupled to
  the CLI version, and **not checkout-local**. OpenCode offers no per-checkout
  attempt bookkeeping to piggyback on. Two OpenCode processes in one checkout
  share that DB and interleave; their tool scopes stay separable only because
  `sessionID`/`callID` are globally unique (T04 must not assume one writer).

**Implication for D11 / T04:** after a hard crash or restart, the adapter's
durable state can hold a live OpenCode scope for which (a) no terminal hook
fired, (b) no async event fired, and (c) an orphaned child may still be
mutating the worktree. **Time since Start cannot distinguish "abandoned" from
"orphan still writing".** A TTL that retires such a scope risks a false negative
window *and*, worse, could let a later boundary claim `AiExclusive` while an
orphan mutates. The safe posture (already the plan's): keep the scope
confirmation-required (D3) so it never produces positive attribution, recover it
only on explicit exact-`callID` terminal evidence, and **document the
availability cost** (intervals around an interrupted OpenCode tool stay
`IneligibleUnscoped`) rather than hide it behind a timer.

---

## Fail-closed execution-barrier probes

### Probe A — `tool.execute.before` failure  ·  PROVEN

`captures/probeA-before-throw.jsonl` (`OC_PROBE_FAULT=before` on `capture`, a
`write`): array `order-first, capture, order-last`.

```
order-first  tool.execute.before   (write)
capture      tool.execute.before   (write)   -> throws "OC_PROBE_FAULT before (capture)"
<order-last  tool.execute.before>             NOT REACHED
<shell.env / tool.execute.after>              NOT REACHED
```

`probeA.txt` was **not created** — the `write` tool's `execute` never ran.
OpenCode surfaced `Error: OC_PROBE_FAULT before (capture)` and marked the tool
call `✗ failed`. Mechanism: `trigger` loop does
`yield* Effect.promise(async () => fn(input, output))`; a rejected promise is an
unrecoverable Effect **defect**, so the loop aborts (later plugins skipped) and
the defect propagates through `run.promise` (`Effect.runPromise`,
`effect/bridge.ts`) as a rejected promise to the AI SDK, which reports a tool
error **without invoking `item.execute`**.

### Probe B — `shell.env` failure  ·  PROVEN

`captures/probeB-shellenv-throw.jsonl` (`OC_PROBE_FAULT=shellenv` on `capture`, a
`bash` with an observable side effect):

```
order-first  tool.execute.before   (bash)
capture      tool.execute.before   (bash)
order-last   tool.execute.before   (bash)       <- full before-chain completes
order-first  shell.env
capture      shell.env                          -> throws "OC_PROBE_FAULT shellenv (capture)"
<order-last  shell.env>                          NOT REACHED
<tool.execute.after>                             NOT REACHED
```

`probeB.txt` was **not created** — the Bash **child process was never spawned**.
Mechanism: `ShellTool.shellEnv` is `plugin.trigger("shell.env", …)` whose defect
propagates out of `run({ …, env: yield* shellEnv(ctx, cwd), … })` argument
evaluation, so `ShellTool.run` — and `spawner.spawn` (`shell.ts` L482) — are
never reached.

### Probe C — earlier-plugin synchronous failure blocks later plugins  ·  PROVEN

`captures/probeC-order-throw.jsonl` (`OC_PROBE_ORDER_FIRST=throw`; array
`order-first, capture, order-last`; a `bash`):

```
order-first  tool.execute.before   (bash)   -> throws "order-first synchronous throw"
<capture     tool.execute.before>            NOT REACHED
<order-last  tool.execute.before>            NOT REACHED
<shell.env / tool.execute.after>             NOT REACHED
```

`probeC.txt` was **not created**. A synchronous throw in an earlier plugin's
`tool.execute.before` prevents every later plugin's `tool.execute.before` and
prevents the tool from executing — the D6 correctness contract. Because SCE's
merge keeps non-SCE plugins **before** generated SCE plugins and the mutation
scope plugin is **last**, an earlier policy/user plugin that rejects a tool
rejects it **before** the mutation-scope Start is established (the plan's D6
intent), and a failure in `sce-bash-policy` or `sce-agent-trace` likewise
prevents the mutation-scope `before`/`shell.env` from running.

---

## apply_patch — live-probe limitation

`apply_patch` could **not** be exercised live on this machine's credentials:

- The patch gate requires `modelID` containing `gpt-` (not `oss`, not `gpt-4`).
- `openai/*` here is a ChatGPT-account (Codex) auth that rejects every model
  offered (`"… not supported when using Codex with a ChatGPT account"`).
- `opencode-go/gpt-5.6-luna` → `Insufficient balance`.
- `opencode/*` free models and `ollama-cloud/*` have no `gpt-` model id
  (`gpt-oss:*` is excluded by the `oss` clause).

**Disposition: PROVEN-BY-SOURCE, lifecycle-equivalent to `write`/`edit`.**
This substitution is made under the T01 *Credential-blocked source-only
evidence* rule (`context/plans/opencode-mutation-scope-integration.md`, task
T01): the pinned upstream source is at the exact frozen version, the missing
live probe is recorded here, and the acceptance criterion demanding live
coverage (AC2) stays outstanding — it is **not** satisfied by this source-only
evidence.
`apply_patch` runs on the **same registry path** as `write`/`edit`
(`session/prompt.ts` `resolveTools`), with the same `tool.execute.before` (Start)
and `tool.execute.after` (Close-on-success) brackets and the same `callID`
scheme, differing only in that its internal patch parsing can `Effect.fail`
before `ctx.ask` (`tool/apply_patch.ts` L36–L52, ask at L206, writes at L228+).
Every D1/D3/D5 property proven live for `write`/`edit` transfers directly.

**Action for T02+ / `/validate`:** AC2 asks for real temporary-worktree tests
for all four tracked tools. A GPT‑5‑class OpenCode credential (Zen balance, or a
working `openai` API key) is required to record live `apply_patch` fixtures
`probe apply_patch success` / `validation failure` / `permission rejection`.
This is a **credential gap, not a soundness gap** — `apply_patch` satisfies the
contract on the pinned versions per source — so it is **not** a re-planning
trigger, but T03/T04/T06 should treat live `apply_patch` fixtures as an
outstanding item to close before `/validate`.

---

## Disposition summary

| # | Decision | Disposition | Primary evidence |
|---|---|---|---|
| D1 | Scope identity = one tool execution `(sessionID, callID)` | **PROVEN** | `bash-success`, `parallel-forced`, `subagent`; `plugin/src/index.ts`, `session/prompt.ts` |
| D2 | Explicit tool-name allowlist; `Untracked` ≠ read-only | **PROVEN** | `customtool`, `edit-success` (`read`), `bash-perm-deny`; `registry.ts` |
| D3 | Confirmation-required (Start reachable without Close) | **PROVEN** (T02 implements) | `edit-perm-ask`, `sigint`, `sigkill`; `permission/index.ts`, `session/prompt.ts` |
| D4 | Bash Start = `shell.env` (post-permission, pre-spawn) | **PROVEN** | `bash-success`, `bash-perm-ask`, `bash-perm-deny`, Probe B; `tool/shell.ts` L412/L482/L628 |
| D5 | `write`/`edit`/`apply_patch` write-ahead Start = `tool.execute.before` | **PROVEN** (write/edit live; apply_patch source) | `write-success`, `edit-success`, `edit-perm-ask`; `tool/write.ts`, `tool/edit.ts`, `tool/apply_patch.ts` |
| D6 | Explicit `plugin` array order is load-bearing; last = mutation scope | **PROVEN** | `order-observe`, `dup`, Probe C; `plugin/index.ts`, `config/plugin.ts` |
| D7 | TS plugin is a thin transport adapter | **PROVEN** (feasibility) | all captures; `plugin/index.ts`, `session/llm.ts` |
| D8 | Model observed via `chat.params`, else `NULL` | **PROVEN** | `bash-success` (seq 13), `subagent`; `session/llm.ts` L162, `plugin/src/index.ts` |
| D9 | Legitimate parallelism preserved; no same-session sweep | **PROVEN** | `parallel-forced`, `subagent` |
| D10 | Async events may clean up, never establish attribution | **PROVEN** | `bash-success` (tool parts), `sigint`, `sigkill`; `plugin/index.ts` (`void hook.event`) |
| D11 | No TTL correctness; hard-kill leaves orphans, no signal | **PROVEN** | `sigint`, `sigkill`, `bash-detached`; `cli/cmd/run/runtime.lifecycle.ts` |
| Probe A | `tool.execute.before` throw blocks the tool | **PROVEN** | `probeA-before-throw` |
| Probe B | `shell.env` throw blocks the spawn | **PROVEN** | `probeB-shellenv-throw` |
| Probe C | earlier-plugin sync throw blocks later plugins + tool | **PROVEN** | `probeC-order-throw` |

**Maximal safe v1 tracked-tool set:** `{ bash, write, edit, apply_patch }` as
tool *names*, with `write`/`edit` and `apply_patch` mutually exclusive per
session (patch gate), `task` as `Delegation`, everything else `Untracked`. No
intended tracked tool fails the soundness contract on OpenCode CLI 1.15.4 /
`@opencode-ai/plugin` 1.15.4. **No re-planning gate is triggered.**

**Outstanding (not blocking T01):** live `apply_patch` fixtures require a
GPT‑5‑class OpenCode credential; to be recorded during T03–T06 before `/validate`
runs AC2.

## Capture index

| File | Scenario | Key result |
|---|---|---|
| `captures/order-observe.jsonl` | 3-plugin array, one bash | hook order = array order, per `callID` |
| `captures/probeA-before-throw.jsonl` | `tool.execute.before` throw (write) | tool did not execute; later plugin skipped |
| `captures/probeB-shellenv-throw.jsonl` | `shell.env` throw (bash) | child not spawned; later plugin skipped |
| `captures/probeC-order-throw.jsonl` | earlier plugin throws in `before` (bash) | later plugins + tool blocked |
| `captures/bash-success.jsonl` | `printf > file` | before → shell.env → after; `chat.params` shape |
| `captures/bash-nonzero.jsonl` | `sh -c '… ; exit 7'` | `after` still fires; partial write persists |
| `captures/bash-timeout.jsonl` | `sleep 30`, timeout 2000ms | `after` fires with `<shell_metadata>` timeout note |
| `captures/bash-detached.jsonl` | `nohup sleep 300 & echo …` | `after` fires; descendant keeps running |
| `captures/bash-perm-deny.jsonl` | `OPENCODE_PERMISSION={"bash":"deny"}` | `bash` absent from registry; no hooks |
| `captures/bash-perm-ask.jsonl` | `{"bash":"ask"}`, headless reject | `before` only; no `shell.env`, no `after` |
| `captures/write-success.jsonl` | `write` new file | `before` → `after` |
| `captures/write-perm-deny.jsonl` | `{"edit":"deny"}` | `write`/`edit`/`apply_patch` absent; model uses bash |
| `captures/edit-success.jsonl` | `read` then `edit` | read-only `read` also brackets `before`/`after` |
| `captures/edit-perm-deny.jsonl` | `{"edit":"deny"}` | fallback to bash |
| `captures/edit-perm-ask.jsonl` | `{"edit":"ask"}`, headless reject | `edit` `before` (Start) then reject; **no `after`** |
| `captures/parallel-forced.jsonl` | one response, two `bash` | overlapping live scopes, distinct `callID` |
| `captures/parallel.jsonl` | "run two commands" (naive prompt) | this model serialized them — contrast case |
| `captures/subagent.jsonl` | `task` → child session runs `bash` | child `sessionID` on child tool hooks; own `chat.params` |
| `captures/session-error.jsonl` | bad command then recovery | cmd-not-found still fires `after`; no `session.error` |
| `captures/sigint.jsonl` | SIGINT during `sleep` bash | capture stops after `shell.env`; orphan completes mutation |
| `captures/sigkill.jsonl` | SIGKILL during `sleep` bash | capture stops mid-`running`; orphan tree survives |
| `captures/customtool.jsonl` | plugin tool `probe_mutate` mutates a file | brackets `before`/`after`; must be `Untracked` |
| `captures/dup.jsonl` | plugin listed explicitly + auto-discovered | `plugin.init` once (dedupe) |

## probe-plugins/

- `capture.ts` — the instrumentation plugin (also Probe A/B fault injection).
- `order-first.ts` / `order-last.ts` — ordering bracket; `order-first` is Probe C.
- `customtool.ts` — `probe_mutate` plugin tool for D2.
- `opencode.json` — the probe repo's `.opencode/opencode.json` (`plugin` array).

Comment-free per `feedback_no_comments_in_code`. These are reference material for
T03–T05, not production code, and are not on any `tsconfig` `include` path.

## T03 prerequisite (not done here — evidence-only task)

`flake.nix` `workspaceSrc` (~L184–L208) enumerates fixture directories included
in the Cargo build source, e.g.
`./cli/src/services/hooks/codex_mutation_scope/fixtures`. When T03 adds a
`mod.rs` and tests that `include_str!` these captures, it must add
`(pkgs.lib.fileset.maybeMissing ./cli/src/services/hooks/opencode_mutation_scope/fixtures)`
there. Until then the directory is inert: no `mod.rs`, not referenced by any
crate module, not in `workspaceSrc`, so it does not affect `cargo` or
`nix flake check`.
