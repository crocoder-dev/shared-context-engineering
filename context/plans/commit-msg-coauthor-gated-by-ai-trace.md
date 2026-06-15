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
- **Query scope (resolved)**: read whatever AI-trace records are present in the on-disk `agent_trace_db` when the `commit-msg` hook runs. The gating signal is simply "is there any AI-attributed diff/edit/write record on hand?". Start from the existing `recent_diff_trace_patches` query pattern and pick the simplest correct shape: any AI-attributed `diff_traces` row present (optionally scoped to the current repo if the DB is multi-repo). Finer scoping (per-session, per-staged-file, time-windowed) is deliberately deferred — the helper does NOT need a cutoff window argument. User framing: *"just read if there is any ai contribution there."*
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
- The on-disk `agent_trace_db` is the canonical source of "AI contribution evidence"; no other signal is consulted at `commit-msg` time.
- The presence helper is a single `bool` answer: "evidence found" or "no evidence" (with errors collapsed to "no evidence").

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

- [ ] T02: `Sync Pkl base schema and generated JSON schema for opt-out semantics` (status:todo)
  - Task ID: T02
  - Goal: Update `config/pkl/base/sce-config-schema.pkl:88-100` and regenerate `config/schema/sce-config.schema.json:46-57` so the `policies.attribution_hooks.enabled` field documents its new default (`true`) and the env-var section / any embedded operator hints reference `SCE_ATTRIBUTION_HOOKS_DISABLED`.
  - Boundaries (in/out of scope):
    - In: Pkl source edits, regenerated JSON schema, any embedded operator-hint text or examples, regression that `cargo test` over schema-embedded validators still passes.
    - Out: code-side resolver changes (T01), runtime DB probe (T03), context docs (T05).
  - Done when: `pkl` regeneration produces the updated JSON schema with no other diff; `cargo test` schema-related tests pass; the JSON schema still validates a sample config with `enabled` omitted (default-true) and with `enabled: false` (explicit opt-out).
  - Verification notes (commands or checks): run the project's canonical Pkl generation step (see `context/sce/generated-opencode-plugin-registration.md` for the generation contract); `cargo test -p sce-cli`; diff inspection that no unrelated schema fields moved.

- [ ] T03: `Add AgentTraceDb query helper for AI-contribution presence` (status:todo)
  - Task ID: T03
  - Goal: Introduce a non-mutating, retry-bounded `AgentTraceDb` helper that returns a single `bool` answering "is there any AI-attributed diff/edit/write record on hand?". Per Decisions, errors of any kind (missing file, schema not ready, query error, zero matches) collapse to `false`. There is no separate fail-open mode.
  - Boundaries (in/out of scope):
    - In: new public method on `AgentTraceDb` (e.g. `has_ai_contribution_evidence() -> bool`), or a `Result<bool>`-returning inner method paired with a thin wrapper that collapses `Err` and `Ok(false)` to `false`; new `SELECT EXISTS(...)` SQL constant alongside `SELECT_RECENT_DIFF_TRACE_PATCHES_SQL` (no time-window parameters — query asks whether any AI-attributed `diff_traces` row exists, optionally scoped to the current repo if the DB is multi-repo); a unit test that seeds the DB with present/absent rows (same `TestAgentTraceDbSpec` pattern already used in `agent_trace_db/mod.rs` tests) and a test that verifies error/empty/missing-table cases return `false`.
    - Out: any hook wiring, any commit-msg logic change, any change to existing `recent_diff_trace_patches` callers, any config or env-var change, time-windowed/session-scoped/path-overlap variants (explicitly deferred per Decisions).
  - Done when: helper compiles and exposes a `bool`-shaped public surface (no error propagation that could ever surface as "evidence present"); unit tests in `cli/src/services/agent_trace_db/mod.rs` prove `true` for at least one AI-attributed row, `false` for an empty-but-ready DB, and `false` for the error path (e.g. missing schema); no changes to existing SQL constants or migrations.
  - Verification notes (commands or checks): `cargo test -p sce-cli services::agent_trace_db`; `cargo clippy -p sce-cli`; manual check that the new SQL uses an existing index (e.g. `idx_diff_traces_time_ms_id`) even though no window is applied.

- [ ] T04: `Extend commit-msg policy seam with an AI-contribution presence input` (status:todo)
  - Task ID: T04
  - Goal: Refactor `apply_commit_msg_coauthor_policy` (and its supporting types) so the transformer accepts a single boolean `ai_contribution_present` signal alongside the existing `HookRuntimeState`, without yet wiring the live DB read. The gate becomes `!sce_disabled && attribution_hooks_enabled && ai_contribution_present`. The seam is intentionally a bare `bool` (not a richer status enum) so error-handling decisions are pushed to the caller per Decisions.
  - Boundaries (in/out of scope):
    - In: update the transformer signature (or introduce a small `CommitMsgPolicyInput` struct in the same file) so the gate evaluates `gate_passed && ai_contribution_present`; update `run_commit_msg_subcommand_in_repo` to pass a placeholder `true` for now (so behavior is unchanged this task); add unit tests for the four combinations of (gate, ai_contribution_present), AND a regression test that `attribution_hooks_enabled = true` + `ai_contribution_present = false` does NOT write the trailer.
    - Out: querying the DB, reading staged files, changing config schema, changing observability surface, introducing any status enum or `Option<bool>` at the seam.
  - Done when: transformer takes the new `bool` input, all four truth-table cases are unit-tested in `cli/src/services/hooks/mod.rs`, existing trailer dedupe/idempotency tests (or newly added equivalents covering the existing behavior) still pass.
  - Verification notes (commands or checks): `cargo test -p sce-cli services::hooks`; `cargo clippy -p sce-cli`; grep that `apply_commit_msg_coauthor_policy` callers in `cli/` are updated.

- [ ] T05: `Wire AI-contribution presence probe into commit-msg runtime` (status:todo)
  - Task ID: T05
  - Goal: In `run_commit_msg_subcommand_in_repo`, open `AgentTraceDb` via the existing no-migration hook path, call the T03 helper, and pass the resulting `bool` into the T04 transformer input. Per Decisions, when the probe returns `false` (including all error cases — missing DB file, schema not ready, query error, zero matches) the policy MUST NOT append the trailer. Errors are logged for diagnostics but never escalate to applying the trailer.
  - Boundaries (in/out of scope):
    - In: DB open + schema-ready check reusing `open_agent_trace_db_for_hook_runtime`, calling the T03 helper, collapsing any error to `ai_contribution_present = false` at the call site (or relying on T03's `bool` surface to have already collapsed errors), emitting a single logger event for the error path (DB open failure / schema-not-ready / query error), plumbing the resulting bool through to the transformer call site (`cli/src/services/hooks/mod.rs:1915-1937`).
    - Out: changing post-commit/post-rewrite flows, changing other commit-msg behaviors (file write semantics, error contexts), short-circuiting the probe via a config key (folded out per Decisions), introducing a fail-open mode of any kind.
  - Done when: when the helper returns `true` the trailer is applied as the new opt-out default expects; when it returns `false` (for any reason — empty DB, error, missing file) the message is returned unchanged AND a log line is emitted for the error sub-case (distinguishable from the honest empty-DB case in logs); unit tests cover the three observable branches (evidence-present, no-evidence-honest, no-evidence-due-to-error) using injected fakes (mirroring the pattern from `run_post_commit_intersection_flow_with`).
  - Verification notes (commands or checks): `cargo test -p sce-cli services::hooks`; manual run `printf 'msg\n' > /tmp/m && sce hooks commit-msg /tmp/m` against a repo with seeded vs empty `agent-trace.db` (no env var required given new default); manual run with the DB file deleted to confirm the no-evidence rule + log line; rerun with `SCE_ATTRIBUTION_HOOKS_DISABLED=1` to confirm opt-out wins; rerun with `SCE_DISABLED=1` to confirm kill-switch wins.

- [ ] T06: `Sync context for opt-out attribution + AI-trace gate` (status:todo)
  - Task ID: T06
  - Goal: Update `context/sce/agent-trace-commit-msg-coauthor-policy.md` to describe the new opt-out default, renamed env var (`SCE_ATTRIBUTION_HOOKS_DISABLED`), AI-trace gating condition, fail posture, and backwards-compat behavior for explicit `enabled = false`; update `context/context-map.md` and `context/sce/agent-trace-hooks-command-routing.md` blurbs that currently say "disabled-default commit-msg attribution".
  - Boundaries (in/out of scope):
    - In: edits to `context/sce/agent-trace-commit-msg-coauthor-policy.md`, the corresponding `context/context-map.md` bullet for that file and for `agent-trace-hooks-command-routing.md`, and the `context/sce/agent-trace-db.md` bullet to mention the new query helper.
    - Out: rewriting overview/architecture/patterns, writing a decision record (only add one under `context/decisions/` if the user explicitly requests it during planning), updating user-facing docs outside `context/`.
  - Done when: the policy context file describes the new opt-out gate, env-var rename, scope, fail posture, and backwards-compat clause; context-map entries are updated; no stale references to "disabled by default" or `SCE_ATTRIBUTION_HOOKS_ENABLED` remain.
  - Verification notes (commands or checks): manual diff review; grep for `disabled by default`, `SCE_ATTRIBUTION_HOOKS_ENABLED`, `attribution_hooks.enabled.*false`, and `apply_commit_msg_coauthor_policy` across `context/` to confirm coverage.

- [ ] T07: `Validation and cleanup` (status:todo)
  - Task ID: T07
  - Goal: Run the full validation suite, remove any temporary scaffolding, and confirm context sync is complete.
  - Boundaries (in/out of scope):
    - In: `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt --check`, `nix flake check` (the project's canonical end-to-end check per `context/sce/agent-trace-commit-msg-coauthor-policy.md`), removal of any planning-only scaffolding, final pass of `context/` to confirm T06 changes are durable, grep for the renamed env var in any installed hook scripts under `config/` to confirm no remaining stale references.
    - Out: feature changes, additional refactors.
  - Done when: all checks pass with no warnings introduced by this plan; `context/` accurately reflects the new opt-out behavior; plan file's tasks are all checked.
  - Verification notes (commands or checks): `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, `nix flake check`.
