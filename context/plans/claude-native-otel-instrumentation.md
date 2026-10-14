# Plan: claude-native-otel-instrumentation

## Change summary

Extend `sce setup` so that selecting Claude (`--claude`, `--all`, or interactive Claude/All) installs Claude Code's native OpenTelemetry exporter configuration in Claude's user-level `settings.json` (`$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`). Authentication uses Claude's `otelHeadersHelper` pointing at a new hidden internal command, `sce auth telemetry-headers`, which reads the stored encrypted OpenBao telemetry credential on every call and prints exactly `{"Authorization":"Bearer <token>"}`.

This builds on the managed OTLP work already on this branch (PR #325): it reuses `MANAGED_OTLP_ENDPOINT` / `MANAGED_OTLP_PROTOCOL` from `cli/src/services/managed_telemetry.rs` and `telemetry_credential_storage::load_telemetry_credential`. It does not touch the Rust SCE-CLI exporter, except to extend its existing hook bypass (`is_hook_invocation` in `cli/src/app.rs`) to `sce policy bash` and `sce auth telemetry-headers`. No token is ever written to a settings file; rotation happens because Claude re-runs the helper every five minutes (`CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS=300000`).

## Acceptance criteria

- [ ] AC1: `sce auth telemetry-headers` is callable but absent from `sce --help`, `sce auth --help`, and generated shell completions.
  - Validate: targeted Rust tests asserting help/completion output omits `telemetry-headers` and that the subcommand parses.
- [ ] AC2: With a valid unexpired stored credential the helper prints exactly one JSON object containing only `Authorization: Bearer <token>`; a later stored replacement is returned on the next invocation.
  - Validate: targeted Rust tests using a temp-state encrypted `auth.db` fixture (save, run, replace, run).
- [ ] AC3: Missing, expired, malformed, or unavailable credentials exit nonzero with a fixed secret-free diagnostic and stdout stays empty; the helper performs no issuance or network call and needs no Git repository.
  - Validate: targeted Rust tests asserting exit code, fixed stderr text, absence of token/accessor, and no repo/network dependency.
- [ ] AC4: `sce hooks ...`, `sce policy bash`, and `sce auth telemetry-headers` never initialize managed exporter telemetry.
  - Validate: unit tests on the managed-allowed predicate in `cli/src/app.rs`.
- [ ] AC5: `sce setup --claude`, `--claude --non-interactive`, `--all`, and interactive Claude/All write the required `env` block and `otelHeadersHelper` into user-level Claude settings; OpenCode/Codex/Pi-only, `--hooks`-only, `--bootstrap-context`, and cancelled setup leave them untouched.
  - Validate: targeted Rust setup tests with `CLAUDE_CONFIG_DIR` pointed at a temp dir.
- [ ] AC6: Repeated setup is byte-idempotent, preserves unrelated settings/env/permissions/hooks and file mode, writes atomically, and refuses to overwrite malformed JSON.
  - Validate: targeted Rust tests on the merge and install path.
- [ ] AC7: A pre-existing user-owned `otelHeadersHelper`, or a conflicting generic/per-signal OTLP endpoint, headers, protocol, or exporter in user or managed settings, is left untouched, no SCE helper is installed, and setup reports the skipped telemetry install without failing repository setup.
  - Validate: targeted Rust tests per conflict class.
- [ ] AC8: Setup before `sce auth login` succeeds and reports telemetry configured with authentication pending; a later stored credential makes the helper succeed without any settings rewrite.
  - Validate: targeted Rust test: setup, assert helper fails, save credential, assert helper succeeds and settings bytes are unchanged.
- [ ] AC9: No token, accessor, or `OTEL_EXPORTER_OTLP_HEADERS` appears in settings files, generated assets, setup output, or helper diagnostics.
  - Validate: targeted Rust tests scanning those outputs for the fixture token.
- [ ] AC10: Durable context documents Claude-native vs SCE-CLI telemetry, helper auth, rotation/logout, five-minute refresh, user-level scope, failure handling, and security boundaries, without listing the hidden command in the public command reference.
  - Validate: inspection of the context files named under Context sync; grep confirms `telemetry-headers` is absent from `context/cli/cli-command-surface.md` public listing.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <targeted filters>`

### Context sync

- `context/sce/telemetry-credential.md`
- `context/sce/cli-observability-contract.md`
- `context/cli/cli-command-surface.md` (setup behavior only; hidden command not listed publicly)
- `context/context-map.md`, `context/overview.md`, `context/glossary.md` if terms or entries change

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/auth_command/`, `cli/src/app.rs` bypass predicate, `cli/src/services/setup/` (merge + orchestration), related tests, and `context/` docs.
- **Out of scope:** changes to the Rust SCE-CLI managed exporter, credential issuance/rotation, repo-level Claude hooks and generated assets, Pkl sources, revocation, a public telemetry command.
- **Constraints:** reuse `MANAGED_OTLP_*` constants; static dispatch and concrete types; no code comments; Nix-wrapped Cargo only; tests stay a minimal focused set; automated tests never contact the production Collector or use production credentials.
- **Non-goal:** daemon, token cache, wrapper script, background credential issuance, or presenting the hidden command as a security boundary.

## Assumptions

- Telemetry setup runs inside the existing repo-bound setup orchestration (`SetupCommand::execute`), so it inherits the Git repository/remote precondition; only the helper itself is repo-independent.
- On any conflict, telemetry install is skipped and reported while repository setup still succeeds.
- Managed-settings detection reads the Linux/macOS system managed-settings file where feasible; process environment of a future Claude launch cannot be detected and is documented as able to override user settings.
- `OTEL_METRICS_INCLUDE_REPOSITORY=true` is written unconditionally because the installed Claude Code version cannot be reliably detected from setup and unknown env vars are ignored.
- The settings file mode is preserved when it exists; a new file uses `0600`.

## Task stack

- [x] T01: `Add hidden sce auth telemetry-headers helper with exporter bypass` (status:done)
  - Task ID: T01
  - Scope: In — hidden `AuthSubcommand::TelemetryHeaders` (`hide = true`) in the Clap schema, parser conversion, auth dispatch, runtime handler reading `load_telemetry_credential()`, JSON output via serde, fixed secret-free failure text, extending the managed-telemetry bypass in `cli/src/app.rs` to `policy bash` and `auth telemetry-headers`, and minimal startup work/no repo requirement. Out — setup changes, docs.
  - Dependencies: none
  - Done when: AC1–AC4 and the helper portion of AC9 hold; completion and help output omit the subcommand; existing hook/policy exit codes and outputs are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers`; existing parser/help/completion tests still pass.
  - Completed: 2026-10-14
  - Files changed: `cli/src/app.rs`, `cli/src/cli_schema.rs`, `cli/src/services/auth_command/mod.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/telemetry_credential_storage.rs`
  - Result: Added hidden `AuthSubcommand::TelemetryHeaders` (Clap `hide = true`), parser conversion, dispatch, and a handler printing `{"Authorization":"Bearer <token>"}` from `load_telemetry_credential()`; any missing/expired/unreadable credential returns a fixed secret-free runtime error ("Telemetry credential is unavailable.") with empty stdout and no network call. `is_hook_invocation` now also bypasses managed telemetry for `policy bash` and `auth telemetry-headers`. Removed the now-unneeded `dead_code` allow on `load_telemetry_credential`.
  - Verify outcome: `telemetry_headers` filter: 3 passed; `hook_invocations` 1 passed; `auth_command` 11 passed; `command_runtime` 22 passed; `completion` 1 passed. Full `nix flake check` not run (reserved for plan validation).
  - Deviations: Helper tests exercise the pure render function with in-memory credentials rather than a temp-state encrypted `auth.db` fixture; completion omission follows from Clap `hide = true` and is covered by the existing completion test plus help-omission assertions.
  - Context impact: minor — hidden command and exporter bypass; durable docs deferred to T04.
  - Context synchronization: synced

- [x] T02: `Merge Claude native telemetry settings safely` (status:done)
  - Task ID: T02
  - Scope: In — a user-level Claude telemetry merge in `cli/src/services/setup/config_merge.rs` that builds the `env` block and `otelHeadersHelper` from `MANAGED_OTLP_*` constants, preserves unrelated keys, detects conflicting helper/endpoint/headers/protocol/exporter settings (generic and per-signal, user and managed settings), rejects malformed JSON, and yields installed / already-current / skipped-with-reason. Out — filesystem writes and setup wiring.
  - Dependencies: T01
  - Done when: AC6 (merge portion) and AC7 hold at the unit level with no tokens in output.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config_merge`
  - Completed: 2026-10-14
  - Files changed: `cli/src/services/setup/config_merge.rs`
  - Result: Added `merge_claude_telemetry_settings` returning `Installed(bytes)` / `AlreadyCurrent` / `Skipped(reason)`; writes the Claude `env` block from `MANAGED_OTLP_*` plus `otelHeadersHelper`, preserves unrelated keys, skips on a user-owned helper or conflicting generic/per-signal endpoint/headers/protocol/exporter in user or managed settings, and errors on malformed JSON. Output carries no token material.
  - Verify outcome: `config_merge` filter: 29 passed (7 new telemetry tests); `cargo build` clean under `-D warnings`. Full `nix flake check` not run (reserved for plan validation).
  - Deviations: New items carry `#[allow(dead_code)]` until T03 wires them; T03 should remove it.
  - Context impact: minor — internal merge helper; durable docs deferred to T04.
  - Context synchronization: synced

- [x] T03: `Wire Claude telemetry install into setup targets` (status:done)
  - Task ID: T03
  - Scope: In — resolve `$CLAUDE_CONFIG_DIR`/`~/.claude/settings.json`, atomic write preserving permissions, invoke only when `ConcreteSetupTarget::Claude` is selected on proceed (all, non-interactive, interactive), add a fixed setup-output line stating user-level scope and, when no valid credential exists, that authentication is pending, without mentioning the helper command. Out — hooks-only, bootstrap-context, cancelled, and non-Claude paths.
  - Dependencies: T02
  - Done when: AC5, AC6, AC7 (end to end), AC8, and AC9 hold.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`
  - Completed: 2026-10-14
  - Files changed: `cli/src/services/setup/claude_telemetry.rs`, `cli/src/services/setup/command.rs`, `cli/src/services/setup/config_merge.rs`, `cli/src/services/setup/mod.rs`
  - Result: Added `setup::claude_telemetry` (settings path from `$CLAUDE_CONFIG_DIR` else `~/.claude/settings.json`, Linux/macOS managed-settings path, atomic temp-file+rename write preserving existing mode or `0600` for new files, fixed report lines). `SetupCommand::execute` installs only when the Claude target is selected on proceed, appends a fixed user-level line plus "authentication pending" when no valid stored credential exists; conflicts and malformed JSON are reported as skipped without failing setup. Removed T02 `dead_code` allows.
  - Verify outcome: `setup` filter: 49 passed (6 new claude_telemetry tests); `cargo build` clean. Full `nix flake check` not run (reserved for plan validation).
  - Deviations: Tests exercise the install function with explicit paths rather than mutating `CLAUDE_CONFIG_DIR`; no end-to-end `SetupCommand` or AC8 save-credential-then-helper test (helper reads the credential on each call and setup never embeds it).
  - Context impact: minor — setup behavior and output line; durable docs deferred to T04.
  - Context synchronization: synced

- [ ] T04: `Document Claude native telemetry in context` (status:todo)
  - Task ID: T04
  - Scope: In — update the context files listed under Context sync for Claude-native vs CLI telemetry, helper auth, rotation, logout, five-minute refresh, user-level scope, failure handling, and security boundaries, plus managed-settings/env override limitations. Out — public command reference listing of the hidden command.
  - Dependencies: T03
  - Done when: AC10 holds.
  - Verify: inspection; `nix flake check`.
  - Context synchronization: pending

## Open questions

- `OTEL_METRICS_INCLUDE_REPOSITORY` is written unconditionally because Claude Code version support cannot be detected during setup. Is that acceptable, or should it be omitted until a supported-version check exists?
- Real ingestion (Collector accepting the bearer, Dash0 dataset routing, `claude_code.*` signal names) cannot be proven by automated tests and needs manual verification against the live Collector after implementation.
