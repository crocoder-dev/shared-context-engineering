# Plan: Auto-push Agent Trace git notes after SCE post-commit

## Change summary

Extend the existing `sce hooks post-commit` Agent Trace git-notes flow so SCE also attempts to push the configured Agent Trace notes ref to the commit remote after successfully writing the local git note. The default behavior is automatic push to the existing post-commit remote (`origin` as passed by the setup-installed hook template), with an optional config switch to disable the push.

The push is best-effort and silent on failure: if the push cannot complete, the hook must not fail and SCE should simply try again on a later post-commit hook invocation.

## Success criteria

- After a successful git `sce hooks post-commit --vcs git --remote-url <url>` Agent Trace run, SCE writes the local git note as today and then attempts to push the configured notes ref to the target remote.
- The default notes ref remains `refs/notes/sce-agent-trace` unless overridden by `policies.agent_trace.git_notes_ref`.
- Notes auto-push is enabled by default and can be disabled through SCE config.
- Push failures are fail-open and silent from the user's perspective: they do not fail the hook, do not block commits, and do not require immediate action.
- A later successful post-commit hook invocation can attempt the push again without requiring a retry queue or persisted failed-push state.
- Tests cover default enabled behavior, config-disabled behavior, configured notes ref behavior, command construction, and silent fail-open push errors.
- Current-state context documents the new default auto-push behavior, disable switch, and fail-open/no-retry-queue posture.

## Constraints and non-goals

- Constraints:
  - Preserve the existing post-commit local git-note write behavior and its best-effort posture.
  - Push only after Agent Trace JSON validation, Agent Trace DB persistence, and local git-note write have completed successfully enough to have a local note to publish.
  - Reuse existing hook config resolution and git command execution patterns; avoid shell interpolation.
  - Keep stdout/stderr output stable unless existing logging conventions require a debug-level internal event.
  - Honor the configured Agent Trace notes ref consistently for local note write and remote push.
  - Use the post-commit remote context already available to the hook flow rather than inventing a separate remote-discovery path in this plan.
- Non-goals:
  - Fetching notes from remotes.
  - Backfilling or pushing historical notes outside normal post-commit hook flow.
  - Adding a retry queue, background daemon, scheduled sync, or persisted failed-push state.
  - Introducing a user-facing `sce sync` command.
  - Changing Agent Trace JSON schema or Agent Trace DB schema.
  - Blocking commits or surfacing push failures as hook failures.

## Assumptions

- The disable switch should live under the existing config namespace, e.g. `policies.agent_trace.push_notes.enabled`, with default `true`; exact naming may follow existing config style during implementation.
- The remote push target should be the remote used by the setup-installed hook template (`origin` in current behavior) or an equivalent validated remote derived from the existing post-commit handoff; this plan should not add arbitrary remote selection UX.
- Silent fail means no user-facing failure and no new stdout/stderr warning. Internal debug/error logging may still be acceptable if it follows existing observability conventions and does not disturb normal hook UX.

## Task stack

- [x] T01: `Add config switch for Agent Trace notes auto-push` (status:done)
  - Task ID: T01
  - Completed: 2026-07-15
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json`, `cli/assets/generated/config/schema/sce-config.schema.json`, `cli/src/services/config/{types,schema,resolver,render}.rs`, `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/config-precedence-contract.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check --print-build-logs` passed (144 Rust tests, clippy/fmt/parity checks clean).
  - Goal: Add a typed config value that controls whether post-commit Agent Trace git notes are auto-pushed, defaulting to enabled.
  - Boundaries (in/out of scope): In - Pkl schema/config schema updates, Rust config DTO/resolver mapping, default `true`, explicit `false` override, `sce config show|validate` visibility if required by existing policy rendering. Out - git push execution, hook runtime wiring, remote selection behavior.
  - Done when: runtime config exposes a resolved auto-push boolean; default resolution is enabled; explicit config disable is honored; generated schema/parity covers the new field; invalid config shapes fail validation consistently with existing config policy fields.
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl`; targeted config tests if appropriate; `nix run .#pkl-check-generated`; `nix flake check`.

- [ ] T02: `Introduce git-notes push helper` (status:todo)
  - Task ID: T02
  - Goal: Add a small, injectable helper that attempts to push the configured Agent Trace notes ref to the chosen git remote without shell interpolation.
  - Boundaries (in/out of scope): In - helper function/type near existing git-note writer logic, command construction for pushing one notes ref, validation of non-blank remote/ref inputs, tests for command args and failure outcome. Out - deciding when to call the helper, config resolution, backfill/fetch/retry behavior.
  - Done when: helper constructs a deterministic `git push <remote> <ref>`-equivalent invocation for the configured notes ref, returns a structured success/failure outcome, does not emit user-facing output directly, and focused tests cover success and git-command failure.
  - Verification notes (commands or checks): targeted hook/helper tests if appropriate; `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix flake check`.

- [ ] T03: `Wire silent auto-push into post-commit Agent Trace flow` (status:todo)
  - Task ID: T03
  - Goal: After successful local Agent Trace git-note persistence, conditionally attempt a best-effort notes push when auto-push config is enabled.
  - Boundaries (in/out of scope): In - post-commit flow ordering, config gate, git-only behavior, existing remote context reuse, fail-open/silent handling, tests proving enabled/default attempt, disabled skip, configured ref use, and push failure does not change hook success. Out - retry queue, user-facing command output changes, fetch/backfill, non-git VCS note pushing.
  - Done when: default git post-commit flow attempts the push after local note write; explicit config disable skips the push; configured notes ref is used; push failure is swallowed from hook success/output and can be retried by a later hook invocation.
  - Verification notes (commands or checks): targeted post-commit hook tests if appropriate; manual local dry-run/review of command construction; `nix flake check`.

- [ ] T04: `Document Agent Trace notes auto-push behavior` (status:todo)
  - Task ID: T04
  - Goal: Sync current-state context to describe default auto-push, disable config, and silent fail-open retry-on-next-commit behavior.
  - Boundaries (in/out of scope): In - focused updates to `context/sce/agent-trace-hooks-command-routing.md`, `context/cli/config-precedence-contract.md`, `context/sce/setup-githooks-hook-asset-packaging.md` if hook behavior text needs adjustment, `context/context-map.md`, and glossary entry if a new term is introduced. Out - broad narrative docs rewrites, completed-work summaries in durable context, implementation code.
  - Done when: context no longer states “No git-notes push/fetch/backfill behavior” as current behavior without qualification; documents that push is default-enabled, config-disableable, silent fail-open, and retried only by future post-commit invocations.
  - Verification notes (commands or checks): `rg "git-notes|git notes|push_notes|push notes|No git-notes" context/`; manual diff review; `git diff --check`.

- [ ] T05: `Validate notes auto-push and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Run final validation for the complete plan and clean up temporary scaffolding.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, formatting/lint/test checks, stale-string review, cleanup of temporary repos/remotes/notes refs used during testing, plan status/evidence updates. Out - new behavior beyond completed task stack.
  - Done when: `nix flake check` passes or any failure is documented as pre-existing/unrelated; `nix run .#pkl-check-generated` passes; context sync is verified; no temporary scaffolding remains.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `git diff --check`; `rg "refs/notes/sce-agent-trace|push_notes|git notes.*push|No git-notes" cli/ config/ context/`.

## Open questions

None. Plan is ready for T01 execution.
