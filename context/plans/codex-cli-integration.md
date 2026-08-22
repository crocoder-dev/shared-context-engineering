# Plan: codex-cli-integration

## Change summary

Adds Codex CLI as a fourth first-class SCE integration target alongside OpenCode, Claude Code, and Pi. This extends existing behavior rather than replacing it: Codex reuses the same canonical Pkl workflow catalog, the same Rust Bash policy engine, the same conversation (`messages`/`parts`) persistence, and the same `diff_traces` → post-commit intersection → `agent_traces` pipeline every other integration already goes through. The only new runtime surface is a Codex-specific hook adapter (`sce hooks codex`) that produces normalized evidence for Codex's two output roots (`.agents/` for skills, `.codex/` for hooks) and a transient before/after snapshot mechanism for `apply_patch` attribution. No Agent Trace schema migration is introduced, and Bash-triggered filesystem mutations are explicitly out of scope for attribution in this change — Bash gets policy enforcement only, matching the current-state boundary already documented for Claude/Pi.

## Acceptance criteria

- [ ] AC1: `sce setup --codex --non-interactive` succeeds in a Git repository, installs `.agents/skills/**` and `.codex/hooks.json` + `.codex/hooks/**`, and persists `{"integrations": {"target": ["codex"]}}` into `.sce/config.json` under existing merge semantics.
  - Validate: run the command in a scratch git repo; inspect `.sce/config.json` and installed files.
- [ ] AC2: `sce setup --all --non-interactive` installs Codex assets alongside OpenCode/Claude/Pi with no regression to the other three targets.
  - Validate: run in a scratch git repo; inspect all four target trees plus `integrations.target`.
- [ ] AC3: Core workflows (`sce-change-to-plan`, `sce-next-task`, `sce-validate`, `sce-commit`, `sce-handover`) appear under `.agents/skills/`, and optional workflows (`brownfield`) obey the existing `integrations.optional_workflows` selection mechanism for Codex the same way they do for OpenCode/Claude/Pi.
  - Validate: `nix run .#pkl-generate -- "$(mktemp -d)"` then inspect `.agents/skills/`; `sce setup --codex --workflow brownfield --non-interactive` includes `sce-brownfield`, a run without `--workflow` does not.
- [ ] AC4: `.codex/hooks.json` registers exactly `UserPromptSubmit`, `Stop`, `PreToolUse` (`Bash`, `apply_patch`), and `PostToolUse` (`apply_patch`) — no Bash `PostToolUse` entry.
  - Validate: inspect generated `.codex/hooks.json` content directly.
- [ ] AC5: A Codex `UserPromptSubmit` event produces exactly one user `message` and one text `part` under session `cx_<session>`.
  - Validate: integration test feeding a synthetic `UserPromptSubmit` payload to `sce hooks codex` and querying the repository Agent Trace DB.
- [ ] AC6: A Codex `Stop` event produces exactly one assistant `message` and one text `part`.
  - Validate: integration test feeding a synthetic `Stop` payload to `sce hooks codex` and querying the DB.
- [ ] AC7: Reprocessing the same turn's `UserPromptSubmit`/`Stop` event does not create a duplicate parent message.
  - Validate: integration test invoking the same payload twice and asserting one row per deterministic message ID.
- [ ] AC8: An allowed Bash command executes with no model-visible SCE tracing output.
  - Validate: integration test asserting empty/silent success output for an allowed command through `sce hooks codex` `PreToolUse` `Bash`.
- [ ] AC9: A denied Bash command is blocked using Codex's native `PreToolUse` deny response shape and includes the SCE policy reason text.
  - Validate: integration test asserting the deny response body/shape and policy reason for a configured blocking policy.
- [ ] AC10: Bash filesystem mutations create no Codex `diff_trace`.
  - Validate: regression test running `echo generated > generated.txt` through the Codex Bash hook path and asserting zero new `diff_traces` rows.
- [ ] AC11: A successful `apply_patch` produces an observed unified patch in `diff_traces` reflecting the actual before/after repository delta, not the requested patch text.
  - Validate: integration test driving `PreToolUse apply_patch` then `PostToolUse apply_patch` against a scratch repo and asserting the persisted patch matches `git diff` of the real file mutation.
- [ ] AC12: The persisted `diff_traces` row carries `session_id = cx_...`, `model_id = openai/...`, `tool_name = codex`, `payload_type = patch`.
  - Validate: same integration test as AC11, asserting row field values.
- [ ] AC13: The same successful `apply_patch` also creates assistant patch conversation evidence (`message` + `part_type = patch`) tied to the same `cx_` session.
  - Validate: same integration test as AC11, querying `messages`/`parts`.
- [ ] AC14: Given a pre-existing dirty worktree change `A` before `PreToolUse apply_patch` and a Codex-authored change `B`, the resulting Codex diff evidence contains `B` but not `A`.
  - Validate: integration test seeding an uncommitted dirty change before the hook sequence and asserting the persisted patch excludes it.
- [ ] AC15: A `PostToolUse apply_patch` with no corresponding pending before-state logs a diagnostic, fails open, and creates no diff evidence.
  - Validate: integration test invoking `PostToolUse apply_patch` without a prior `PreToolUse apply_patch` for the same correlation key.
- [ ] AC16: Identical before/after repository states produce no diff trace and are treated as a successful no-op.
  - Validate: integration test running the full pending → finalize sequence with no actual file change.
- [ ] AC17: A commit containing a recorded Codex `apply_patch` diff_trace is attributed through the existing, unmodified `post-commit` intersection pipeline.
  - Validate: integration test recording a Codex diff_trace, committing the same change, running `sce hooks post-commit`, and inspecting `post_commit_patch_intersections`.
- [ ] AC18: The resulting Agent Trace identifies Codex as the tool and preserves the Codex model ID through the existing attribution machinery.
  - Validate: same integration test as AC17, asserting the built `agent_traces.trace_json` contributor/tool metadata.
- [ ] AC19: No Agent Trace repository schema migration is added; `diff_traces`/`agent_traces`/`messages`/`parts` and `RepositoryAgentTraceDbSpec::migrations()` remain unchanged.
  - Validate: `git diff` shows no new file under `cli/migrations/agent-trace-repository/` and no changed baseline SQL.
- [ ] AC20: Existing OpenCode, Claude, and Pi setup, generated assets, conversation tracing, diff tracing, policy behavior, and Agent Trace tests continue to pass.
  - Validate: `nix flake check`.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/context-map.md`, `context/overview.md`, `context/architecture.md` — Codex named as a fourth supported integration target wherever the OpenCode/Claude/Pi target set is currently stated.
- `context/cli/cli-command-surface.md` — `sce setup --codex`, `sce hooks codex` command-surface additions.
- `context/cli/config-precedence-contract.md` — `integrations.target` accepting `"codex"`.
- `context/cli/default-path-catalog.md` — the new transient Codex `apply_patch` pending-state path helper.
- New `context/sce/codex-integration-runtime.md` (modeled on `context/sce/pi-extension-runtime.md`) — `cx_` session prefix, `openai/` model normalization, UserPromptSubmit/Stop mapping, Bash policy delegation, `apply_patch` before/after attribution flow, explicit Bash-mutation-tracing non-goal.
- `context/sce/doctor-human-text-contract.md` — Codex integration group/area ordering.
- `context/sce/agent-trace-hooks-command-routing.md` and `context/sce/agent-trace-db.md` — note that `sce hooks codex` is a second writer into the same `diff_traces`/`messages`/`parts` tables via the existing insert helpers, with no new adapter.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside
  the status. Never infer `synced` from conversation history; write every lifecycle
  transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/setup/`, `cli/src/services/config/`, `cli/src/services/hooks/`, `cli/src/services/doctor/`, `cli/src/services/default_paths.rs`, `cli/build.rs`, `config/pkl/base/`, `config/pkl/renderers/`, a new `config/codex-target/` build-time asset source, and the durable context files listed under Context sync.
- **Out of scope:** any change to `cli/migrations/agent-trace-repository/`; any change to OpenCode/Claude/Pi's own generated behavior beyond what is mechanically required to add a fourth target to shared enums/renderers; Codex App Server or `codex exec --json` integration; MCP-tool or subagent attribution; `AGENTS.md` generation/management; a Codex slash-command compatibility layer.
- **Constraints:** reuse `cli/src/services/bash_policy.rs` for Bash policy evaluation without reimplementing matching; reuse `DiffTraceInsert`/`insert_diff_trace`, `InsertMessageInsert`/`insert_messages`, `InsertPartInsert`/`insert_parts` for persistence without a Codex-specific DB adapter; reuse `cli/src/services/patch.rs` for unified-diff parsing/`git diff` output, no second diff engine.
- **Non-goal:** Bash-created filesystem change attribution for Codex, Claude, or Pi (deferred — tracked as a known gap, not solved here); a generic cross-producer mutation tracker; any `diff_traces`/Agent Trace DB schema column for snapshot/pending state.

## Assumptions

- The Codex hook lifecycle event names and field names given in the change request (`hook_event_name`, `session_id`, `turn_id`, `cwd`, `model`, `tool_name`, `tool_use_id`, `tool_input`, `tool_response`; events `UserPromptSubmit`, `Stop`, `PreToolUse`, `PostToolUse`; tool identifiers `Bash`, `apply_patch`) are taken as the working contract for T06. T06 begins by checking these against current Codex CLI documentation/behavior per the change request's own instruction ("Check the current official/current Codex hook schema rather than relying on old assumptions"); if reality differs, the typed parser is adjusted to match without changing the architecture, dispatcher shape, or any acceptance criterion above (all of which are stated as SCE-side observable outcomes, not exact Codex wire-format assertions).
- Codex's `PreToolUse` deny response shape is whatever the installed Codex CLI currently expects for a blocking tool-call response; T09 confirms and reuses that shape rather than inventing one, consistent with the existing OpenCode/Pi/Claude precedent of matching each harness's native block contract (see `context/sce/pi-extension-runtime.md`, `context/sce/bash-tool-policy-enforcement-contract.md`).
- The transient `apply_patch` pending-state directory lives under the existing SCE per-user state root (`cli/src/services/default_paths.rs`), not under the repository working tree, consistent with how checkout identity (`<git-dir>/sce/checkout-id`) and Agent Trace DBs are already scoped outside the tracked worktree.
- "SCE state namespace" for the pending-state path is a new named accessor added to `default_paths.rs` (per repo convention: "Production CLI code should define named path accessors ... not introduce new hardcoded path owners elsewhere"), not an ad hoc path literal inside the hooks module.

## Task stack

- [x] T01: `Add AgentProducer identity, cx_ session prefixing, and openai/ model normalization` (status:done)
  - Task ID: T01
  - Scope: In — a shared `AgentProducer` enum (`OpenCode`, `Claude`, `Pi`, `Codex`) if useful for explicit producer identity; extend the existing tool-prefixed session-ID helper (`prefixed_diff_trace_session_id` and its conversation-trace analog in `cli/src/services/hooks/mod.rs`) with an idempotent `"codex" -> cx_` arm; add an idempotent `openai/`-prefixing model-ID normalizer for Codex model IDs. Out — any hook parsing, any dispatcher, any CLI wiring.
  - Dependencies: none
  - Done when: unit tests prove `cx_` prefixing is idempotent (a `cx_`-prefixed input is unchanged) and does not affect `oc_`/`cc_`/`pi_` prefixing for other tool names; unit tests prove the model normalizer turns `gpt-5.6-codex` into `openai/gpt-5.6-codex` and leaves an already-prefixed `openai/gpt-5.6-codex` unchanged.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::'` (or the narrower module path the implementation lands in).
  - Completed: 2026-08-22
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Result: Added `DIFF_TRACE_CODEX_SESSION_ID_PREFIX` (`cx_`) and `CODEX_TOOL_NAME` (`"codex"`) constants with a `"codex"` arm in `prefixed_session_id` (backing both `prefixed_diff_trace_session_id` and `prefixed_conversation_trace_session_id`); added `OPENAI_MODEL_ID_PREFIX` (`"openai/"`) and `normalize_codex_model_id`, mirroring the existing `normalize_claude_model_id` pattern. No `AgentProducer` enum was introduced: the existing OpenCode/Claude/Pi code uses plain string constants and match arms with no producer enum anywhere in the module, so Codex follows the same pattern rather than adding an unused abstraction. `OPENAI_MODEL_ID_PREFIX` and `normalize_codex_model_id` carry `#[allow(dead_code)]` (existing repo precedent, e.g. `default_paths.rs`, `app.rs`) since dispatcher/CLI wiring is out of scope until T06+.
  - Verify: `nix flake check` (direct `cargo test hooks::` is blocked by this repo's Bash policy `use-nix-flake-check-over-cargo-test`, which requires `nix flake check` instead) — passed: `all checks passed!`, including `services::hooks::tests::prefixed_diff_trace_session_id_prefixes_fresh_codex_session_id`, `..._keeps_already_prefixed_codex_session_id`, `..._adding_codex_does_not_affect_other_tool_prefixes`, `normalize_codex_model_id_prefixes_fresh_model_id`, `normalize_codex_model_id_keeps_already_prefixed_model_id` (381 passed total), plus clippy and fmt.
  - Context impact: none — an internal helper addition inside `cli/src/services/hooks/mod.rs` with no dispatcher/CLI wiring yet (deferred to T06+); no user-visible behavior, public interface, or documented architecture changed.
  - Context synchronization: synced

- [x] T02: `Generate Codex workflow Skills into .agents/skills/` (status:done)
  - Task ID: T02
  - Scope: In — a Codex Pkl renderer (parallel to `opencode-content.pkl`/`claude-content.pkl`/`pi-content.pkl`) consuming the same `workflow-composite.pkl` composition and canonical `workflow-catalog.pkl`/workflow modules to emit `.agents/skills/{skill-slug}/SKILL.md` (and package-local references) for the five core workflows, honoring the existing optional-workflow catalog for `brownfield`; extend `config/pkl/generate.pkl` output mappings, `config/pkl/renderers/metadata-coverage-check.pkl`, and `config/pkl/renderers/generation-contract-check.pkl` for the new Codex artifact inventory (Codex adds no per-target frontmatter, matching Pi). Out — `.codex/` hook assets (T03), any Rust/CLI change, any `AGENTS.md` generation.
  - Dependencies: none
  - Done when: `nix run .#pkl-generate -- "$(mktemp -d)"` produces `.agents/skills/sce-change-to-plan/SKILL.md`, `.agents/skills/sce-next-task/SKILL.md`, `.agents/skills/sce-validate/SKILL.md`, `.agents/skills/sce-commit/SKILL.md`, `.agents/skills/sce-handover/SKILL.md` unconditionally, and `.agents/skills/sce-brownfield/SKILL.md` only when the catalog marks it selected for the run; `nix run .#pkl-check-generated` passes with the updated exact-path contract; no `.agents/commands/` output exists.
  - Verify: `nix run .#pkl-check-generated`.
  - Completed: 2026-08-22
  - Files changed: `config/pkl/renderers/codex-content.pkl` (new), `config/pkl/generate.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/renderers/generation-contract-check.pkl`
  - Result: Added `codex-content.pkl` mirroring `pi-content.pkl` exactly (empty extra-frontmatter, no `commands` mapping since Codex has no command dir), exposing only `skillDocuments` built from `workflowResults.skillDocuments.apply("")` plus `decision.skillDocuments.apply("")`. Wired its output into `generate.pkl` under `config/.agents/skills/`. Extended `metadata-coverage-check.pkl` with a `codex-skill-documents` exact-key inventory check (same `expectedSkillDocumentPaths` used for OpenCode/Claude/Pi) plus a forced-render coverage block; no command-route checks were added since Codex has no commands. Extended `generation-contract-check.pkl`: imported `codex-content.pkl`; folded its 26 documents into `expectedArtifactPaths` (bumping `expectedArtifactPathCount` 107 → 133) and `workflowDocuments`; added `.agents` to the `expectedDecisionDocumentPaths` and `assertPhaseReferenceContract` target lists; extended `assertTargetNeutralReferences` to also require the Codex reference body to match Pi/Claude/OpenCode when a Codex path exists, while preserving the original thrown diagnostic text unchanged (so `check-generated.sh`'s substring-matched negative fixture still passes) via a `containsKey` guard rather than an unconditional Codex comparison; bumped `assertHandoverContent`/`assertBrownfieldContent` expected document count 3 → 4. Verified generated output directly: `.agents/skills/**` contains exactly the five core `SKILL.md` files plus `sce-brownfield` and the internal `sce-decision` package, no `.agents/commands/` directory exists, and Codex's `sce-change-to-plan/SKILL.md` is byte-identical to Pi's (confirming no per-target frontmatter leaked in).
  - Verify: `nix run .#pkl-check-generated` — passed: "Ephemeral Pkl generation passed: 133 files, inventory sha256 c4d6ff1cf7f09e2f2b2236a9888de0cb4987700a5d36d767d0eeefdfc4266fb8." All `generation-contract-check.pkl` contract checks and `metadata-coverage-check.pkl` inventory checks evaluated successfully (both `pkl eval` directly and via the full `check-generated.sh` negative-fixture suite).
  - Context impact: root (revised from the initially reported `none` during synchronization — the root pass found the reported classification understated it). The shared canonical Pkl generation pipeline (`workflow-composite.pkl`/`decision-skill.pkl` composition, exact-path generation contract) now produces a fourth target, and the exact artifact-path count the contract enforces changed from 107 to 133, which several root context files stated as fact. `sce setup --codex`/`integrations.target` CLI wiring still lands in T04/T05.
  - Context synchronization: synced

- [x] T03: `Generate Codex hooks (.codex/hooks.json and hook helper script)` (status:done)
  - Task ID: T03
  - Scope: In — canonical Pkl source for `.codex/hooks.json` registering `UserPromptSubmit`, `Stop`, `PreToolUse` (`Bash`) — no `apply_patch` registration yet, no Bash `PostToolUse` entry, no `$schema`; `.codex/hooks/run-sce-or-show-install-guidance.sh` following the existing fail-open/install-guidance pattern used by `.claude/hooks/run-sce-or-show-install-guidance.sh`, routing all lifecycle JSON to `sce hooks codex`; extend `generate.pkl` output mappings and the generation-contract check for these two new paths. Out — the actual `sce hooks codex` Rust implementation (T06), CLI/build.rs embedding (T04), `apply_patch` hook registration and tracing (deferred to a later task).
  - Dependencies: T02
  - Done when: a temporary generation root contains `.codex/hooks.json` with exactly the three lifecycle registrations above (verified by direct content inspection) and `.codex/hooks/run-sce-or-show-install-guidance.sh` with the same missing-`sce` fail-open guidance text pattern as the Claude helper; `nix run .#pkl-check-generated` passes.
  - Verify: `nix run .#pkl-check-generated`; manual inspection of generated `.codex/hooks.json` and hook script content.
  - Completed: 2026-08-22
  - Files changed: `config/pkl/renderers/codex-content.pkl`, `config/pkl/generate.pkl`, `config/pkl/renderers/generation-contract-check.pkl`
  - Result: Added `hooksJson` and `sceHookScript` (`common.RenderedTextFile`) to `codex-content.pkl`, mirroring `claude-content.pkl`'s `settings`/`sceHookScript` pattern. Every Codex event/matcher entry routes to the single command `sce hooks codex`, matching T06's single-dispatcher scope. `PreToolUse` registers only `Bash`; there is no `PostToolUse` entry and no `apply_patch` registration — Codex `apply_patch` tracing is not yet implemented. The hook script invokes itself via a project-root-relative path (`.codex/hooks/run-sce-or-show-install-guidance.sh`) rather than an unconfirmed Codex-specific env var analog to `$CLAUDE_PROJECT_DIR` — no such env var is established anywhere in this repo, and no plan AC depends on the exact invocation mechanism. Wired both renders into `generate.pkl` under `config/.codex/hooks.json` and `config/.codex/hooks/run-sce-or-show-install-guidance.sh`. Added both paths to `generation-contract-check.pkl`'s `expectedArtifactPaths` and bumped `expectedArtifactPathCount` 133 → 135.
  - Verify: `nix run .#pkl-check-generated` — manual inspection of a fresh `nix run .#pkl-generate` temp-dir output confirmed `.codex/hooks.json` contains exactly the three lifecycle registrations (`UserPromptSubmit`, `Stop`, `PreToolUse` `Bash`), no `$schema`, validated as well-formed JSON via `jq`; `.codex/hooks/run-sce-or-show-install-guidance.sh` matched the Claude helper's fail-open guidance text verbatim (only the forwarded command differs).
  - Context impact: root — `context/patterns.md` and `context/overview.md` state the generation contract's exact artifact-path count as a literal fact (133), now stale at 135; `context/overview.md`'s Codex-renderer sentence also describes Codex output as "skills-only" with "no CLI setup/install wiring yet", which is now incomplete since `.codex/hooks.json`/hook-script generation is a second Codex asset kind in the pipeline (still with no CLI setup/install wiring — that remains T04/T05).
  - Context synchronization: synced

- [ ] T04: `Wire Codex's dual .agents/ + .codex/ output roots into embedded-asset install` (status:todo)
  - Task ID: T04
  - Scope: In — a `config/codex-target/` build-time source layout (`.agents/skills/**`, `.codex/hooks.json`, `.codex/hooks/**`); `cli/build.rs` `CODEX_EMBEDDED_ASSETS` generation from the Pkl-generated payload (parallel to `OPENCODE_EMBEDDED_ASSETS`/`CLAUDE_EMBEDDED_ASSETS`/`PI_EMBEDDED_ASSETS`); package-fallback preparation (`scripts/prepare-cli-generated-assets.sh` or equivalent) for the two new roots; the shared per-target install layout struct in `cli/src/services/setup/mod.rs` (around line 98) changed so `command_dir: Option<&'static str>` (Codex has no command dir — skills only), with existing OpenCode/Claude/Pi behavior unchanged (`Some(...)`); optional-workflow asset filtering adjusted to skip command-file exclusion when `command_dir` is `None`. Out — the `SetupTarget`/CLI-flag/config-schema plumbing that actually selects Codex for a run (T05).
  - Dependencies: T02, T03
  - Done when: an embedded-asset unit test proves `CODEX_EMBEDDED_ASSETS` contains normalized relative-path entries for every generated `.agents/skills/**` and `.codex/**` file with no `.agents/commands/**` entries; existing OpenCode/Claude/Pi embedded-asset tests still pass unmodified.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test setup::'`.
  - Context synchronization: pending

- [ ] T05: `Add Codex as a setup/integration target end-to-end` (status:todo)
  - Task ID: T05
  - Scope: In — `SetupTarget::Codex` in `cli/src/services/setup/mod.rs`; `IntegrationTargetId::Codex` in `cli/src/services/config/types.rs` (+ schema.rs mapping); `--codex` CLI flag, mutual-exclusion validation, non-interactive validation, help/error text, interactive setup choice, `--all` expansion to include Codex, install engine wiring to `CODEX_EMBEDDED_ASSETS`, `integrations.target` persistence accepting `"codex"`, and the Pkl-authored config JSON Schema (`sce-config-schema.pkl`) accepting `"codex"` in `integrations.target`. Out — doctor coverage (T13), hook runtime (T06+).
  - Dependencies: T04
  - Done when: `sce setup --codex --non-interactive` in a scratch git repo installs `.agents/skills/**` and `.codex/hooks.json` + `.codex/hooks/**` and records `{"integrations": {"target": ["codex"]}}`; `sce setup --all --non-interactive` includes Codex alongside OpenCode/Claude/Pi with no regression to the other three; `sce config validate` accepts a config file with `integrations.target: ["codex"]` and rejects an unknown target while listing `codex` among the valid values.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test setup:: config::'`; manual `sce setup --codex --non-interactive` run in a scratch repo per AC1/AC2.
  - Context synchronization: pending

- [ ] T06: `Implement sce hooks codex: typed event parsing and dispatcher skeleton` (status:todo)
  - Task ID: T06
  - Scope: In — `HookSubcommand::Codex` (or equivalent) wired into `cli/src/app.rs` / `cli/src/services/hooks/mod.rs` CLI parsing and help text; a typed, explicit Codex hook-event parser covering `hook_event_name`, `session_id`, `turn_id`, `cwd`, `model`, `tool_name`, `tool_use_id`, `tool_input`, `tool_response`; a dispatcher matching `UserPromptSubmit`, `Stop`, `PreToolUse(Bash)`, `PreToolUse(apply_patch)`, `PostToolUse(apply_patch)`, with every other event/tool combination falling through to a deterministic successful no-op; tracing/parse failures logged and fail-open (hook success, non-zero exit reserved for genuine parse-time CLI usage errors matching existing hook-command conventions). Out — the actual behavior behind each dispatch arm (T07–T12): this task's arms are stubs proven only by dispatch-routing tests.
  - Dependencies: T01
  - Done when: `sce hooks codex --help` and top-level `sce hooks --help` list the new subcommand; unit tests prove each of the five supported event/tool combinations routes to its own internal arm and every unsupported combination (e.g. an unknown `tool_name` under `PreToolUse`, or an unrecognized `hook_event_name`) routes to the no-op arm without error; a malformed/non-JSON STDIN payload is logged and returns hook success.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex'`.
  - Context synchronization: pending

- [ ] T07: `Capture Codex UserPromptSubmit into messages/parts` (status:todo)
  - Task ID: T07
  - Scope: In — the `UserPromptSubmit` dispatch arm: build `session_id = cx_<session_id>`, `message_id = cx:<turn_id>:user`, one `role="user"` message row via the existing `InsertMessageInsert`/`insert_messages` path, one `part_type="text"` part row (`text = prompt`) via `InsertPartInsert`/`insert_parts`, `generated_at_unix_ms` from hook receipt time. Out — `Stop` (T08), any new conversation table.
  - Dependencies: T06
  - Done when: an integration test feeding a synthetic `UserPromptSubmit` payload through `sce hooks codex` produces exactly one `messages` row and one `parts` row under session `cx_<session>` with the expected deterministic `message_id`; reprocessing the identical payload does not create a duplicate `messages` row (relies on the existing `ON CONFLICT (session_id, message_id) DO NOTHING` semantics).
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::user_prompt_submit'`.
  - Context synchronization: pending

- [ ] T08: `Capture Codex Stop into messages/parts` (status:todo)
  - Task ID: T08
  - Scope: In — the `Stop` dispatch arm: `session_id = cx_<session_id>`, `message_id = cx:<turn_id>:assistant`, one `role="assistant"` message row, one `part_type="text"` part row (`text = last_assistant_message`). Out — session-level model caching (explicitly not needed here).
  - Dependencies: T06
  - Done when: an integration test feeding a synthetic `Stop` payload through `sce hooks codex` produces exactly one `messages` row and one `parts` row under session `cx_<session>` with the expected deterministic `message_id`; reprocessing the identical payload does not create a duplicate `messages` row.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::stop'`.
  - Context synchronization: pending

- [ ] T09: `Route Codex Bash PreToolUse through the existing SCE Bash policy engine` (status:todo)
  - Task ID: T09
  - Scope: In — the `PreToolUse(Bash)` dispatch arm delegating the command string to `cli/src/services/bash_policy.rs` unchanged; on allow, silent hook success with no model-visible output; on deny, the Codex-native `PreToolUse` deny response shape carrying the SCE policy ID/message (matching the pattern in `context/sce/bash-tool-policy-enforcement-contract.md`'s "Block behavior contract"); no `diff_traces`/snapshot/pending-state writes on either branch. Out — `apply_patch` handling (T10/T11).
  - Dependencies: T06
  - Done when: an allowed Bash command produces silent success output; a command matching a configured blocking policy produces the deny response including the policy ID and message text; a regression test runs `echo generated > generated.txt` through the Codex Bash hook path end-to-end and asserts zero new `diff_traces` rows exist afterward.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::bash_policy'`.
  - Context synchronization: pending

- [ ] T10: `Capture apply_patch before-state via temporary-index snapshot` (status:todo)
  - Task ID: T10
  - Scope: In — a temporary-`GIT_INDEX_FILE` snapshot helper (`git read-tree HEAD` + `git add -A` + `git write-tree`) producing a `before_tree_oid` without mutating the real index; a new named path accessor in `cli/src/services/default_paths.rs` for the Codex pending-state directory (`<state-root>/sce/repos/<repository-id>/hooks/codex/pending/`); a hashed/sanitized event-key derivation from `(session_id, turn_id, tool_use_id)`; atomic pending-state file write (`{before_tree_oid, created_at_unix_ms}`) wired into the `PreToolUse(apply_patch)` dispatch arm. Out — the `PostToolUse` finalize logic (T11); no write to `agent-trace.db`.
  - Dependencies: T06
  - Done when: unit tests prove the event-key derivation is deterministic for the same triple and distinct for different triples, and is safe as a filesystem path segment; an integration test runs `PreToolUse(apply_patch)` against a scratch repo with a pre-existing dirty (uncommitted) change and asserts the written pending file's `before_tree_oid` reflects the dirty worktree state (tracked changes + non-ignored untracked files) rather than `HEAD`.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::apply_patch::pre'`.
  - Context synchronization: pending

- [ ] T11: `Finalize apply_patch: after-state, observed diff, cleanup` (status:todo)
  - Task ID: T11
  - Scope: In — the `PostToolUse(apply_patch)` dispatch arm: look up the pending file by the same event-key derivation; on hit, take a second temporary-index snapshot for `after_tree_oid`, compute `git diff --binary --find-renames <before_tree_oid> <after_tree_oid>`; on empty diff, treat as a successful no-op; consume (remove) the pending file idempotently after processing (safe for a second/duplicate cleanup attempt); on missing or unusable pending state, log a diagnostic, fail open, and produce no diff evidence (no guessing from the raw patch command). Out — DB persistence of the resulting non-empty patch (T12).
  - Dependencies: T10
  - Done when: integration tests cover file creation, file edit, file deletion, and (if the underlying delta supports it) rename, each producing the expected `git diff` shape from the helper; a test covers a `PostToolUse` call with no matching pending file (logs + fails open, no evidence); a test covers a malformed/unreadable pending-state file (same fail-open behavior); a test covers before==after (no-op, pending file still consumed); a test proves a pre-existing dirty change present at `PreToolUse` time (change `A`) is excluded from the finalize-time diff when only change `B` was made by the tool call in between.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::apply_patch::post'`.
  - Context synchronization: pending

- [ ] T12: `Persist Codex apply_patch diff evidence, patch conversation, and prove post-commit reuse` (status:todo)
  - Task ID: T12
  - Scope: In — for a non-empty finalize-time delta from T11: `DiffTraceInsert` with `time_ms` (finalization time), `session_id = cx_<session>`, `patch` (the observed diff), `model_id = openai/<model>` (via T01's normalizer), `tool_name = "codex"`, `tool_version = NULL`, `payload_type = "patch"`, persisted through the existing `insert_diff_trace()`; one assistant patch message/part (`message_id = cx:<turn_id>:<tool_use_id>:patch`, `role = assistant`, `part_type = patch`) via the existing `InsertMessageInsert`/`InsertPartInsert` path, tied to the same `cx_` session; an integration test proving the existing, unmodified `post-commit` intersection pipeline (`recent_diff_trace_patches` → `combine_patches` → `intersect_patches` → `build_agent_trace`) attributes a committed Codex change correctly and preserves `tool_name="codex"`/the Codex model ID in the resulting `agent_traces.trace_json`. Out — any new persistence path, any Codex-specific DB adapter, any post-commit code change.
  - Dependencies: T11, T01
  - Done when: the diff evidence and conversation evidence tests above pass; the post-commit integration test (recording a Codex diff_trace, committing the same change, running `sce hooks post-commit`, then inspecting the persisted `agent_traces` row) passes with no modification to `cli/src/services/hooks/mod.rs`'s existing post-commit flow functions beyond what T06–T11 already required.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test hooks::codex::apply_patch::persist hooks::post_commit'`.
  - Context synchronization: pending

- [ ] T13: `Add Codex doctor coverage` (status:todo)
  - Task ID: T13
  - Scope: In — a Codex integration group in `cli/src/services/doctor/inspect.rs` (parallel to the Claude/OpenCode/Pi groups) reporting missing/mismatched `.agents/skills/**` for the resolved workflow selection, missing/mismatched `.codex/hooks.json`, and missing/mismatched `.codex/hooks/run-sce-or-show-install-guidance.sh`; actionable guidance text for Codex's project hook trust/review requirement (informational only — doctor does not bypass or grant trust); Codex added to the doctor target-resolution set (`integrations.target` entries / repo-root `.codex/` detection) and to `context/sce/doctor-human-text-contract.md`'s target/area ordering. Out — any change to doctor's fix-mode git-hook repair logic (unrelated to Codex).
  - Dependencies: T04, T05
  - Done when: `sce doctor` in a repo with Codex installed and current reports `[PASS]` for the Codex integration group; deleting or corrupting a Codex asset produces the matching `[FAIL]`/`[MISS]` problem with actionable text; `sce doctor --format json` includes a Codex integration group entry alongside `opencode`/`claude`/`pi`.
  - Verify: `nix develop -c sh -c 'cd cli && cargo test doctor::'`; manual `sce doctor` / `sce doctor --format json` run against a Codex-installed scratch repo.
  - Context synchronization: pending

## Open questions

- The exact current Codex CLI hook JSON schema (event names, field names, tool-call identifiers, and the native `PreToolUse` deny response shape) cannot be verified from this repository — Codex CLI is an external, evolving tool. T06 and T09 open by checking the change request's assumed schema against current Codex CLI behavior/documentation before finalizing the parser and deny-response builder; this is recorded as an assumption above rather than a blocking question because no acceptance criterion in this plan depends on the exact wire format — every AC is stated as an SCE-side observable outcome (DB rows, generated files, policy behavior) that holds regardless of the precise Codex JSON shape.
