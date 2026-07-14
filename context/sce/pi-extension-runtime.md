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

## Verification

- `nix develop -c ./config/pkl/check-generated.sh` covers
  `config/.pi/extensions` in its parity paths.
- Regeneration must be diff-clean; edit the `config/lib/pi-plugin/` source,
  never the generated copy.

## Planned extensions (not yet implemented)

Edit/write diff capture (`sce hooks diff-trace`, `tool_name: "pi"`), `pi_`
session-ID prefixing in Rust, asset sync/embedding, and doctor coverage are
tracked in `context/plans/pi-extension-sce-integration.md` (T03–T07).
Deferred non-goals: user-shell `!`/`!!` policy enforcement and bash-mutation
diff tracing.

See also: [generated-opencode-plugin-registration.md](generated-opencode-plugin-registration.md),
[bash-tool-policy-enforcement-contract.md](bash-tool-policy-enforcement-contract.md)
