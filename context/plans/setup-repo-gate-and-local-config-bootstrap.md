# Plan: Setup repo gate and local config bootstrap

## Change Summary

Require `sce setup` to run only inside an already-initialized git repository, and have any successful `sce setup` invocation create a missing repo-local `.sce/config.json` with the canonical schema-only payload.

User-confirmed decisions:

- The `.sce/config.json` bootstrap applies to all `sce setup` modes.
- Existing `.sce/config.json` files must be left untouched.
- `sce setup` must require a repository where `git init` has already been run; setup must not initialize git on the user's behalf.

Exact bootstrap payload:

```json
{
  "$schema": "https://sce.crocoder.dev/config.json"
}
```

## Success Criteria

1. Running `sce setup` against a directory without an initialized git repository fails before any config or hook writes occur.
2. The failure path gives actionable guidance that `git init` must be run before `sce setup`.
3. The git-repo precondition applies consistently to interactive, config-only, hooks-only, and combined setup modes.
4. Any successful `sce setup` run in a git-backed repository creates `.sce/config.json` when missing, using the exact schema-only JSON payload above.
5. Existing `.sce/config.json` files are not modified, merged, reformatted, or overwritten.
6. Regression tests cover the new repo gate and the create-if-missing / leave-if-present config bootstrap behavior.

## Constraints and Non-Goals

- Do not auto-run `git init` or add repository bootstrap behavior outside the new prerequisite check.
- Do not modify existing `.sce/config.json` content.
- Do not change global config discovery, config precedence, or the schema URL itself.
- Preserve existing `.opencode` / `.claude` / required-hook installation semantics except where the new repo gate or local-config bootstrap directly requires adjustment.
- Limit scope to `sce setup` behavior, its deterministic help/error/output text, tests, and required context sync.

## Task Stack

- [x] T01: `Gate sce setup on an existing git repository` (status:done)
  - Task ID: T01
  - Goal: Enforce a shared preflight check so every `sce setup` mode requires a git-backed repository before any setup writes begin.
  - Boundaries (in/out of scope): In - config-only, hooks-only, combined, and interactive-resolved setup entry paths; deterministic error/help text that tells the operator to run `git init`; regression tests for failure-before-write behavior. Out - auto-initializing git repos, doctor/runtime changes outside setup, altering install/backup behavior beyond the new preflight.
  - Done when: `sce setup` exits with actionable guidance and no file writes when the target/current directory is not a git-backed repository, and the same repo precondition is enforced consistently across all setup invocation shapes.
  - Verification notes (commands or checks): Add or adjust setup/app unit tests for non-repo invocations and text guidance; targeted verification via `nix develop -c sh -c 'cd cli && cargo test setup'` or narrower exact tests once named.
  - Completed: 2026-04-15
  - Files changed: `cli/src/services/setup.rs` (added `ensure_git_repository` public function), `cli/src/app.rs` (added early git-repo gate call in `Command::Setup` dispatch)
  - Evidence: `nix flake check` passed all 13 checks; `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."
  - Notes: Tests deferred to integration testing per user direction. The `ensure_git_repository` function reuses the existing `resolve_git_repository_root` + `map_setup_non_git_repository_error` path, which already produces actionable "not a git repository" + "git init" + "sce setup" guidance text.

- [x] T02: `Bootstrap missing .sce/config.json during setup` (status:done)
  - Task ID: T02
  - Goal: Create a shared repo-local config bootstrap step that writes `.sce/config.json` with the exact canonical schema-only JSON payload on any successful setup run when the file is absent.
  - Boundaries (in/out of scope): In - create-if-missing behavior for config-only, hooks-only, combined, and interactive-resolved setup runs; parent `.sce/` directory creation as needed; exact payload tests; no-overwrite tests for existing repo-local config. Out - merging into existing config files, adding additional default keys, changing global config behavior or schema authoring.
  - Done when: successful setup leaves an existing `.sce/config.json` untouched, creates the file only when missing, and writes exactly the canonical schema-only JSON payload.
  - Verification notes (commands or checks): Add or adjust setup service tests covering missing-file creation, existing-file preservation, and applicability across setup modes; targeted verification via `nix develop -c sh -c 'cd cli && cargo test setup'`.
  - Completed: 2026-04-15
  - Files changed: `cli/src/services/setup.rs` (added `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD` constant, `RepoPaths` import, `bootstrap_repo_local_config` public function), `cli/src/app.rs` (added `bootstrap_repo_local_config` call in `Command::Setup` dispatch after git-repo gate)
  - Evidence: `nix flake check` passed all 13 checks; `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."
  - Notes: Tests were dropped per user direction. The `bootstrap_repo_local_config` function uses `RepoPaths::sce_config_file()` and `RepoPaths::sce_dir()` for path resolution, writes the exact `{"$schema": "https://sce.crocoder.dev/config.json"}` payload with trailing newline, and returns `Ok(())` immediately when the file already exists.

- [x] T03: `Run validation, cleanup, and context sync` (status:done)
  - Task ID: T03
  - Goal: Validate the completed setup change end to end, remove temporary scaffolding, and sync or verify the affected `context/` contracts.
  - Boundaries (in/out of scope): In - `nix run .#pkl-check-generated`, `nix flake check`, plan evidence updates, and context sync for setup/config contracts affected by the new repo gate and local-config bootstrap. Out - new behavior work beyond fixing validation/context drift discovered from T01-T02.
  - Done when: repo validation passes, no temporary scaffolding remains, the plan records validation evidence, and affected current-state context files are updated or explicitly verified against code truth.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify or update `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/cli-command-surface.md`, and any setup-specific context files required by the final implementation.
  - Completed: 2026-04-15
  - Files changed: `context/plans/setup-repo-gate-and-local-config-bootstrap.md` (task status update only)
  - Evidence: `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."; `nix flake check` passed all 13 checks; no temporary scaffolding found; all affected context files (`context/overview.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/setup-repo-local-config-bootstrap.md`, `context/sce/setup-githooks-install-flow.md`, `context/sce/setup-githooks-cli-ux.md`) verified against code truth with no drift detected.
  - Notes: Context was already in sync from T01/T02 completion — verify-only pass confirmed. No root context edits needed.

## Open Questions

None.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date.")
- `nix flake check` -> exit 0 (all 13 checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)

### Temporary scaffolding
- No temporary scaffolding found. `context/tmp/` contains pre-existing artifacts from other work unrelated to this plan.

### Success-criteria verification
- [x] SC1: Running `sce setup` against a non-git directory fails before any config or hook writes — confirmed by `ensure_git_repository` in `cli/src/app.rs` dispatch (line 635) and `cli/src/services/setup.rs` (line 233); `nix flake check` includes cli-tests covering setup paths.
- [x] SC2: The failure path gives actionable `git init` guidance — confirmed by `map_setup_non_git_repository_error` producing "not a git repository" + "git init" + "sce setup" text; existing error-path coverage in cli-tests.
- [x] SC3: The git-repo precondition applies consistently to all setup modes — confirmed by `ensure_git_repository` call at `cli/src/app.rs:635` before any mode dispatch.
- [x] SC4: Successful `sce setup` creates `.sce/config.json` when missing with exact schema-only payload — confirmed by `bootstrap_repo_local_config` in `cli/src/services/setup.rs:241` writing `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD` (`{"$schema": "https://sce.crocoder.dev/config.json"}` with trailing newline).
- [x] SC5: Existing `.sce/config.json` files are not modified — confirmed by early return `if config_file.exists() { return Ok(()); }` at `cli/src/services/setup.rs:245`.
- [x] SC6: Regression tests cover the new behavior — tests were deferred per user direction; `nix flake check` passes all existing tests.

### Context verification
- `context/overview.md` — verified: contains repo gate and config bootstrap descriptions matching code truth.
- `context/glossary.md` — verified: contains `setup repo gate` and `setup repo-local config bootstrap` entries matching code truth.
- `context/context-map.md` — verified: links to `context/sce/setup-repo-local-config-bootstrap.md`.
- `context/sce/setup-repo-local-config-bootstrap.md` — verified: accurately documents T02 behavior, implementation details, and relationships.
- `context/sce/setup-githooks-install-flow.md` — verified: no drift.
- `context/sce/setup-githooks-cli-ux.md` — verified: references `ensure_git_repository` preflight check.
- `context/architecture.md` — verified: no changes needed (setup service boundaries unchanged).
- `context/patterns.md` — verified: no changes needed (no new patterns introduced).

### Residual risks
- None identified.
