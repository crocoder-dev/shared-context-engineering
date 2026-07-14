# Plan: pi-extension-sce-integration

## Change summary

Extend the completed Pi harness integration (`pi-harness-integration.md`) with a project-local Pi extension that wires Pi into SCE's two runtime systems: bash policy enforcement and Agent Trace capture. A single combined extension source lives at `config/lib/pi-plugin/sce-pi-extension.ts`, is emitted verbatim by Pkl into `config/.pi/extensions/sce/index.ts` (mirroring how `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` is emitted into `config/.opencode/plugins/sce-agent-trace.ts` via `config/pkl/generate.pkl`), synced into `cli/assets/generated/config/pi/`, embedded, and installed by the existing `sce setup --pi` whole-tree copy.

The extension implements three adapters, all fail-open and all delegating persistence/validation to the Rust CLI:

1. **Bash policy**: Pi `tool_call` event narrowed to `bash`, evaluated via `sce policy bash --input normalized --output json`, returning `{ block: true, reason }` on `decision === "deny"` with policy ID and message.
2. **Conversation trace**: Pi `message_end` events mapped to the mixed `message` / `message.part` envelope (`text`, `reasoning` parts) piped to `sce hooks conversation-trace`.
3. **Edit/write diff trace**: pre/post file snapshots around Pi `edit`/`write` tool calls; unified diffs produced by spawning `git diff --no-index --no-ext-diff` on temp files (no npm dependencies); emitted as a conversation `patch` part and as a normalized `sce hooks diff-trace` payload with `tool_name: "pi"`.

On the Rust side, `prefixed_diff_trace_session_id()` (`cli/src/services/hooks/mod.rs:44`) gains a `"pi" => "pi_"` arm so Pi sessions carry distinct provenance instead of raw passthrough.

Decisions resolved with the user (2026-07-13):
- Unified diffs via `git diff --no-index` on temp files — no npm dependencies, single-file extension.
- User-shell `!`/`!!` policy enforcement and bash-mutation diff tracing (repo snapshots around bash calls) are excluded; recorded as non-goals for a possible follow-up plan.
- Rust prefix change adds `pi_` only; unknown `tool_name` passthrough behavior is unchanged (no generic `tool_` fallback).

## Success criteria

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` deterministically emits `config/.pi/extensions/sce/index.ts`; a clean re-run produces no diff; `nix develop -c ./config/pkl/check-generated.sh` covers the new path and passes.
- `bash scripts/prepare-cli-generated-assets.sh` syncs the extension into `cli/assets/generated/config/pi/extensions/sce/index.ts`, and `sce setup --pi --non-interactive` in a scratch repo installs it to `.pi/extensions/sce/index.ts` without disturbing other `.pi/` content.
- With a deny rule configured (e.g. `forbid-git-commit`), an agent bash tool call to `git commit` in a Pi session is blocked before subprocess execution, and the block reason contains the policy ID and policy message; allowed commands run unchanged; missing `sce`, timeout, non-zero exit, and invalid JSON all fail open.
- Pi user and assistant messages produce `message` and `message.part` rows via `sce hooks conversation-trace` (text and reasoning preserved as separate parts), keyed by the Pi session ID.
- Pi `edit`/`write` tool calls produce unified diffs recorded both as conversation `patch` parts and as `diff_traces` rows with `tool_name: "pi"`, `model_id` in `provider/model` form, nullable `tool_version`, and session IDs prefixed `pi_`.
- Rust unit tests cover `pi_` prefixing (fresh and already-prefixed IDs) and Pi normalized payload ingestion; post-commit intersection preserves Pi provenance for overlapping diffs.
- `sce doctor` reports the Pi extension asset as part of Pi integration health (present/missing/drifted).
- `nix flake check` passes (canonical replacement for direct `cargo test` per repo bash policy).

## Constraints and non-goals

- No Pi core changes; integration is entirely a project-local extension plus SCE-side support.
- No policy enforcement for Pi user-shell `!`/`!!` commands (`user_bash` event) — deferred.
- No bash-mutation diff tracing (git snapshots around bash tool calls) — deferred; attribution for bash-based edits is knowingly incomplete in this release, matching Claude's structured-patch scope.
- No npm dependencies or `node_modules` inside `.pi/extensions/`; the extension is one self-contained TypeScript file (type-only imports from `@earendil-works/pi-coding-agent` are allowed).
- No direct Agent Trace database writes from the extension; all persistence goes through `sce hooks ...` / `sce policy bash`.
- No change to unknown-`tool_name` session-ID passthrough; no `tool_` generic prefix.
- Do not hand-edit generated artifacts (`config/.pi/**`, `cli/assets/generated/**`); all content changes go through `config/lib/pi-plugin/` and Pkl.
- Do not change existing OpenCode/Claude plugin behavior or generated outputs.

## Assumptions

- Pi auto-discovers `.pi/extensions/*/index.ts` in a trusted project (per the source write-up) with no registration step.
- Pi's `tool_call` handler can block by returning `{ block: true, reason }`, and `ctx.sessionManager.getSessionId()` is stable across a session.
- `sce hooks conversation-trace` and `sce hooks diff-trace` accept the normalized (non-Claude) payload shapes exactly as the OpenCode plugin emits them; Pi reuses those shapes with `tool_name: "pi"`.
- Pi tool version is sourced by resolving `@earendil-works/pi-coding-agent/package.json` at runtime, falling back to `null` (normalized diff traces permit nullable `tool_version`).

## Task stack

- [x] T01: `Add Pi extension with bash policy enforcement, emitted via Pkl` (status:done, completed 2026-07-14)
  - Task ID: T01
  - Files changed: `config/lib/pi-plugin/sce-pi-extension.ts` (new), `config/lib/package.json` + `config/lib/bun.lock` (added `@earendil-works/pi-coding-agent@0.80.6`, type-only), `config/lib/tsconfig.json` (include `pi-plugin/**/*.ts`), `config/pkl/generate.pkl` (verbatim emission), `config/pkl/check-generated.sh` (`config/.pi/extensions` path), `config/.pi/extensions/sce/index.ts` (generated).
  - Evidence: `pkl eval` emits `config/.pi/extensions/sce/index.ts` identical to source (`diff` clean, re-run diff-clean); `nix develop -c ./config/pkl/check-generated.sh` passes; `tsc --noEmit` reports zero pi-plugin errors (43 pre-existing bash-policy test errors unchanged); biome check/format clean; `bun test` 12/12 pass.
  - Notes: Pi 0.80.6 API reconciled — default-export `ExtensionFactory`, `pi.on("tool_call")` with `ToolCallEventResult { block, reason }`, `isToolCallEventType("bash", ...)` all match plan assumptions. Deny reason passes through Rust `format_policy_block_message` (`Blocked by SCE bash-tool policy '{id}': {message}`), so policy ID + message are included. All failure paths (ENOENT/timeout/non-zero exit/empty/invalid JSON) return null → fail-open.
  - Goal: Create `config/lib/pi-plugin/sce-pi-extension.ts` implementing the bash policy adapter, and wire Pkl to emit it verbatim as `config/.pi/extensions/sce/index.ts`.
  - Boundaries (in/out of scope): In — extension skeleton (default export receiving `ExtensionAPI`), `tool_call` handler narrowed to `bash` via `isToolCallEventType`, synchronous `sce policy bash --input normalized --output json` invocation with `{ "command": ... }` on stdin, `{ block: true, reason }` on deny including policy ID and policy message, fail-open on missing CLI / timeout (10s) / non-zero exit / empty or invalid JSON; Pkl emission wiring in `config/pkl/generate.pkl` (and the pi renderer/base files as needed, following the `config/lib` → `config/.opencode/plugins` pattern in `config/pkl/base/opencode.pkl`); extend `config/pkl/check-generated.sh` paths to cover `config/.pi/extensions`. Out — conversation/diff tracing (T02/T03), asset sync (`cli/assets/generated`), doctor, docs.
  - Done when: `nix develop -c pkl eval -m . config/pkl/generate.pkl` writes `config/.pi/extensions/sce/index.ts` identical to the `config/lib/pi-plugin` source; re-run is diff-clean; `nix develop -c ./config/pkl/check-generated.sh` passes; the extension type-checks under `config/lib` tooling (`bun`/`tsc` per `config/lib/tsconfig.json`).
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl && git status --short config/.pi`; `diff config/lib/pi-plugin/sce-pi-extension.ts config/.pi/extensions/sce/index.ts`; `nix develop -c ./config/pkl/check-generated.sh`; type-check in `config/lib`; manual read confirming deny path surfaces `policy_id` + `reason` and all failure paths return without blocking.

- [x] T02: `Add conversation text capture to the Pi extension` (status:done, completed 2026-07-14)
  - Task ID: T02
  - Files changed: `config/lib/pi-plugin/sce-pi-extension.ts` (message_end handler + conversation-trace spawn helper), `config/.pi/extensions/sce/index.ts` (regenerated).
  - Evidence: `pkl eval` regeneration diff-clean; `nix develop -c ./config/pkl/check-generated.sh` passes; `tsc --noEmit` zero pi-plugin errors (43 pre-existing baseline unchanged); biome clean; `bun test` 12/12 pass. Live Pi scratch smoke not run this session (deferred to T07 end-to-end smoke).
  - Notes: Pi 0.80.6 exposes no per-message IDs on `message_end` (`AgentMessage` union lacks `id`); handler uses `AssistantMessage.responseId` when present, else `randomUUID()` — message + parts ship in one mixed batch so no cross-event mapping is needed. Roles narrowed to `user`/`assistant` (skips `toolResult`/custom messages). `text` parts from `TextContent.text`, `reasoning` parts from `ThinkingContent.thinking`, empty text skipped. Spawn helper mirrors OpenCode adapter fail-open shape (stdio pipe/ignore/ignore, ENOENT warn, resolve-only promise, fire-and-forget), cwd from `ctx.cwd`.
  - Goal: Capture completed Pi user/assistant messages and pipe them to `sce hooks conversation-trace` as mixed `message` + `message.part` batches.
  - Boundaries (in/out of scope): In — `message_end` handler in `config/lib/pi-plugin/sce-pi-extension.ts`; session ID from `ctx.sessionManager.getSessionId()`; preserve Pi message IDs when exposed, otherwise generate one and keep an event-local mapping so parts share it; emit `text` and `reasoning` parts separately; async fire-and-forget `spawn("sce", ["hooks", "conversation-trace"])` helper writing JSON to stdin with stderr ignored and no unhandled rejections (mirror the OpenCode adapter's fail-open behavior); Pkl regeneration of the emitted copy. Out — patch parts (T03), diff traces (T03), Rust changes.
  - Done when: In a Pi session against a scratch repo, a user prompt and assistant reply produce `message` and `message.part` rows in the Agent Trace DB (verifiable via `sce` trace/db tooling); reasoning content lands as `part_type: "reasoning"`; missing `sce` binary causes no extension error; regeneration is diff-clean.
  - Verification notes (commands or checks): regenerate + `check-generated.sh`; scratch-repo smoke: run Pi with the extension installed, send a prompt, then inspect the trace DB (e.g. `sce` db shell/tables) for the new message rows keyed by the Pi session ID; temporarily rename `sce` off PATH and confirm the session continues cleanly.

- [x] T03: `Add edit/write diff capture to the Pi extension` (status:done, completed 2026-07-14)
  - Task ID: T03
  - Files changed: `config/lib/pi-plugin/sce-pi-extension.ts` (edit/write tool_call capture, tool_result diff emission, diff-trace spawn helper, tool-version resolution), `config/.pi/extensions/sce/index.ts` (regenerated).
  - Evidence: `pkl eval` regeneration identical to source and diff-clean on re-run; `nix develop -c ./config/pkl/check-generated.sh` passes; `tsc --noEmit` zero pi-plugin errors (43 pre-existing bash-policy test baseline unchanged); biome check clean; `bun test` 12/12 pass; `git diff --no-index` exit-1/header-shape behavior verified in a temp-dir smoke. Live Pi scratch smoke deferred to T07.
  - Notes: Pending-call map keyed by `toolCallId`, entry deleted first on every `tool_result` (covers errors/duplicates). `before` read pre-execution (undefined when absent → `--- /dev/null` label); no-op when post-read fails or contents unchanged; `git diff --no-index --no-ext-diff` on `mkdtemp` files, only exit status 1 accepted, temp dir removed in `finally`. Header labels rewritten only before the first `@@` to avoid touching content lines. Patch part shipped as synthetic assistant message + `patch` part (`${toolCallId}-patch`), mirroring OpenCode. `model_id` = `${ctx.model.provider}/${ctx.model.id}` else null (Rust accepts null). `tool_version` resolved by walking up from the resolved package entry (package.json not in Pi's `exports` map), nullable.
  - Goal: Record unified diffs for Pi `edit` and `write` tool calls as conversation `patch` parts and normalized `diff-trace` payloads with `tool_name: "pi"`.
  - Boundaries (in/out of scope): In — `tool_call` handler for `edit`/`write` recording tool-call ID, resolved target path, and prior file contents (undefined when absent); `tool_result` handler that skips errored results, reads post-contents, and produces a unified diff by writing before/after contents to temp files and spawning `git diff --no-index --no-ext-diff` (empty diff → no-op; temp files always cleaned up; path labels rewritten to the repo-relative target path); emit `message.part` with `part_type: "patch"` to `conversation-trace` and the normalized `{ sessionID, diff, time, model_id, tool_name: "pi", tool_version }` payload to `diff-trace`; `model_id` as `${ctx.model.provider}/${ctx.model.id}` when available; `tool_version` resolved from the Pi package's `package.json`, nullable; pending-call map cleanup on every result including duplicates/failures. Out — bash-produced modifications, Rust prefixing (T04), multi-file tools.
  - Done when: In a scratch-repo Pi session, a `write` of a new file and an `edit` of an existing file each produce a `diff_traces` row with a valid unified diff, `tool_name: "pi"`, and correct `model_id`, plus a conversation `patch` part; failed edits and no-op writes produce no rows; regeneration is diff-clean.
  - Verification notes (commands or checks): regenerate + `check-generated.sh`; scratch smoke covering: new-file write, existing-file edit, edit emptying a file, failed edit, write producing identical content; inspect `diff_traces` rows for diff validity, `tool_name`, `model_id`, `tool_version`; confirm no temp files leak (`ls /tmp` pattern used by the helper).

- [ ] T04: `Add pi_ session prefix and Pi ingestion coverage in Rust` (status:todo)
  - Task ID: T04
  - Goal: Give Pi diff traces distinct provenance by prefixing normalized Pi session IDs with `pi_`.
  - Boundaries (in/out of scope): In — add `PI_TOOL_NAME` / `DIFF_TRACE_PI_SESSION_ID_PREFIX` ("pi_") and a `"pi"` match arm to `prefixed_diff_trace_session_id()` (`cli/src/services/hooks/mod.rs:38-56`); unit tests for fresh and already-prefixed Pi IDs; an ingestion test asserting a normalized payload with `tool_name: "pi"` persists a `diff_traces` row with the `pi_` session ID, `model_id`, and nullable `tool_version`; a post-commit intersection test (or extension of an existing one) confirming Pi provenance survives into the final Agent Trace. Out — unknown-`tool_name` fallback changes, conversation-trace session prefixing changes, extension code.
  - Done when: New tests pass; existing `oc_`/`cc_` behavior and unknown-tool passthrough are unchanged; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check` (repo policy blocks direct `cargo test`); review test assertions cover both prefix idempotency cases and end-to-end ingestion with `tool_name: "pi"`.

- [ ] T05: `Sync, embed, and doctor-check the Pi extension asset` (status:todo)
  - Task ID: T05
  - Goal: Ship the extension through the asset pipeline and surface its health in `sce doctor`.
  - Boundaries (in/out of scope): In — run `bash scripts/prepare-cli-generated-assets.sh` and commit `cli/assets/generated/config/pi/extensions/sce/index.ts` (the script already copies the whole `config/.pi` tree; extend it only if the copy misses the new subdirectory); verify `sce setup --pi` installs the extension via the existing embedded whole-tree deploy, adjusting setup asset enumeration only if needed; add a `Pi extensions` group (or extend existing Pi groups) in `collect_pi_integration_groups()` (`cli/src/services/doctor/inspect.rs`) so doctor reports the extension present/missing/drifted; update doctor tests. Out — new doctor problem categories, extension content changes, docs (T06).
  - Done when: `bash scripts/prepare-cli-generated-assets.sh && diff -r config/.pi cli/assets/generated/config/pi` is clean; scratch `sce setup --pi --non-interactive` installs `.pi/extensions/sce/index.ts`; `sce doctor` passes after setup and flags a deleted or modified extension file; `nix flake check` passes.
  - Verification notes (commands or checks): asset diff command above; scratch smoke: setup, `sce doctor` (PASS), delete `.pi/extensions/sce/index.ts`, `sce doctor` (reports the problem with `sce setup --pi` remediation); `nix flake check`.

- [ ] T06: `Document the Pi extension integration` (status:todo)
  - Task ID: T06
  - Goal: Update docs so Pi is described as covered by bash policy and Agent Trace, including scope limits.
  - Boundaries (in/out of scope): In — README and `context/architecture.md` sections describing integrations/policy/trace coverage; `config/pkl/README.md` ownership notes for `config/lib/pi-plugin` → `config/.pi/extensions`; note the deferred gaps (no `!`/`!!` policy, no bash-mutation tracing) where integration coverage is described. Out — code changes, Pi user tutorials.
  - Done when: Touched docs accurately state Pi extension coverage, the `pi_` session prefix, and the deferred gaps; no stale claims that Pi has no hooks/extensions remain in touched docs.
  - Verification notes (commands or checks): `grep -rn "No hooks for Pi\|extensions are out of scope\|only OpenCode" context/ README.md config/pkl/README.md` returns no stale claims; proofread rendered sections.

- [ ] T07: `Validation and cleanup` (status:todo)
  - Task ID: T07
  - Goal: Run the full verification suite, confirm the end-to-end Pi extension flow, remove scaffolding, and sync context.
  - Boundaries (in/out of scope): In — full checks, end-to-end scratch smoke (setup → policy deny → conversation/edit tracing → commit → post-commit trace with Pi provenance), removal of any temporary scaffolding, context sync verification, plan status updates and validation report. Out — new features.
  - Done when: `nix flake check` passes; `nix develop -c ./config/pkl/check-generated.sh` and `nix run .#pkl-check-generated` pass; clean regeneration + asset-prep re-run yields no git diff; scratch smoke succeeds end to end including a denied `git commit` bash call and a post-commit Agent Trace containing `tool_name: "pi"` with a `pi_` session ID; plan checkboxes and touched context docs reflect final state.
  - Verification notes (commands or checks): `nix flake check`; `nix develop -c sh -c 'pkl eval -m . config/pkl/generate.pkl && bash scripts/prepare-cli-generated-assets.sh' && git status --short`; `nix run .#pkl-check-generated`; scratch-repo smoke with isolated XDG roots; review `context/plans/pi-extension-sce-integration.md` statuses.

## Open questions

- None blocking. If Pi's actual extension API diverges from the write-up (event names, blocking contract, `ctx` shape), reconcile against the installed Pi package's types in T01 before finalizing the handler signatures.
