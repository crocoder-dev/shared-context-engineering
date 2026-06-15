# Commit-msg co-author trailer: opt-out default + AI-trace presence gate

## Change summary
Flip the canonical `Co-authored-by: SCE <sce@crocoder.dev>` trailer to **opt-out by default** on `sce hooks commit-msg`, AND keep the planned AI-trace presence gate as the always-on filter. After this change, the trailer is appended on every commit-msg invocation when:

1. The opt-out signal is NOT set (new default is "attribution on"; current `policies.attribution_hooks.enabled = false` default flips to `true`, and the env-var contract is reworked accordingly), AND
2. `SCE_DISABLED` is not truthy (unchanged master kill switch), AND
3. The Agent Trace DB shows at least one relevant AI-authored code change in scope (the gate originally planned).

When the AI-trace check finds no relevant AI change, the trailer is NOT appended even if attribution is enabled.

This change preserves the existing transformer surface (`apply_commit_msg_coauthor_policy`) but:
- Reverses the default of `policies.attribution_hooks.enabled` from `false` to `true`.
- Reworks the env-var semantics (`SCE_ATTRIBUTION_HOOKS_ENABLED` -> opt-out form; see Decisions below).
- Updates user-facing CLI help text at `cli/src/cli_schema.rs:32-33` ("Run attribution-only git hooks (disabled by default)") to reflect the new default.
- Folds the proposed `require_ai_trace` flag into the always-on default: with opt-out attribution, the AI-trace gate is the canonical behavior, no separate config key.
- Honors any existing explicit `enabled = false` in user config files as a backwards-compat opt-out signal (no silent flip for already-deployed configs).

## Decisions (resolved during planning)
- **Attribution default**: opt-out. `policies.attribution_hooks.enabled` default flips from `false` to `true`.
- **Env-var contract**: rename `SCE_ATTRIBUTION_HOOKS_ENABLED` -> `SCE_ATTRIBUTION_HOOKS_DISABLED` (opt-out semantics). Justification: matching name to default avoids the well-known "double negative" bug (`SCE_ATTRIBUTION_HOOKS_ENABLED=0` looks like opt-out but operators set it after copy/pasting the variable from docs that defaulted to opt-in). The new name makes the operator intent explicit at every call site, removes ambiguity in shell scripts, and aligns with `SCE_DISABLED` as the existing opt-out pattern. The flag still feeds the same `attribution_hooks_enabled` resolved value (inverted on read), so `ResolvedHookRuntimeConfig` and downstream gate logic do not change shape.
- **AI-trace `require_ai_trace` key**: dropped. With opt-out attribution, the AI-trace gate is the always-on filter; no dedicated key.
- **Backwards compat**: a user config file that explicitly sets `policies.attribution_hooks.enabled = false` MUST continue to suppress the trailer (interpreted as an explicit opt-out signal). Only the *default* changes; explicit values still win.
- **Query scope (revised during T03 review)**: `commit-msg` should perform a cheap preflight evidence check itself rather than asking `pre-commit` to pass state forward. The check should inspect the currently staged diff (`git diff --cached`) and compare it with already-captured AI/editor diff traces from AgentTraceDb using the existing patch combine/intersection primitives. The final Agent Trace payload is still calculated in `post-commit`, after the commit SHA exists; this preflight is only a boolean "does staged content overlap with AI/editor trace evidence?" gate for deciding whether to append the trailer. Because the preflight only needs a boolean, it should short-circuit at the first AI/editor conversation/trace row that produces a positive staged-diff intersection instead of combining all conversations before deciding.
- **No-evidence rule (resolved, unified fail posture)**: any of the following suppress the trailer — DB file missing, DB present but empty / no AI-attributed records, DB read error of any kind, query returns zero matches. User framing: *"if you can't produce evidence there is no SCE."* This is effectively fail-closed, but framed as "no evidence" rather than as an error-handling mode. Errors are still logged for diagnostics, but they never cause the trailer to be appended. There is no separate fail-open/fail-closed knob; do not add one.

## Success criteria
- With no config and no env override, `sce hooks commit-msg` appends the canonical trailer whenever the AI-trace check confirms an AI change is present in scope.
- With `SCE_ATTRIBUTION_HOOKS_DISABLED=1` (or `policies.attribution_hooks.enabled = false` in a config file), the trailer is never appended, regardless of AI-trace state.
- With `SCE_DISABLED=1`, the trailer is never appended (master kill switch behavior unchanged).
- When attribution is enabled (default or explicit) and the AI-trace check determines no AI change is present, the commit message is returned unchanged and no trailer is written.
- When the AI-trace DB is missing, unreadable, errors, or returns zero matches, the trailer is never appended; the commit message is returned unchanged regardless of attribution settings. Errors are logged but never escalate to applying the trailer.
- The policy entrypoint surface keeps a single transformer responsibility and remains unit-testable without touching the live Agent Trace DB.
- Hook runtime stays within commit-msg latency budget (cheap DB read, deterministic no-evidence-suppresses rule).
- CLI help text at `cli/src/cli_schema.rs:32-33` reflects the new "enabled by default; suppressible via SCE_ATTRIBUTION_HOOKS_DISABLED, SCE_DISABLED, or `policies.attribution_hooks.enabled = false`" reality.
- All new behavior is covered by unit tests; existing trailer-idempotency and gate semantics are preserved.
- The pure AI-overlap predicate used by the commit-msg evidence gate has golden fixture coverage for overlap, no-overlap, empty-input, and structured Claude-derived patch scenarios before runtime wiring depends on it.
- Context (`context/sce/agent-trace-commit-msg-coauthor-policy.md` and any related context-map entry) accurately reflects the new opt-out gating contract.

## Constraints and non-goals
- Constraints
  - Must reuse `AgentTraceDb::open_for_hooks_without_migrations` plus `ensure_schema_ready_for_hooks` — never run migrations on the commit-msg hot path.
  - DB read must respect the shared retry budget already enforced by `TursoDb` (see `context/sce/shared-turso-db.md`); no new retry policy.
  - No change to the trailer string, dedupe rules, idempotency rules, or trailing-newline preservation.
  - No changes to `policies.attribution_hooks.enabled` semantics for other hooks (post-commit, post-rewrite remain unaffected by the AI-trace gate; they only see the new default for the gate itself).
  - No new long-running shell-outs to `git`; staged-file inspection is explicitly out of scope for the resolved query (presence-only), but if a future iteration revisits path-overlap scoping it must reuse `run_git_command_capture_stdout` patterns already in the hooks module.
  - Explicit user config (`enabled = false` set in a `sce/config.json` file) MUST be respected as an opt-out signal after the default flip.
- Non-goals
  - Defining or persisting a new notion of "AI changes" beyond what `diff_traces` (and the related session/model attribution rows) already record.
  - Backfilling historical commits or rewriting `post-commit` patch intersection logic.
  - Changing how OpenCode/Claude plugins emit diff/session/model rows.
  - Surfacing the AI-trace check result to user-visible CLI output beyond the hook's existing `(policy_gate_passed=..., trailer_applied=...)` summary line.
  - Adding a new `require_ai_trace` config key (folded into always-on default).
  - Migrating user data or auto-rewriting existing config files; the default flip is purely a code-side default change.

## Open questions
None. All previously-open questions (query scope, fail posture, empty-DB first-commit case) are resolved in the Decisions block above. Plan is ready for T01 execution.

## Assumptions
- Env var is renamed to `SCE_ATTRIBUTION_HOOKS_DISABLED` with opt-out semantics; old name is NOT kept (one canonical contract).
- The on-disk `agent_trace_db` remains the source of captured AI/editor trace rows, but `commit-msg` evidence is scoped by overlap with the staged diff instead of mere row presence.
- The preflight helper is a single `bool` answer: "staged AI overlap found" or "no evidence" (with errors collapsed to "no evidence").

## Task stack

- [x] T01: `Flip attribution_hooks_enabled default to opt-out and rename env var` (status:done)
  - Task ID: T01
  - Goal: Change the resolver default for `attribution_hooks_enabled` from `false` to `true`, rename `SCE_ATTRIBUTION_HOOKS_ENABLED` -> `SCE_ATTRIBUTION_HOOKS_DISABLED` with inverted parse semantics, and update CLI help text to reflect "enabled by default". Explicit config-file `enabled = false` MUST still suppress the trailer.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/config/resolver.rs:428-447` default + env-var read flip, `cli/src/services/config/types.rs:20` env-var constant rename (e.g. `ENV_ATTRIBUTION_HOOKS_DISABLED`), `cli/src/cli_schema.rs:32-33` `HOOKS_CLAP_ABOUT` / `HOOKS_TOP_LEVEL_PURPOSE` updated string, resolver unit tests covering: (a) no config + no env -> `true`; (b) env opt-out truthy -> `false`; (c) config `enabled = false` -> `false`; (d) flag/env precedence over config; (e) backwards-compat for users who today rely on the default-off (explicit `false` in config still wins).
    - Out: any Pkl/JSON schema regeneration (next task), any change to the hooks runtime gate logic (covered by existing `commit_msg_policy_gate_passed`), AI-trace probe wiring.
  - Done when: `resolve_config` returns `attribution_hooks_enabled = true` by default; `SCE_ATTRIBUTION_HOOKS_DISABLED=1` sets it to `false`; explicit config-file `enabled = false` is honored; CLI help string updated; resolver unit tests cover the five cases above and pass; no remaining grep matches for `SCE_ATTRIBUTION_HOOKS_ENABLED` in `cli/`.
  - Verification notes (commands or checks): `cargo test -p sce-cli services::config`; `cargo clippy -p sce-cli`; grep `SCE_ATTRIBUTION_HOOKS_ENABLED` should return no matches; manual `sce --help` shows new wording.
  - Completed: 2026-06-15
  - Files changed: `cli/src/services/config/types.rs`, `cli/src/services/config/resolver.rs`, `cli/src/cli_schema.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix flake check` passed; `fff_grep` found no `SCE_ATTRIBUTION_HOOKS_ENABLED` matches under `cli/`; direct targeted `cargo test services::config` was blocked by repo bash policy in favor of `nix flake check`.
  - Notes: Resolver default is now enabled, `SCE_ATTRIBUTION_HOOKS_DISABLED` is parsed with inverted opt-out semantics, explicit config `enabled = false` remains honored, and hooks help text now states enabled-by-default opt-out controls.

- [x] T02: `Sync Pkl base schema and generated JSON schema for opt-out semantics` (status:done)
  - Task ID: T02
  - Goal: Update `config/pkl/base/sce-config-schema.pkl:88-100` and regenerate `config/schema/sce-config.schema.json:46-57` so the `policies.attribution_hooks.enabled` field documents its new default (`true`) and the env-var section / any embedded operator hints reference `SCE_ATTRIBUTION_HOOKS_DISABLED`.
  - Boundaries (in/out of scope):
    - In: Pkl source edits, regenerated JSON schema, any embedded operator-hint text or examples, regression that `cargo test` over schema-embedded validators still passes.
    - Out: code-side resolver changes (T01), runtime DB probe (T03), context docs (T05).
  - Done when: `pkl` regeneration produces the updated JSON schema with no other diff; `cargo test` schema-related tests pass; the JSON schema still validates a sample config with `enabled` omitted (default-true) and with `enabled: false` (explicit opt-out).
  - Verification notes (commands or checks): run the project's canonical Pkl generation step (see `context/sce/generated-opencode-plugin-registration.md` for the generation contract); `cargo test -p sce-cli`; diff inspection that no unrelated schema fields moved.
  - Completed: 2026-06-15
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated` passed; targeted `cargo test services::config` was blocked by repo bash policy in favor of `nix flake check`; `nix flake check` passed; sample configs with `policies.attribution_hooks.enabled` omitted and with `enabled=false` both passed `sce config validate` via `SCE_CONFIG_FILE`.
  - Notes: Generated schema drift is limited to attribution-hooks description/default metadata; no unrelated generated files changed.

- [x] T03: `Add cheap staged-diff AI-overlap evidence helper` (status:done)
  - Task ID: T03
  - Goal: Introduce a unit-testable helper for `commit-msg` that returns a single `bool` answering "does the currently staged diff overlap with captured AI/editor diff-trace evidence?". The helper should reuse existing staged-diff capture, recent diff-trace loading, patch combination, and patch intersection primitives where possible, but should short-circuit as soon as the first AI/editor conversation/trace row produces a positive intersection. Per Decisions, errors of any kind (missing DB, schema not ready, query error, malformed rows only, staged diff read failure, empty staged diff, zero overlap) collapse to `false`. There is no separate fail-open mode.
  - Boundaries (in/out of scope):
    - In: helper surface in the hooks service or a small hooks-owned support seam; staged diff input path based on existing git command helpers; recent diff-trace query reuse with a bounded lookback consistent with the current post-commit flow; patch combine/intersection reuse with early exit on the first positive staged-diff intersection; injected/testable dependencies so unit tests do not require live Git or the operator DB; tests proving `true` for overlapping staged diff + AI trace, `false` for no overlap, `false` for empty staged diff, `false` for error/no-evidence cases, and early-exit behavior that does not keep combining/intersecting later conversations after a positive match.
    - Out: appending or editing the commit message, changing `apply_commit_msg_coauthor_policy`, changing config/env semantics, adding new DB queries/migrations, changing post-commit Agent Trace generation, adding `pre-commit` state files, or changing `pre-commit` behavior.
  - Done when: helper compiles and exposes a `bool`-shaped surface usable by `commit-msg`; tests prove overlap/no-overlap/error outcomes and first-positive early exit; no new AgentTraceDb SQL constants or migrations are added; existing post-commit flow behavior is unchanged.
  - Verification notes (commands or checks): `cargo test -p sce-cli services::hooks`; `cargo clippy -p sce-cli`; grep that no new `SELECT EXISTS` AgentTraceDb presence query was added.
  - Completed: 2026-06-15
  - Files changed: `cli/src/services/hooks/mod.rs`, `cli/src/services/agent_trace.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; targeted `nix develop -c sh -c 'cd cli && cargo test services::hooks'` was blocked by repo bash policy in favor of `nix flake check`; `nix flake check` passed before and after follow-up test removal and after moving pure overlap logic into `agent_trace.rs`; `fff_grep` found no new `SELECT EXISTS` query; migration files remain clean.
  - Notes: Added a hooks-owned staged-diff overlap preflight helper with injectable staged-patch/time/recent-trace dependencies. The live helper uses the no-migration Agent Trace DB hook path, the same seven-day recent diff-trace window as post-commit, `git diff --cached --patch --no-ext-diff`, and existing patch combine/intersection primitives. All read/parse/time/query/open/schema no-evidence paths collapse to `false`; helper is intentionally not wired into commit-msg until T05. Follow-up feedback removed the generated unit tests and their test-only helper function, then moved the pure overlap predicate to `agent_trace::patches_have_overlap` so it is ready for future golden fixture tests.

- [ ] T04: `Add golden tests for AI-overlap evidence predicate` (status:todo)
  - Task ID: T04
  - Goal: Add fixture-backed golden coverage for `agent_trace::patches_have_overlap` so the commit-msg AI-trace evidence gate is protected by deterministic examples before runtime wiring depends on it.
  - Boundaries (in/out of scope):
    - In: checked-in golden fixtures under the existing Rust fixture conventions (prefer `cli/src/services/agent_trace/fixtures/` unless a narrower local convention already exists), tests in the relevant Rust service test module that load candidate/target patches from fixtures, and cases covering positive overlap, no overlap, empty/untouched patch behavior, and at least one Claude structured-patch-derived scenario if it can be represented with existing fixture formats.
    - Out: changing `patches_have_overlap` behavior except to fix a test-proven defect, wiring the helper into `commit-msg`, changing AgentTraceDb queries, changing generated config/Pkl, or broad refactors of patch parsing/intersection.
  - Done when: golden tests fail on fixture drift, prove the intended boolean overlap semantics, run without live Git or live AgentTraceDb access, and reuse existing parser/fixture helpers where practical without duplicating large test harnesses.
  - Verification notes (commands or checks): targeted Rust tests for the agent-trace/patch overlap module (for example `nix develop -c sh -c 'cd cli && cargo test services::agent_trace'` if permitted by policy); `nix flake check` as the repo-level validation fallback.

- [ ] T05: `Extend commit-msg policy seam with an AI-contribution presence input` (status:todo)
  - Task ID: T05
  - Goal: Refactor `apply_commit_msg_coauthor_policy` (and its supporting types) so the transformer accepts a single boolean `ai_contribution_present` signal alongside the existing `HookRuntimeState`, without yet wiring the live DB read. The gate becomes `!sce_disabled && attribution_hooks_enabled && ai_contribution_present`. The seam is intentionally a bare `bool` (not a richer status enum) so error-handling decisions are pushed to the caller per Decisions.
  - Boundaries (in/out of scope):
    - In: update the transformer signature (or introduce a small `CommitMsgPolicyInput` struct in the same file) so the gate evaluates `gate_passed && ai_contribution_present`; update `run_commit_msg_subcommand_in_repo` to pass a placeholder `true` for now (so behavior is unchanged this task); add unit tests for the four combinations of (gate, ai_contribution_present), AND a regression test that `attribution_hooks_enabled = true` + `ai_contribution_present = false` does NOT write the trailer.
    - Out: querying the DB, reading staged files, changing config schema, changing observability surface, introducing any status enum or `Option<bool>` at the seam.
  - Done when: transformer takes the new `bool` input, all four truth-table cases are unit-tested in `cli/src/services/hooks/mod.rs`, existing trailer dedupe/idempotency tests (or newly added equivalents covering the existing behavior) still pass.
  - Verification notes (commands or checks): `cargo test -p sce-cli services::hooks`; `cargo clippy -p sce-cli`; grep that `apply_commit_msg_coauthor_policy` callers in `cli/` are updated.

- [ ] T06: `Wire staged-diff AI-overlap preflight into commit-msg runtime` (status:todo)
  - Task ID: T06
  - Goal: In `run_commit_msg_subcommand_in_repo`, call the T03 staged-diff AI-overlap preflight helper and pass the resulting `bool` into the T05 transformer input. Per Decisions, when the preflight returns `false` (including all error cases — missing DB file, schema not ready, query error, staged diff read failure, malformed/no rows, zero overlap) the policy MUST NOT append the trailer. Errors are logged for diagnostics but never escalate to applying the trailer.
  - Boundaries (in/out of scope):
    - In: invoking the T03 helper from `commit-msg`; DB open + schema-ready check only as needed by the helper and still through the existing no-migration hook path; collapsing any preflight error to `ai_contribution_present = false`; emitting a single logger event for error paths; plumbing the resulting bool through to the transformer call site (`cli/src/services/hooks/mod.rs:1915-1937`).
    - Out: changing `pre-commit`, changing post-commit/post-rewrite flows, changing other commit-msg behaviors (file write semantics, error contexts), short-circuiting the probe via a config key (folded out per Decisions), introducing a fail-open mode of any kind.
  - Done when: when staged diff overlaps captured AI/editor evidence the trailer is applied as the new opt-out default expects; when there is no overlap or any preflight error the message is returned unchanged AND a log line is emitted for the error sub-case (distinguishable from honest no-overlap/no-evidence in logs); unit tests cover the three observable branches (overlap-present, no-overlap/no-evidence-honest, no-evidence-due-to-error) using injected fakes (mirroring the pattern from `run_post_commit_intersection_flow_with`).
  - Verification notes (commands or checks): `cargo test -p sce-cli services::hooks`; manual run `printf 'msg\n' > /tmp/m && sce hooks commit-msg /tmp/m` against a repo with staged diff overlapping seeded diff-trace rows vs empty/non-overlapping rows (no env var required given new default); manual run with the DB file deleted to confirm the no-evidence rule + log line; rerun with `SCE_ATTRIBUTION_HOOKS_DISABLED=1` to confirm opt-out wins; rerun with `SCE_DISABLED=1` to confirm kill-switch wins.

- [ ] T07: `Sync context for opt-out attribution + AI-trace gate` (status:todo)
  - Task ID: T07
  - Goal: Update `context/sce/agent-trace-commit-msg-coauthor-policy.md` to describe the new opt-out default, renamed env var (`SCE_ATTRIBUTION_HOOKS_DISABLED`), AI-trace gating condition, fail posture, and backwards-compat behavior for explicit `enabled = false`; update `context/context-map.md` and `context/sce/agent-trace-hooks-command-routing.md` blurbs that currently say "disabled-default commit-msg attribution".
  - Boundaries (in/out of scope):
    - In: edits to `context/sce/agent-trace-commit-msg-coauthor-policy.md`, the corresponding `context/context-map.md` bullet for that file and for `agent-trace-hooks-command-routing.md`, and the `context/sce/agent-trace-db.md` bullet to mention the new query helper.
    - Out: rewriting overview/architecture/patterns, writing a decision record (only add one under `context/decisions/` if the user explicitly requests it during planning), updating user-facing docs outside `context/`.
  - Done when: the policy context file describes the new opt-out gate, env-var rename, scope, fail posture, and backwards-compat clause; context-map entries are updated; no stale references to "disabled by default" or `SCE_ATTRIBUTION_HOOKS_ENABLED` remain.
  - Verification notes (commands or checks): manual diff review; grep for `disabled by default`, `SCE_ATTRIBUTION_HOOKS_ENABLED`, `attribution_hooks.enabled.*false`, and `apply_commit_msg_coauthor_policy` across `context/` to confirm coverage.

- [ ] T08: `Validation and cleanup` (status:todo)
  - Task ID: T08
  - Goal: Run the full validation suite, remove any temporary scaffolding, and confirm context sync is complete.
  - Boundaries (in/out of scope):
    - In: `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt --check`, `nix flake check` (the project's canonical end-to-end check per `context/sce/agent-trace-commit-msg-coauthor-policy.md`), removal of any planning-only scaffolding, final pass of `context/` to confirm T07 changes are durable, grep for the renamed env var in any installed hook scripts under `config/` to confirm no remaining stale references.
    - Out: feature changes, additional refactors.
  - Done when: all checks pass with no warnings introduced by this plan; `context/` accurately reflects the new opt-out behavior; plan file's tasks are all checked.
  - Verification notes (commands or checks): `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, `nix flake check`.
