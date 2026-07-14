# Pi extension runtime (SCE)

Project-local Pi extension that wires the Pi coding agent into SCE runtime
systems. Source of truth: `config/lib/pi-plugin/sce-pi-extension.ts`, emitted
verbatim by `config/pkl/generate.pkl` to `config/.pi/extensions/sce/index.ts`
(same source-to-generated pattern as the OpenCode plugins under
`config/lib/{bash-policy-plugin,agent-trace-plugin}/`).

## Registration model

- Pi auto-discovers `.pi/extensions/*/index.ts` in a trusted project; there is
  no registration manifest (unlike OpenCode's `opencode.json` `plugin` field).
- The extension is a single self-contained TypeScript file with no npm
  dependencies; imports from `@earendil-works/pi-coding-agent` are type-only
  (plus the `isToolCallEventType` narrowing helper).
- `config/lib/package.json` declares `@earendil-works/pi-coding-agent` so the
  source type-checks under `config/lib` tooling; nothing is installed under
  `.pi/extensions/`.
- Entry contract: default export `ExtensionFactory` — `(pi: ExtensionAPI) =>
  void` — registering handlers via `pi.on(event, handler)`.

## Implemented slice: bash policy enforcement

- `pi.on("tool_call", ...)` narrowed to bash via
  `isToolCallEventType("bash", event)`; non-bash events pass through.
- Delegates to `spawnSync("sce", ["policy", "bash", "--input", "normalized",
  "--output", "json"])` with `{ "command": ... }` on stdin and a 10s timeout —
  the same Rust evaluator contract used by the OpenCode plugin and the Claude
  `PreToolUse` hook.
- On `decision === "deny"` with a `reason`, the handler returns
  `{ block: true, reason }` (Pi's block-by-return contract; OpenCode blocks by
  throwing). The reason string comes from Rust
  `format_policy_block_message`, so it carries the policy ID and message.
- Fail-open: missing `sce` binary (ENOENT logs install guidance), timeout,
  non-zero exit, empty stdout, or invalid JSON all return without blocking.

## Implemented slice: conversation text capture

- `pi.on("message_end", ...)` captures completed messages, narrowed to
  `role === "user" | "assistant"` (skips `toolResult` and custom messages).
- Pi 0.80.6 exposes no per-message IDs on `message_end`; the handler uses
  `AssistantMessage.responseId` when present, otherwise `randomUUID()`. The
  parent `message` item and its `message.part` items ship in one mixed batch,
  so no cross-event ID mapping is needed.
- Part extraction: `TextContent.text` → `part_type: "text"`,
  `ThinkingContent.thinking` → `part_type: "reasoning"`; string user content
  becomes a single text part; empty text is skipped.
- Batches are piped to `sce hooks conversation-trace` (same normalized mixed
  `message` / `message.part` envelope as the OpenCode agent-trace plugin),
  keyed by `ctx.sessionManager.getSessionId()` with `cwd` from `ctx.cwd`.
- Fail-open fire-and-forget spawn: stdio `["pipe", "ignore", "ignore"]`,
  ENOENT logs install guidance, the promise resolves on every outcome and is
  not awaited by the handler.

## Implemented slice: edit/write diff capture

- `pi.on("tool_call", ...)` narrowed to `edit`/`write` records a pending
  mutation keyed by `toolCallId`: absolute target path (resolved against
  `ctx.cwd`), a repo-relative diff label (absolute when outside `cwd`), and
  prior file contents (`undefined` when the file does not exist yet).
- `pi.on("tool_result", ...)` looks up and deletes the pending entry first
  (cleanup on every result, including errors and duplicates), skips
  `isError` results, then re-reads the file; missing post-contents or
  unchanged contents are no-ops.
- Unified diffs come from writing before/after contents to `mkdtemp` temp
  files and spawning `git diff --no-index --no-ext-diff` — no npm
  dependencies. Only exit status 1 ("files differ") is accepted; the temp dir
  is always removed in `finally`. Header labels (`diff --git`, `---`, `+++`)
  are rewritten to the diff label only before the first `@@` marker so
  content lines are never touched; file creation rewrites to `--- /dev/null`.
- Each diff is emitted twice, both fire-and-forget fail-open spawns:
  - `sce hooks conversation-trace`: a mixed batch with a synthetic assistant
    `message` (`message_id` = `${toolCallId}-patch`) plus one
    `part_type: "patch"` part, mirroring the OpenCode patch-batch shape.
  - `sce hooks diff-trace`: normalized `{ sessionID, diff, time, model_id,
    tool_name: "pi", tool_version }` where `model_id` is
    `${ctx.model.provider}/${ctx.model.id}` or null, and `tool_version` is
    resolved from the installed Pi package (walking up from the resolved
    entry point, since `package.json` is not in Pi's `exports` map), nullable.

## Verification

- `nix develop -c ./config/pkl/check-generated.sh` covers
  `config/.pi/extensions` in its parity paths.
- Regeneration must be diff-clean; edit the `config/lib/pi-plugin/` source,
  never the generated copy.

## Rust-side session provenance

Rust `diff-trace` intake prefixes stored `diff_traces.session_id` values for
`tool_name: "pi"` payloads with `pi_` (idempotent for already-prefixed IDs),
via the `"pi"` arm in `prefixed_diff_trace_session_id()`
(`cli/src/services/hooks/mod.rs`). Unknown tool names still pass through
unprefixed.

## Asset pipeline, install, and doctor coverage

- `scripts/prepare-cli-generated-assets.sh` copies the whole `config/.pi` tree
  (including `extensions/sce/index.ts`) into
  `cli/assets/generated/config/pi/`; `cli/build.rs` embeds that tree wholesale
  as `PI_EMBEDDED_ASSETS`, so `sce setup --pi` installs the extension to
  repo-root `.pi/extensions/sce/index.ts` with no per-asset enumeration.
- `sce doctor` buckets embedded Pi assets under `extensions/` into a
  `Pi extensions` integration group (`collect_pi_integration_groups()` in
  `cli/src/services/doctor/inspect.rs`, `pi_asset::EXTENSIONS_DIR`), reporting
  present/missing/content-mismatch through the existing Pi integration problem
  kinds.

## Deferred non-goals

User-shell `!`/`!!` policy enforcement and bash-mutation diff tracing are
deferred (see `context/plans/pi-extension-sce-integration.md`).

See also: [generated-opencode-plugin-registration.md](generated-opencode-plugin-registration.md),
[bash-tool-policy-enforcement-contract.md](bash-tool-policy-enforcement-contract.md)
