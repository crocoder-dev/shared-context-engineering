# Plan: config-schema-series-compatibility

## Change summary

Config files currently break on every release. This plan makes SCE config
`$schema` declarations compatible within a `major.minor` release series. It also
makes `sce setup` upgrade an older-series declaration in the canonical repository
and global config files. The upgrade replaces only `$schema`, validates the full
candidate before saving, and saves through the existing guarded atomic-write
path. Validation, `sce config validate`, `sce doctor`, and runtime config
loading stay read-only and keep validating every field against the embedded
schema.

This plan extends existing behavior. It is unrelated to PR #351
(`fix/sce-login-status`, open, all checks passing), which changes saved-login
status reporting in doctor, token storage, the `sce-info` plugin, and hook
assets. None of its 26 changed files overlap this plan's Rust config or setup
files. This branch (`fix/schema-versions`) currently points at PR #351's head
(`3e9e313f`), so the plan stacks on that work. Rebase onto `main` after #351
merges. Do not fold this work into that PR.

### Current behavior and causes of release-to-release incompatibility

1. **Exact-version `const`.** `config/pkl/base/sce-config-schema.pkl` builds
   `configSchemaUrl` from the full `.version` string
   (`https://sce.crocoder.dev/v0.4.0-pre-alpha-v14/config.json`). It uses that
   URL as both the schema `$id` and the `const` of the `$schema` property. Each
   `nix run .#bump-version` changes `.version`, so the next CLI's embedded
   schema rejects every declaration written by an earlier release, including
   earlier releases in the same series.
2. **Setup never revisits an existing declaration.**
   `bootstrap_repo_local_config` (`cli/src/services/setup/mod.rs`) returns
   early when `.sce/config.json` exists. The access writer
   (`cli/src/services/setup/access_preferences.rs`) only sets `$schema` when it
   creates a missing repository file. Setup never writes the global file. A
   stale declaration therefore remains stale after setup.
3. **A stale declaration invalidates the whole file.** `parse_file_config`
   (`cli/src/services/config/schema.rs`) fails on the `/$schema` `const`
   mismatch. In `resolve_runtime_config_with`
   (`cli/src/services/config/resolver.rs`), a failing default-discovered file
   is skipped and recorded in `validation_errors`. Every setting in that file
   is ignored, including `auth.mode`, `telemetry.enabled`,
   `agent_trace.repository_remote`, integrations, and custom policies. The
   access policy degrades to `TelemetryDisabledReason::InvalidConfig` and
   `background_cloud_allowed = false`. An explicit `--config` or
   `SCE_CONFIG_FILE` target fails hard.
4. **Setup preflights read the file before an upgrade could run.** In
   `SetupCommand::execute` (`cli/src/services/setup/command.rs`):
   - `resolve_setup_repository` calls
     `resolve_agent_trace_storage_runtime_config`. A stale local file is
     skipped, so the named-remote preflight checks `origin` instead of the
     configured `agent_trace.repository_remote`. It can fail or check the wrong
     remote.
   - `prepare_dispatch` then reads the following inputs, all through
     `parse_file_config`:
     - the access policy, which loses any global or repository restriction in
       the stale file;
     - `read_repository_access_preferences`, which returns
       `AccessPreferences::Invalid` and skips access prompts;
     - `persisted_integration_targets`, `persisted_optional_workflows`, and
       `persisted_setup_confirmations`, which drop saved selections.
   - `persist_repository_access_preferences` refuses an invalid file.
   - `persist_integration_targets` skips persistence for an invalid local file.
   - App startup (`cli/src/app.rs` → `app_support::invalid_discovered_config_guidance`)
     logs the generic "Local `.sce` config is invalid" guidance. It does not say
     that `sce setup` can repair the problem.

### Target behavior for each declaration shape

| Declaration in an existing canonical file | Validation (all read paths) | `sce setup` |
|---|---|---|
| `$schema` absent | Accepted; other fields validated as today | Unchanged; no declaration added; no report line |
| Canonical URL in the CLI series (for a `0.4` CLI: `0.4.0-pre-alpha-v1`, `-v14`, `0.4.0`, `0.4.7`, `0.4.7+build.5`) | Accepted | Unchanged, even when patch or prerelease differs; no report line |
| Canonical URL in an older series (`0.3.3`, `0.1.0`; with a `0.5` CLI, any `0.4.x`) | Rejected with an actionable message naming both series and `sce setup` | Candidate with only `$schema` set to the CLI's concrete URL is validated. A passing candidate is saved and reported as upgraded. A failing candidate leaves the file byte-identical and reports the validation errors with remediation |
| Canonical URL in a newer series (`0.5.0`, `1.4.0` for a `0.4` CLI) | Rejected with a message to upgrade the SCE CLI | Never downgraded; byte-identical; reported as left unchanged |
| Malformed or foreign value (`https://example.com/s.json`, unversioned `https://sce.crocoder.dev/config.json`, `…/v0.4/config.json`, `http://…`, non-string) | Rejected with a message naming the current canonical URL | Never rewritten; byte-identical; reported as unrecognized |
| Explicit `--config` / `SCE_CONFIG_FILE` file at a non-canonical path | Same validation rules | Never written |

When both canonical files exist, setup evaluates and saves each one
independently. One invalid file does not block upgrading the other. A file
that is still invalid keeps the existing degraded-config behavior. An invalid
local file skips access prompts and target persistence. An invalid global file
is skipped by resolution. A missing global file is never created to upgrade a
schema. Repository bootstrap and the access writer create missing repository
files with the current URL as they do today.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: The embedded config schema accepts every canonical `https://sce.crocoder.dev/v<major>.<minor>.<patch>[-pre][+build]/config.json` declaration in the CLI's `major.minor` series. It rejects other series, malformed or foreign URLs, and non-string values. Every other field is still validated.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_declaration`, which runs the embedded-schema compatibility matrix and the classifier parity tests, plus the `$schema`-valid/other-field-invalid case.
- [ ] AC2: The accepted series comes from `.version` alone. Changing the release series changes the generated `$schema` rule and the Rust classifier without editing any other constant.
  - Validate: `nix run .#pkl-check-generated`, which runs the Pkl series-pattern fixtures for `0.4.0-pre-alpha-v14`, `0.5.0`, and `1.4.2`. Also confirm that `nix shell nixpkgs#ripgrep -c rg -n '0\\.4' config/pkl/base/sce-config-schema*.pkl cli/src/services/config/schema_declaration.rs` finds no hard-coded series outside test fixtures.
- [ ] AC3: `sce config validate`, `sce doctor` (including `--fix`), and ordinary command startup never modify a config file with an older, newer, or foreign declaration. Each prints an actionable `$schema` message.
  - Validate: the T07 end-to-end scenario's read-only phase compares file hashes before and after each command.
- [ ] AC4: `sce setup` upgrades an older-series declaration in the existing repository `.sce/config.json` and the existing global config. Only the `$schema` value changes, and every other byte is preserved, including auth policy, telemetry consent, integrations, and custom policies. A second run leaves both files byte-identical.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_upgrade` plus the T07 scenario's upgrade and idempotency phases.
- [ ] AC5: Setup never writes a newer-series, malformed, foreign, absent, or same-series declaration. It never creates a missing global config for a schema upgrade, and it never writes an explicit `--config`/`SCE_CONFIG_FILE` file at a non-canonical path.
  - Validate: `schema_upgrade` unit matrix plus the `setup` command tests from T05.
- [ ] AC6: When an older-series candidate fails validation, the file stays byte-identical and setup reports the path, the validation errors, and the remediation. Any other canonical file is still upgraded.
  - Validate: `schema_upgrade` candidate-invalid test plus the T07 mixed-file phase.
- [ ] AC7: Upgrade writes keep the existing protections. A symlinked file, whether its target is valid or dangling, is refused and left untouched. A file whose bytes change between planning and commit is refused. The write is atomic and keeps the file mode.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_upgrade` (symlink, dangling-symlink, concurrent-modification, and mode cases) and `… test … access_preferences`, which must stay unchanged after the T03 refactor.
- [ ] AC8: An older-series declaration does not change setup's preflights. The remote preflight checks the remote configured in a stale but otherwise valid local file. Access prompts, the access gate, and saved selections come from the upgraded files. A failed Git or remote preflight writes nothing.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` (T05 ordering tests).

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`

### Context sync

- `context/sce/config-schema-publication.md`: series-scoped `$schema` rule; `$id` unchanged; snapshot policy.
- `context/cli/config-precedence-contract.md`: `$schema` compatibility rule, actionable messages, and the read-only guarantee. Also repair the drifted unversioned `https://sce.crocoder.dev/config.json` mention.
- `context/sce/setup-repo-local-config-bootstrap.md`: schema-upgrade ordering, the global `$schema`-only write, the `Config schema:` report section, and the replacement for the statement that the global config is only read.
- `context/glossary.md`: "config schema compatibility series".
- `context/decisions/`: a new ADR for setup writing only the global `$schema` value. It amends the read-only global-config statement in `2026-10-22-setup-writes-access-answers-to-repository-config.md` without changing the rule that access answers go only to the repository config.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - Pkl schema: `config/pkl/base/sce-config-schema.pkl`, a new `config/pkl/base/sce-config-schema-url.pkl` helper, and `config/pkl/renderers/generation-contract-check.pkl`.
  - Config: `cli/src/services/config/schema.rs`, a new `cli/src/services/config/schema_declaration.rs`, and `cli/src/services/config/mod.rs`. In `cli/src/services/config/resolver.rs`, only the visibility of `resolve_agent_trace_storage_runtime_config_with` changes.
  - Setup: `cli/src/services/setup/access_preferences.rs`, `cli/src/services/setup/atomic_write.rs`, a new `cli/src/services/setup/config_file_write.rs`, a new `cli/src/services/setup/schema_upgrade.rs`, `cli/src/services/setup/mod.rs`, and `cli/src/services/setup/command.rs`.
  - App startup: the `cli/src/services/app_support.rs` guidance line.
  - End-to-end: a new `scripts/test-setup-schema-upgrade.sh` and its `flake.nix` check.
  - Context files listed under Context sync.
- **Out of scope:**
  - Rewriting historical `schema/v*/config.json` snapshots, including the released `v0.4.0-pre-alpha-v14` snapshot.
  - Changes to `nix/bump-version.sh`; it already regenerates the snapshot from `.version`.
  - Field migrations.
  - Network schema fetches.
  - Upgrades from `sce doctor --fix` or `sce config`.
  - The Agent Trace schema.
  - PR #351's login-status work.
- **Constraints:**
  - No new crate dependency. Version parsing is a small typed parser; `semver` is not added.
  - Static dispatch and typed error enums, as `AGENTS.md` requires.
  - Every candidate is validated through `parse_file_config` with the file's own `ConfigPathSource`, using the same validator as all other read paths.
  - Writes go through the existing reread-compare, symlink-refusal, and `write_atomically` sequence.
  - The Git-repo gate and named-remote preflight still run before every setup write.
  - Startup, validate, and doctor resolution stay read-only.
- **Non-goal:**
  - A general config migration framework, or accepting declarations from other series "leniently".
  - Adding `$schema` to existing files that omit it.
  - Normalizing formatting of an upgraded file.

## Assumptions

- The canonical declaration form is exactly `https://sce.crocoder.dev/v<semver>/config.json`. Every historical `schema/v*/config.json` snapshot (`v0.1.0` through `v0.4.0-pre-alpha-v14`) uses it, so it is the only form setup recognizes as an upgradable older declaration.
- The CLI series comes from `services::version::PACKAGE_VERSION`. It reads `SCE_VERSION` from `.version` and falls back to `CARGO_PKG_VERSION`, which `bump-version` and the version-parity check keep equal. The Pkl series comes from the same `.version`.
- The schema `$id` stays the CLI's concrete versioned URL. Only the `$schema` property changes from `const` to a series `pattern`. Published snapshots therefore keep a concrete identity while accepting same-series declarations.
- Schema-upgrade outcomes are reported but never fatal. This matches the existing degraded handling of invalid discovered config: setup continues and exits successfully, and a still-invalid file keeps today's degraded behavior. Upgrade write failures do not reuse the access writer's fatal `SCE-ERR-RUNTIME` path, because a global config managed through dotfile symlinks should not block repository setup.
- Upgrades run in every normal setup mode (interactive, non-interactive, target flags, and hooks-only) but not in context-only (`--bootstrap-context`). Context-only setup performs no config work.
- An upgraded file keeps its original bytes apart from the declaration string. When the old URL string occurs exactly once and swapping it yields a structurally identical document apart from `$schema`, only that literal is replaced. Otherwise the candidate uses the same `serde_json::to_string_pretty` plus trailing newline that the access writer already uses.

## Task stack

- [x] T01: `Derive a series-scoped $schema rule from .version in the Pkl schema` (status:done)
  - Task ID: T01
  - Scope:
    - In:
      - Add `config/pkl/base/sce-config-schema-url.pkl`. From a version string, it produces the concrete URL and the series `pattern`, with `major.minor` escaped. The pattern accepts patch `0|[1-9][0-9]*`, optional `-` prerelease, and optional `+` build, anchored to `^https://sce\.crocoder\.dev/v…/config\.json$`.
      - Switch `sce-config-schema.pkl` to use the helper. `$id` uses the concrete URL. The `$schema` property uses `type: string` plus the `pattern`, and a description that names the series.
      - Add fixture assertions to `generation-contract-check.pkl` for `0.4.0-pre-alpha-v14`, `0.5.0`, and `1.4.2`.
      - Replace `accepts_versioned_schema_declaration` in `schema.rs` with an embedded-schema matrix built from `PACKAGE_VERSION`'s series. The matrix accepts same-series stable, prerelease, and build declarations. It rejects other series, malformed values, and foreign values. It keeps the case where `$schema` is valid but another field is invalid.
    - Out: the Rust classifier (T02) and historical snapshots.
  - Dependencies: none
  - Done when:
    - The generated schema's `$schema` property is a series pattern derived from `.version`.
    - The Pkl fixtures prove that a different `.version` series yields a different pattern.
    - The embedded-schema matrix passes against the current series with no literal `0.4` outside fixtures.
  - Verify:
    - `nix run .#pkl-check-generated`
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema`
  - Completed: 2026-11-10
  - Files changed:
    - `config/pkl/base/sce-config-schema-url.pkl` (new)
    - `config/pkl/base/sce-config-schema.pkl`
    - `config/pkl/renderers/generation-contract-check.pkl`
    - `cli/src/services/config/schema.rs`
  - Result:
    - The new helper exposes `concreteUrl`, `series`, and `seriesPattern`, and throws on a non-semver version.
    - The generated `$schema` property is now `type: string` with an anchored `^https://sce\.crocoder\.dev/v<major>\.<minor>\.(0|[1-9][0-9]*)(-prerelease)?(+build)?/config\.json$` pattern. Its description names the series. `$id` is still the concrete URL.
    - New contract check `config-schema-series-pattern` covers `0.4.0-pre-alpha-v14`, `0.5.0`, and `1.4.2`. It also asserts that the generated schema has no exact-version `const`.
    - `accepts_versioned_schema_declaration` is replaced by module `schema_declaration_acceptance_tests`, with 4 tests built from the series in `PACKAGE_VERSION`: same-series acceptance; rejection of other series; rejection of malformed, foreign, and non-string values; and the case where `$schema` is valid but another field is invalid.
    - Per a user instruction during the run, the new Pkl helper has no comments.
  - Verify:
    - `nix run .#pkl-check-generated`: passed (145 files; contract checks include `config-schema-series-pattern`).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema`: passed (29 passed, 0 failed).
    - Supplementary checks:
      - `cargo fmt --check`: passed.
      - `rg -n '0\.4'` over the schema Pkl files and `schema.rs`: no matches.
  - Context impact: localized. The generated config schema now accepts the whole series. `context/sce/config-schema-publication.md` still describes the exact-version `const` until T08 updates it.
  - Context synchronization: synced

- [x] T02: `Add typed schema-declaration classification and actionable $schema errors` (status:done)
  - Task ID: T02
  - Scope:
    - In:
      - Add `cli/src/services/config/schema_declaration.rs` with these types:
        - `SchemaVersion`: major, minor, and patch, plus a prerelease and build that are parsed but not compared.
        - `SchemaSeries`.
        - `current_schema_series()`, parsed from `PACKAGE_VERSION`.
        - `classify_schema_declaration(value: Option<&Value>, current: SchemaSeries) -> SchemaDeclaration`, returning `Absent | CurrentSeries | OlderSeries(SchemaVersion) | NewerSeries(SchemaVersion) | Unrecognized`.
      - Add a parity test. For the same URL matrix, embedded-schema acceptance of `{"$schema": url}` must equal `CurrentSeries`. Add classifier tests with an injected `0.5` series. They show that `0.4.0-pre-alpha-v1`, `0.4.7`, and `0.3.3` are `OlderSeries`, and that `1.4.0` is `NewerSeries`.
      - In `validate_config_value_against_schema`, replace the generic `/$schema` pattern error with a message for each class:
        - Older: "declares SCE config schema series X.Y; this CLI uses A.B; run 'sce setup' to upgrade the repository and global config, or set `$schema` to <current url>".
        - Newer: "… was written by a newer SCE release; upgrade the SCE CLI".
        - Unrecognized: "… is not a canonical SCE schema URL; use <current url>".
      - Keep every other schema error and the sorted join.
      - In `app_support::invalid_discovered_config_guidance`, mention `sce setup` when the only failure is an older-series declaration.
    - Out: any write path, and setup wiring.
  - Dependencies: T01
  - Done when:
    - The classifier and embedded schema agree on the full matrix.
    - `sce config validate` on a file declaring `0.3.3`, `0.5.0`, or a foreign URL reports the matching message, and the file is unchanged.
    - Other field errors are still reported alongside the `$schema` message.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_declaration`
    - `… test … config`
  - Completed: 2026-11-10
  - Files changed:
    - `cli/src/services/config/schema_declaration.rs` (new)
    - `cli/src/services/config/mod.rs`
    - `cli/src/services/config/schema.rs`
    - `cli/src/services/app_support.rs`
  - Result:
    - New `schema_declaration` module defines:
      - `SchemaVersion`, with a typed parser that mirrors the Pkl `seriesPattern` grammar, plus `series()` and `release()`.
      - `SchemaSeries`.
      - `current_schema_version()` and `current_schema_series()`, both parsed from `PACKAGE_VERSION`.
      - `classify_schema_declaration`, returning `Absent | CurrentSeries | OlderSeries | NewerSeries | Unrecognized`.
      - `schema_declaration_issue`.
      - `is_older_series_only_failure`.
    - `validate_config_value_against_schema` collapses every `/$schema` schema error into one classified issue. Other errors and the sorted ` | ` join are unchanged.
    - `invalid_discovered_config_guidance` delegates to `discovered_config_guidance`. When every validation error is only an older-series declaration, the guidance names `sce setup`, for both the local and the discovered-file wording.
    - Deviation (user instruction during the run): the messages print full `major.minor.patch` versions rather than `X.Y` series. Prerelease and build are dropped. Final wording:
      - Older: `/$schema: declares SCE config schema version 0.3.3; this CLI uses 0.4.0 and accepts 0.4.x; run 'sce setup' to upgrade the repository and global config, or set `$schema` to <current url>`.
      - Newer: `… version 0.5.0, which was written by a newer SCE release; this CLI uses 0.4.0 and accepts 0.4.x; upgrade the SCE CLI`.
      - Unrecognized: `/$schema: is not a canonical SCE schema URL; use <current url>`.
    - Fixed a T01 test helper (`declaration_accepted` now takes `&Value`) to clear the clippy `needless_pass_by_value` failure under `--all-targets -D warnings`.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_declaration`: passed (13 passed, 0 failed). This includes the classifier/embedded-schema parity matrix, the injected-`0.5` classifier tests, and the guidance tests.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config`: passed (109 passed, 0 failed).
    - Supplementary checks:
      - `clippy --all-targets -- -D warnings`: passed.
      - `cargo fmt --check`: passed.
      - Behavioral check: `sce config validate` against a discovered `.sce/config.json` declaring `0.3.3`, `0.5.0`, or `https://example.com/s.json` printed the matching message, and a sha256 check confirmed each file was unchanged. `0.3.3` plus `log_level: "loud"` reported both issues.
  - Context impact: localized. `$schema` validation messages and the startup guidance changed. `context/cli/config-precedence-contract.md` does not yet describe the actionable messages; T08 updates it.
  - Context synchronization: synced

- [x] T03: `Extract the guarded config-file write shared by setup writers` (status:done)
  - Task ID: T03
  - Scope:
    - In:
      - Move the `ConfigLayer` enum and the read-original, reread-compare, symlink-refusal (`fs::symlink_metadata`), and `write_atomically` sequence into `cli/src/services/setup/config_file_write.rs`. Expose them through a typed `GuardedWriteError { ConcurrentModification, SymlinkManaged, Io(…) }` and a `write_guarded(path, layer, original: Option<&[u8]>, serialized: &[u8], reread)` function.
      - Port `access_preferences.rs` to use the new module with byte-identical behavior and messages.
    - Out: any new behavior.
  - Dependencies: none
  - Done when:
    - `access_preferences` uses the shared helper.
    - All existing access-preference tests pass unchanged, including the symlink, dangling-symlink, concurrent-modification, invalid-file, and mode-preservation tests.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_preferences`
    - `… test … setup`
  - Completed: 2026-11-10
  - Files changed:
    - `cli/src/services/setup/config_file_write.rs` (new)
    - `cli/src/services/setup/access_preferences.rs`
    - `cli/src/services/setup/mod.rs`
  - Result:
    - New `config_file_write` module (`pub(super)`) owns:
      - `ConfigLayer`, with `path_source`, `label`, and `bootstrap_value`;
      - `read_optional`;
      - the private `is_symlink`;
      - `GuardedWriteError { ConcurrentModification, SymlinkManaged, Io(anyhow::Error) }`;
      - `write_guarded(path, layer, original: Option<&[u8]>, serialized: &[u8], reread)`. It rereads and compares against `original`, refuses a symlink through `fs::symlink_metadata`, then calls `write_atomically`, in the same order as before.
    - `access_preferences.rs` now uses the shared helper. Its private `WriteFailure` converts from `GuardedWriteError` through `From`. Every public type, error variant, and message is unchanged. The test bodies are unchanged; only the test module imports moved.
    - `atomic_write.rs` is unchanged, and `claude_telemetry.rs` still uses it directly.
    - Deviation: the done check names a dangling-symlink test, but `access_preferences` had none. Three helper tests were added in `config_file_write` instead: creating a missing file, refusing a file created after planning, and refusing a dangling symlink while leaving it intact.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_preferences`: passed (7 passed, 0 failed). This covers the symlink, concurrent-modification, invalid-file, and mode-preservation tests.
    - `… test … setup`: passed (81 passed, 0 failed).
    - Supplementary checks:
      - `… test … config_file_write`: passed (3 passed, 0 failed).
      - `clippy --all-targets -- -D warnings`: passed.
      - `cargo fmt --check`: passed.
  - Context impact: localized. This is an internal refactor of setup's write helpers with no behavior change.
  - Context synchronization: synced

- [x] T04: `Plan and commit a validated $schema-only upgrade for one config file` (status:done)
  - Task ID: T04
  - Scope:
    - In:
      - Add `cli/src/services/setup/schema_upgrade.rs`. It defines:
        - `plan_schema_upgrade(path, layer) -> SchemaUpgradePlan`, with variants `NotPresent | NoDeclaration | Current | NewerSeries(version) | Unrecognized | Unreadable(reason) | Ready { original, candidate, from }` and `CandidateInvalid { from, reason }`.
        - `commit_schema_upgrade(path, layer, plan) -> SchemaUpgradeOutcome`, with variants `Upgraded { from } | Unchanged | Skipped(SchemaUpgradeSkip)`.
        - A typed `SchemaUpgradeSkip { NewerSeries, Unrecognized, CandidateInvalid, SymlinkManaged, ConcurrentModification, Io }`.
      - Planning reads the file, classifies `$schema` with T02, and builds the candidate. The candidate uses the byte-preserving literal replacement, with a structural-equality guard and a pretty-serialization fallback. Planning validates the candidate with `parse_file_config` and the layer's `ConfigPathSource`. Planning never writes.
      - Committing calls T03's `write_guarded` against the planned original bytes.
      - Unit tests cover:
        - absent file;
        - absent `$schema`;
        - same-series patch and prerelease values preserved;
        - `0.3.3` upgraded with all other bytes preserved, using a fixture that contains `auth.mode`, `telemetry.enabled`, `integrations`, and custom `policies.bash`;
        - a second plan returning `Current`;
        - newer and foreign values untouched;
        - an older file with an unknown key yielding `CandidateInvalid` with a byte-identical file;
        - symlink and dangling-symlink refusal;
        - concurrent modification between plan and commit;
        - mode preservation;
        - the global layer validated as `DefaultDiscoveredGlobal`.
    - Out: setup ordering and reporting.
  - Dependencies: T02, T03
  - Done when:
    - Each declaration shape in the change summary table produces the specified plan and outcome.
    - Only the `Ready` plan writes, and it writes only after successful validation.
    - All protections hold.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_upgrade`
  - Completed: 2026-11-10
  - Files changed:
    - `cli/src/services/setup/schema_upgrade.rs` (new)
    - `cli/src/services/setup/mod.rs`
  - Result:
    - New `schema_upgrade` module defines:
      - `SchemaUpgradePlan { NotPresent, NoDeclaration, Current, NewerSeries(SchemaVersion), Unrecognized, Unreadable(anyhow::Error), Ready { original, candidate, from }, CandidateInvalid { from, reason } }`;
      - `SchemaUpgradeOutcome { Upgraded { from }, Unchanged, Skipped(SchemaUpgradeSkip) }`;
      - `SchemaUpgradeSkip { NewerSeries, Unrecognized, CandidateInvalid { from, reason }, SymlinkManaged, ConcurrentModification, Io }`, which converts from `GuardedWriteError`.
    - `plan_schema_upgrade(path, layer)` reads the file with `read_optional`. It classifies `$schema` with `classify_schema_declaration` against `current_schema_series()`. For an older series, it builds the candidate and validates it with `parse_file_config` and `layer.path_source()`. It never writes.
    - Building the candidate: the JSON-encoded old URL literal is swapped for the current URL only when it occurs exactly once and the reparsed result equals the original with only `$schema` replaced. Otherwise the candidate falls back to `to_string_pretty` plus a trailing newline.
    - `commit_schema_upgrade(path, layer, plan)` writes only for `Ready`, through `write_guarded` against the planned original bytes. `NotPresent`, `NoDeclaration`, and `Current` map to `Unchanged`. The other plans map to `Skipped`.
    - Approved assumptions:
      - `Unreadable` (a read failure, non-UTF-8, non-JSON, or a non-object) maps to `Skipped(Io)` rather than adding a new skip variant.
      - A dangling symlink reads as `NotPresent`. It yields `Unchanged`, and the link is left intact; there is nothing to upgrade.
    - `mod schema_upgrade` carries `#[expect(dead_code, reason = …)]` until T05/T06 consume the planner and its skip details. Because it is an `expect`, the build fails once the module is fully used, which forces its removal.
    - 13 unit tests cover:
      - a missing file and no declaration;
      - same-series prerelease, patch, and build values left unchanged;
      - older-series `0.3.3`/`0.1.0` upgrades that change only the `$schema` bytes, using a fixture with `auth.mode`, `telemetry.enabled`, `integrations.target`, and custom `policies.bash` and irregular whitespace;
      - a second plan returning `Current` and leaving the file byte-identical;
      - the ambiguous-literal fallback;
      - newer-series, foreign, unversioned, `v0.4`, `http://`, and non-string values left untouched;
      - unreadable files;
      - an unknown key yielding `CandidateInvalid` with the file byte-identical;
      - the global layer validated as `DefaultDiscoveredGlobal`;
      - concurrent modification between plan and commit;
      - symlink and dangling-symlink handling;
      - an atomic write that keeps the file mode.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml schema_upgrade`: passed (13 passed, 0 failed).
    - Supplementary checks:
      - `… test … setup`: passed (94 passed, 0 failed), including `access_preferences` and `config_file_write`.
      - `clippy --all-targets -- -D warnings`: passed.
      - `cargo fmt --check`: passed.
  - Context impact: localized. This adds a new internal setup module that is not yet wired into the `sce setup` flow, so user-visible behavior is unchanged until T05 and T06.
  - Context synchronization: synced

- [x] T05: `Run schema upgrades before setup's preflight readers` (status:done)
  - Task ID: T05
  - Scope:
    - In:
      - In `SetupCommand::execute`, plan upgrades after `ensure_git_repository`, unless the run is context-only. The local path comes from `RepoPaths::sce_config_file()`. The global path comes from `resolve_default_global_config_path()` and is planned only if the file exists; if the path does not resolve, setup skips the global upgrade.
      - Make `resolve_agent_trace_storage_runtime_config_with` `pub(crate)`. Call it with a `read_file` overlay that serves `Ready` candidate bytes for exactly those two paths. The named-remote preflight then sees the validated candidate, and the resolver itself stays unchanged.
      - After the remote preflight passes, commit the plans and keep the outcomes.
      - Run `prepare_dispatch`, the access writer, lifecycle bootstrap, and target persistence after the commit, so they read the upgraded files.
      - Split `resolve_setup_repository` as needed. Never pass an explicit config path to the planner.
      - Add tests:
        - A stale local file with `agent_trace.repository_remote: "upstream"` in a repo that has only `upstream` passes the preflight and is upgraded.
        - The same repo with a failing remote preflight writes nothing.
        - A stale global file with `auth.mode: "disabled"` locks the access gate after the upgrade.
        - A stale local file's saved `auth.mode`, telemetry, targets, and optional workflows drive dispatch.
        - A missing global file is not created.
        - Context-only mode writes nothing.
        - With an invalid stale local file and a valid stale global file, the global file is upgraded and the local file stays byte-identical, with prompts skipped as today.
    - Out: report rendering (T06).
  - Dependencies: T04
  - Done when: AC8 holds and the existing `setup` tests pass.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`
  - Completed: 2026-11-10
  - Files changed:
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/schema_upgrade.rs`
    - `cli/src/services/config/resolver.rs`
  - Result:
    - `schema_upgrade.rs` adds:
      - `CanonicalSchemaUpgrades`, which plans the repository file and, when its path resolves, the global file through `plan_schema_upgrade`. A missing file plans as `NotPresent` and is never created.
      - `candidate(path)`, which returns the `Ready` candidate bytes for a planned path.
      - `commit(self) -> Vec<SchemaUpgradeReport { path, outcome }>`.
    - `resolve_setup_repository` now takes `context_only` and delegates to `resolve_setup_repository_with`, which takes injectable env lookup and global-path resolvers. It runs these steps in order:
      1. `ensure_git_repository`.
      2. Plan the upgrades, which is skipped in context-only mode. Only the canonical local and global paths are planned; an explicit config path never is.
      3. `resolve_agent_trace_storage_runtime_config_with`, now `pub(crate)`, with a `read_file` overlay that serves candidate bytes for the planned paths.
      4. `ensure_git_remote`.
      5. Commit the upgrades.
    - It returns a `SetupRepository { root, schema_upgrades }`. `execute` then runs `prepare_dispatch` and every later reader or writer against the committed files.
    - `SetupRepository.schema_upgrades` carries `#[cfg_attr(not(test), expect(dead_code, …))]` until T06 renders it. The module-level `expect(dead_code)` on `schema_upgrade` stays, because the skip details are still unread.
    - 7 new tests in `setup::command::tests`:
      - The remote preflight uses `upstream` from a stale local file in an `upstream`-only repo. The file is upgraded with only `$schema` changed, and the global file is not created.
      - A failing remote preflight leaves both stale files byte-identical.
      - A failing Git gate leaves the stale global file byte-identical.
      - A stale global `auth.mode: disabled` does not lock the gate before the upgrade, locks it as `Global` after the upgrade, and suppresses the access prompts.
      - A stale local file's targets, optional workflows, and saved access answers drive dispatch after the upgrade: only `login` is prompted, and the access write is `Unchanged`.
      - Context-only mode plans and writes nothing.
      - With an invalid stale local file and a valid stale global file, only the global file is upgraded, the local file stays byte-identical, and prompts are skipped as today.
    - Approved assumption: the global file is passed to the planner whenever its path resolves. `NotPresent` covers "planned only if the file exists" without a separate existence check.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`: passed (101 passed, 0 failed).
    - Supplementary checks:
      - `… test … schema_upgrade`: passed (16 passed, 0 failed).
      - `clippy --all-targets -- -D warnings`: passed.
      - `cargo fmt --check`: passed.
  - Context impact: localized. The `sce setup` ordering now upgrades stale canonical `$schema` declarations before the remote preflight and before dispatch, which changes setup behavior for stale files. Setup output is unchanged until T06, and durable documentation is scheduled for T08.
  - Context synchronization: synced

- [x] T06: `Report schema upgrade outcomes in setup output` (status:done)
  - Task ID: T06
  - Scope:
    - In:
      - Render a `Config schema:` section as the first setup section. It renders only when some file is `Upgraded` or `Skipped`, with one line for each file, using fixed wording:
        - Upgraded: `upgraded '<path>' from v<from> to v<current>`.
        - Newer series: `left '<path>' unchanged; it declares newer schema series X.Y; upgrade the SCE CLI`.
        - Unrecognized: `left '<path>' unchanged; '$schema' is not a canonical SCE schema URL; set it to <url>`.
        - Candidate invalid: `left '<path>' unchanged; upgrading '$schema' to v<current> would fail validation: <errors>. Fix the listed fields, then rerun 'sce setup'.`
        - Symlink-managed or concurrent: the existing wording style, telling the user to update the source directly or rerun.
      - Add renderer unit tests for each line.
      - The exit code stays successful.
    - Out: changes to other sections.
  - Dependencies: T05
  - Done when:
    - Every non-silent outcome renders its fixed line.
    - Silent outcomes (absent file, no declaration, current series) render nothing.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`
  - Completed: 2026-11-10
  - Files changed:
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/schema_upgrade.rs`
    - `cli/src/services/setup/mod.rs`
  - Result:
    - `render_schema_upgrade_section(&[SchemaUpgradeReport]) -> Option<String>` in `command.rs` renders a `Config schema:` heading and one `- `-prefixed line for each `Upgraded` or `Skipped` file, using the fixed wording. It returns `None` when every outcome is `Unchanged`, which covers an absent file, no declaration, and the current series.
    - `SetupCommand::execute` puts the section first, before the context baseline message. Context-only mode plans no upgrades, so it never renders the section.
    - Wording:
      - Upgraded: `upgraded '<path>' from v<from> to v<current_schema_version()>`.
      - Newer series: `version.series()` gives the `X.Y` series.
      - Unrecognized: the line names `sce_config_schema_url()`.
      - Symlink-managed: `it is a symlink; SCE cannot modify a symlink-managed config. Update the file's source directly.`
      - Concurrent modification: `it changed while setup was upgrading '$schema'. Rerun 'sce setup'.`
    - The setup exit code is unchanged (successful).
    - Removed the `dead_code` expects on `SetupRepository.schema_upgrades` and on `mod schema_upgrade`.
    - 7 new tests in `setup::command::tests`:
      - the section is omitted for no reports or only `Unchanged` reports;
      - the upgraded line, with `Unchanged` files skipped;
      - the newer-series line;
      - the unrecognized line;
      - the candidate-invalid line;
      - the symlink, concurrent-modification, and I/O lines;
      - real setup resolution with an invalid stale local file and a valid stale global file, which renders both lines in order.
    - Approved assumptions:
      - `Skipped(Io)`, which covers unreadable or non-JSON files and I/O write failures, renders `left '<path>' unchanged; setup could not upgrade '$schema': <error>`. The plan gave this case no fixed wording.
    - Deviations:
      - `from` was removed from `SchemaUpgradePlan::CandidateInvalid` and `SchemaUpgradeSkip::CandidateInvalid`. The fixed candidate-invalid wording does not use it, and once the expects were removed it was dead code that failed `clippy -D warnings`. The T04 test drops its `from` assertion.
      - When the user cancels an interactive prompt, the section is also put before the cancellation text, because upgrades are committed before dispatch prompts. That text still reads "No files were changed." Its wording was left unchanged because other sections are out of scope.
      - `execute` carries `#[allow(clippy::too_many_lines)]`, matching `config/resolver.rs` and `config/render.rs`.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`: passed (108 passed, 0 failed).
    - Supplementary checks:
      - `… test … schema_upgrade`: passed (23 passed, 0 failed).
      - `clippy --all-targets -- -D warnings`: passed.
      - `cargo fmt --check`: passed.
  - Context impact: localized, user-visible. `sce setup` output now starts with a `Config schema:` section whenever a canonical file's `$schema` is upgraded or skipped. Durable documentation of the section is scheduled for T08.
  - Context synchronization: synced

- [x] T07: `Add an end-to-end setup schema-upgrade scenario check` (status:done)
  - Task ID: T07
  - Scope:
    - In:
      - Add `scripts/test-setup-schema-upgrade.sh`, following the isolation pattern in `scripts/stress-test-conversation-trace-firehose-mixed.sh`: a built `sce` binary, a temporary Git repo with a dummy `origin`, and isolated `HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME`. Wire it into `flake.nix` as a `checks` entry.
      - The script runs these phases:
        1. **Read-only.** With `0.3.3` declarations in both the global and local files, and in a newer and a foreign variant, `sce version`, `sce config validate`, `sce config show`, `sce doctor`, and `sce doctor --fix` leave file hashes unchanged and print the `$schema` messages.
        2. **Upgrade.** `sce setup --hooks --repo <tmp>` upgrades both files. The diff against the originals is exactly the `$schema` line, and `sce config validate` then passes.
        3. **Idempotency.** A second run leaves both files byte-identical and prints no `Config schema:` section.
        4. **Mixed.** With a stale invalid local file and a stale valid global file, the global file is upgraded and the local file is byte-identical, and the report names the local file's errors.
        5. **Newer.** A newer-series local file stays byte-identical.
        6. **Symlink.** A symlinked global config is reported and left untouched.
    - Out: VM or npm scenarios.
  - Dependencies: T06
  - Done when: the script passes locally and under `nix flake check`, covering AC3 through AC7 end to end.
  - Verify:
    - `nix develop -c ./scripts/test-setup-schema-upgrade.sh`
    - `nix flake check`
  - Completed: 2026-11-10
  - Files changed:
    - `scripts/test-setup-schema-upgrade.sh` (new, staged intent-to-add so the flake source includes it)
    - `flake.nix`
  - Result:
    - The new script runs a built `sce` (`--sce-bin`, `$SCE_BIN`, or `sce` on `PATH`). It isolates `HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, and the Git global config. Each phase uses a fresh temporary Git repo with an `https://example.com/acme/demo.git` `origin`.
    - The global config path follows the CLI's platform resolution: `$XDG_CONFIG_HOME/sce/config.json`, or `~/Library/Application Support/sce/config.json` on Darwin.
    - The current version, current URL, and newer series (`major.(minor+1).0`) are read from `sce version`. The only literal series in the script is the `0.3.3` fixture.
    - Phases:
      1. **Read-only.** For older (`0.3.3`), newer, and foreign declarations in both files, `sce version`, `config validate`, `config show`, `doctor`, and `doctor --fix` leave both sha256 hashes unchanged. Every command except `version` prints the matching `$schema` message.
      2. **Upgrade.** `sce setup --hooks --repo` reports both `upgraded …` lines. Each file equals its original with only the URL swapped, with exactly one changed line, using a fixture with `auth.mode`, `telemetry.enabled`, `integrations.target`, custom `policies.bash`, and irregular whitespace. `sce config validate` then reports valid.
      3. **Idempotency.** A second run prints no `Config schema:` section and leaves both hashes unchanged.
      4. **Mixed.** With an unknown-key stale local file and a valid stale global file, the local file is byte-identical and its candidate-invalid line names `not_a_real_key` and `rerun 'sce setup'`. The global file is upgraded with only `$schema` changed.
      5. **Newer.** The local file is byte-identical, the newer-series line is reported, and no global config is created.
      6. **Symlink.** A global config symlinked to a `dotfiles` target stays a symlink with the same target and target bytes, and is reported with the symlink line.
    - `flake.nix` adds a `setupSchemaUpgradeCheck` `runCommand`, which runs the script against `scePackage` with `bash`, `coreutils`, `diffutils`, `gnugrep`, `gnused`, and `git`, and exposes it as `checks.<system>.setup-schema-upgrade`.
    - Approved assumptions:
      - Read-only commands may exit non-zero on a stale config. The script asserts on their output and hashes, not their exit codes. Setup must exit 0.
      - `nix develop -c ./scripts/test-setup-schema-upgrade.sh` needs a built `sce` on `PATH` or `SCE_BIN`. The dev shell does not provide one, so the local run put `cli/target/debug` on `PATH`.
  - Verify:
    - `nix develop -c ./scripts/test-setup-schema-upgrade.sh` (with `cli/target/debug` on `PATH`): passed (all 6 phases).
    - `nix flake check`: passed ("all checks passed!", running on x86_64-linux only; Darwin and aarch64 systems were omitted by default).
    - Supplementary checks:
      - `nix build .#checks.x86_64-linux.setup-schema-upgrade -L`: passed in the Nix sandbox (all 6 phases).
      - Negative control: against a stale pre-T02 `cli/target/release/sce` (exact-version `const`), the script failed at the first read-only assertion, as expected.
  - Context impact: localized. This adds a test-only script and a flake check, with no runtime behavior change. A new check name now appears in `nix flake check`.
  - Context synchronization: synced

- [ ] T08: `Document series compatibility and setup schema upgrades` (status:todo)
  - Task ID: T08
  - Scope:
    - In:
      - Update the four context files listed under Context sync. This includes fixing the drifted unversioned schema URL line in `config-precedence-contract.md`, and replacing "The global config is only read" in `setup-repo-local-config-bootstrap.md` with the `$schema`-only global write.
      - Add the glossary entry.
      - Add an ADR in `context/decisions/` titled roughly `setup-upgrades-config-schema-declarations`. It records:
        - acceptance by series;
        - that setup is the only writer;
        - the global write limited to `$schema`;
        - no downgrade;
        - no field migrations;
        - non-fatal reporting;
        - historical snapshots left unchanged, with the next `nix run .#bump-version` publishing the first series-pattern snapshot.
    - Out: README and npm docs. Neither currently documents `$schema`.
  - Dependencies: T07
  - Done when:
    - Context describes the implemented behavior with no remaining claim of an exact-version `const`, a read-only global config, or an unversioned declaration.
    - The ADR is linked from the setup bootstrap and schema publication files.
  - Verify:
    - `nix shell nixpkgs#ripgrep -c rg -n 'const.*config.json|global config is only read|sce.crocoder.dev/config.json' context --glob '!context/plans/**' --glob '!context/decisions/**'` returns no stale claims.
  - Context synchronization: pending

## Open questions

None. The one judgment call is that schema-upgrade failures are reported but
not fatal. It is recorded under Assumptions with its rationale. It follows the
documented degraded-invalid-config setup behavior. Revise the plan if setup
should instead exit non-zero when a stale canonical file cannot be upgraded.
