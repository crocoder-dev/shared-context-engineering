# OpenCode mutation-scope plugin transport

Detail split out of
[`opencode-mutation-scope-integration.md`](opencode-mutation-scope-integration.md)
to keep that file under the repository's per-file line budget: how the T05
generated plugin observes model provenance and how it is wired into OpenCode's
plugin ordering.

## Model and session provenance

The construction helper `opencode_scope_provenance` exists as of T03 (see
[`opencode-mutation-scope-integration.md`](opencode-mutation-scope-integration.md)'s
**Adapter identity and encoding**); as of T04 the adapter stamps its result onto
every tracked `Start` ingress boundary. As of T05 the generated plugin supplies
the observed `model` from its per-session `chat.params` map (`providerID/api.id`,
ignoring the internal `title` agent); absent evidence forwards `model: null`.

`chat.params` (`packages/opencode/src/session/llm.ts` L162) fires before every
LLM call — before that turn's `tool.execute.before` — carrying
`{ sessionID, agent, model, provider }` with `model.providerID` + `model.api.id`.
An ephemeral per-`sessionID` model map populated on `chat.params` is always ready
before that session's next tracked `Start`. Subagents get their own child-session
`chat.params`. At `Start`: `session_id = oc_<sessionID>`,
`model_id = normalized(providerID + "/" + api.id)` from the live observation,
else `NULL` — never guessed, never copied from another session, never backfilled
(consistent with [`mutation-scope-provenance.md`](mutation-scope-provenance.md)'s
insert-once semantics). The internal `title` agent's `chat.params` must be
ignored so it does not pollute the map. Every non-`title` `chat.params` event for
a session **replaces** that session's cached observation rather than merging
into it: a valid `providerID` + `api.id` overwrites the previous entry, and an
invalid/missing model on a later event **deletes** the cached entry rather than
leaving the prior model in place — a model switch (or a turn with unavailable
model evidence) can never resurrect a stale observation from an earlier turn.

## Generated plugin and ordering

The plugin (`config/lib/mutation-scope-plugin/opencode-sce-mutation-scope-plugin.ts`,
copied verbatim to `config/.opencode/plugins/sce-mutation-scope.ts` by
`config/pkl/generate.pkl`) is a pure transport adapter — no protocol state, no
business logic. It observes the model per turn from `chat.params` (see **Model
and session provenance** above), maps `write`/`edit`/`apply_patch`
`tool.execute.before` → fail-closed `ToolExecuteBefore`, `shell.env` →
fail-closed `ShellEnv` bash `Start`, `tool.execute.after` → best-effort
`ToolExecuteAfter` `Close`, and a tool-part `error` event → best-effort `ToolError`. It calls
`sce hooks opencode-mutation-scope` synchronously (`spawnSync`, 20s timeout) and
**throws** on any failure to establish a tracked `Start` — non-zero adapter
exit, spawn failure, timeout, or the `sce` CLI being absent (`ENOENT`) — so
OpenCode always blocks the tool when Start cannot be established; `ENOENT`
additionally logs a one-line installation warning, but still fails closed like
every other transport failure. The broad asynchronous events `SessionIdle` /
`SessionError` / `ServerDisposed` are not forwarded at all — T04's adapter
dispatch is a genuine no-op for them, so the plugin does not spawn the adapter
process for a signal it knows will do nothing; `session.deleted` only clears the
plugin's own local per-session model cache (see **Model and session
provenance** above).

OpenCode runs plugin hooks **sequentially in merged `plugin` config-array order**
(`packages/opencode/src/plugin/index.ts`): an earlier plugin that throws in
`tool.execute.before` blocks later plugins and the tool
(`captures/probeC-order-throw.jsonl`); a failing `shell.env` blocks the spawn
(`captures/probeB-shellenv-throw.jsonl`). `config/pkl/renderers/common.pkl` lists
`sce-mutation-scope` last, and the config merge
(`cli/src/services/setup/config_merge.rs`) appends the SCE entries after every
surviving user plugin, so the installed order is
`[<user plugins…>, sce-bash-policy, sce-agent-trace, sce-mutation-scope]` for any
configuration — mutation-scope stays last, so an earlier policy/user plugin
rejects a tool before its `Start` is established. `sce doctor`
(`inspect_opencode_plugin_ordering_health`) flags an installed `opencode.json`
that lists `./plugins/sce-mutation-scope.ts` anywhere but last. SCE keeps
explicit array entries because auto-discovered `.opencode/plugin(s)/*` order is
unsorted glob order; an explicit entry and its auto-discovered file dedupe
correctly (`captures/dup.jsonl`). See
[`generated-opencode-plugin-registration.md`](../sce/generated-opencode-plugin-registration.md).
