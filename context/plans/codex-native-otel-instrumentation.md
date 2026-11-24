# Plan: codex-native-otel-instrumentation

## Change summary

Codex CLI exports no telemetry to the SCE managed collector today. Claude Code does: `sce setup` installs Claude-native OTLP settings with `otelHeadersHelper = "sce auth telemetry-headers"`, and Claude-invoked SCE hooks export `sce.hook` spans through the managed exporter. This plan brings Codex to the closest parity that Codex 0.172.0 supports. It reuses the existing managed telemetry infrastructure and adds no second telemetry architecture.

The plan extends three existing surfaces:

- The hidden helper `sce auth telemetry-headers` (`cli/src/services/auth_command/mod.rs`). Today `run_telemetry_headers` derives the repository from `std::env::current_dir()`. Codex runs `otel.headers_helper` from `CODEX_HOME`, so the working directory cannot identify the repository. The helper may need explicit repository context. Its no-argument Claude behavior must stay byte-identical.
- Codex launch configuration. Repository-local Codex config cannot set `otel.*`. Codex accepts process-local `-c` overrides. T01 decides with evidence whether a thin `sce codex` launcher is required, or whether a smaller mechanism is safe.
- Managed hook telemetry (`cli/src/app.rs::managed_span_kind`, `cli/src/services/managed_telemetry/trace_export.rs`). Today `ManagedSpanKind` is `{ Command, ClaudeHook }`, and every `sce hooks codex` and `sce hooks codex-mutation-scope` process resolves `NoopTelemetry`. This plan admits those two Codex hook commands to the same `sce.hook` / `sce.mutation_scope` / `sce.git_snapshot` / `sce.persistence` span contract. `TRACEPARENT` parenting applies only when T01 proves that Codex passes valid current-span context to command hooks.

Credential provisioning, the encrypted `auth.db`, repository enrollment, the effective access policy, non-interactive lookup, fail-closed helper behavior, Claude telemetry, and Codex hook behavior are preserved.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Durable context records evidence from real Codex 0.172.0 runs against a local OTLP receiver with dummy credentials. The evidence covers: the header-helper invocation contract (argv form, working directory, environment, stdout format, refresh cadence, failure recovery, header caching); configuration precedence across `-c`, profiles, user `config.toml`, and enterprise requirements; whether a helper-authorized header can reach a non-canonical exporter; attribution behavior for interactive, `exec`, `resume`, `fork`, working-directory changes, and multiple threads; whether command hooks receive `TRACEPARENT`/`TRACESTATE` that names an exported Codex span (distinguished from inherited process environment); the mapping between native session identifiers and SCE `cx_` session identities; the exported token, cost, cache, tool, duration, model, repository, and subagent attributes with their observed semantics; and which config options control prompt, response, and tool-argument content in native logs. The record ends with one implementation decision and a Claude/Codex parity matrix.
  - Validate: Inspect `context/sce/codex-native-telemetry-contract.md` for the evidence tables, the Codex version, the decision, and the parity matrix. `git ls-files` shows no probe script or receiver fixture. `rg -n 'Bearer [A-Za-z0-9._-]{12,}|hvs\.|s\.[A-Za-z0-9]{20,}' context/sce/codex-native-telemetry-contract.md` returns no match.
- [ ] AC2: `sce auth telemetry-headers` with no arguments behaves exactly as before for Claude. When the T01 decision requires explicit repository context, the helper accepts it, applies the same access-policy gate, destination tripwire, `auth.db` presence check, repository-binding lookup, and `CREDENTIAL_LOOKUP_DEADLINE` to that repository, and fails closed with the fixed diagnostic `Telemetry credential is unavailable.` and empty stdout for a missing, non-Git, unenrolled, opted-out, or otherwise unauthorized repository. The helper never initializes managed telemetry.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers`; the existing no-argument tests in `cli/src/services/auth_command/mod.rs` and `cli/src/services/telemetry_credential_storage.rs` pass unchanged; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_span_kind`.
- [ ] AC3: Native Codex telemetry is enabled only through the mechanism the T01 decision selects, and only for invocation modes that T01 proved repository-safe. When the decision selects a `sce codex` launcher, it resolves the effective repository and access policy before launch, supplies native OTEL settings only as process-local `-c` overrides that pin `MANAGED_OTLP_ENDPOINT` / `MANAGED_OTLP_PROTOCOL` and the helper with explicit repository context, forwards supported Codex arguments unchanged, preserves the terminal, signal delivery, and the Codex exit code, and refuses or launches without SCE telemetry for unproven modes. No credential is written to any file, and no global Codex config is changed unless T01 proves it necessary.
  - Validate: Targeted Rust tests for the launch argument builder and mode classification (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_telemetry`); `rg -n 'Bearer|OTEL_EXPORTER_OTLP_HEADERS' cli/src/services` shows no credential in launch arguments or written config; inspection of the T02 completion record for the exit-code and signal checks.
- [x] AC4: SCE credentials cannot be sent to an exporter that targets an unrelated endpoint through any path T01 identified. User-owned or enterprise-managed Codex OTEL configuration that conflicts with the managed endpoint causes SCE to skip native telemetry and report the reason; it is never overwritten.
  - Validate: Targeted Rust tests per conflict class identified in T01; inspection of the residual-risk statement in `context/sce/telemetry-credential.md`.
- [ ] AC5: With a valid stored credential, each `sce hooks codex` and `sce hooks codex-mutation-scope` process exports exactly one `sce.hook` span with `service.name=sce`, `sce.agent=codex`, `sce.hook.command`, `sce.hook.event`, `sce.hook.tool` when present, `sce.outcome`, and only the Codex session, turn, tool-use, and model identifiers that T01 verified. `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` child spans appear only when that operation executed in the process. Claude hook spans keep their current names and `sce.claude.*` attributes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`, including loopback-receiver tests over Codex `PostToolUse(apply_patch)` and non-tracked fixtures that decode the OTLP payload and assert span name, count, parent links, and attributes; existing Claude loopback tests pass unchanged.
- [x] AC6: A Codex hook `sce.hook` span is parented on upstream `TRACEPARENT` only when T01 proved that Codex supplies valid current-span context for that hook execution. Otherwise the span is an independent root with correlation attributes. No trace ID or parent span ID is ever derived from a session, turn, or tool-use identifier.
  - Validate: Loopback-receiver tests for valid, invalid, all-zero, and missing traceparent when parenting ships; otherwise `rg -n 'extract_parent_context' cli/src/services/managed_telemetry` shows extraction gated to the proven agent only, and `context/sce/codex-native-telemetry-contract.md` states the verdict.
- [ ] AC7: Telemetry cannot change Codex hook behavior. With credential lookup failure, an expired credential, a refused or stalled receiver, or exporter timeout, hook stdout, stderr, exit code, permission decisions, Agent Trace DB rows, and mutation-trace state match the telemetry-disabled run, and process lifetime stays within `CREDENTIAL_LOOKUP_DEADLINE` + `SHUTDOWN_TIMEOUT`.
  - Validate: Parity tests running the same Codex hook fixtures with Noop and with failing managed telemetry, comparing outputs and persisted state and asserting elapsed-time bounds.
- [ ] AC8: Live verification is recorded. A real Codex session in an enrolled repository delivers native traces, metrics, and logs, plus SCE Codex hook spans, to the correct Dash0 dataset with correct session and repository attribution. The record covers token, cache, model, cost, and tool telemetry; hook-span correlation with native Codex telemetry; subagent attribution where upstream provides identity; credential replacement, expiry, logout, and failed authorization; repository opt-out and global consent withdrawal; two repositories in different workspaces; preservation of user-owned and managed Codex config; an unchanged Claude session; and the absence of prompt, response, and tool content. A collector 2xx alone is not recorded as delivery.
  - Validate: Inspect the live-verification section of `context/sce/codex-native-telemetry-contract.md` and the T03 completion record.
- [ ] AC9: Current-state context describes the shipped Codex telemetry mechanism, the helper repository-context contract, hook admission and span attributes, the trace-context verdict, privacy controls, the final Claude/Codex parity matrix, remaining upstream limitations, the minimum verified Codex version, and required control-plane follow-ups. The hidden helper and any hidden flags stay absent from the public command reference.
  - Validate: Inspect the files listed under Context sync against the implemented code; `rg -n 'telemetry-headers' context/cli/cli-command-surface.md` shows no public listing.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth`

### Context sync

- `context/sce/codex-native-telemetry-contract.md` (new: T01 evidence, decision, parity matrix, live verification, smoke procedure)
- `context/sce/telemetry-credential.md` (helper repository context, Codex launch context, endpoint residual risk)
- `context/sce/cli-observability-contract.md` (Codex hook admission and span attributes)
- `context/sce/codex-integration-runtime.md` and `context/cli/codex-mutation-scope-integration.md` (span boundaries over the Codex hooks)
- `context/cli/access-policy.md` (helper and launcher enforcement points)
- `context/cli/cli-command-surface.md` (only if a public `sce codex` command ships)
- `context/sce/doctor-human-text-contract.md` (only if a doctor row changes)
- `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/context-map.md`
- `context/decisions/` (one ADR only if T02 introduces a launcher or another substantive architectural choice)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/auth_command/` (helper repository context); `cli/src/cli_schema.rs` and `cli/src/services/parse/command_runtime.rs` (hidden helper flag, optional `sce codex`); a launcher module under `cli/src/services/` only if T01 requires it; `cli/src/app.rs` (`managed_span_kind` admission, helper bypass); `cli/src/services/managed_telemetry.rs` and `managed_telemetry/trace_export.rs` (hook-agent span kind, Codex attributes, gated parent extraction); span boundaries at existing call sites in `cli/src/services/hooks/codex/` and `cli/src/services/hooks/codex_mutation_scope/`; Codex setup and doctor code only where the T01 decision requires it; focused tests; the context files listed above.
- **Out of scope:** Control Plane, Collector, Dash0 dataset, or warehouse changes; credential issuance, storage schema, or lifecycle changes; OpenCode and Pi telemetry; Pkl-generated `.codex/hooks.json` contract changes; changes to Claude settings, Claude span names, or Claude attributes; a public telemetry command.
- **Constraints:** static dispatch and concrete types (`AGENTS.md`); typed errors over `anyhow` where a caller branches on the cause; reuse `MANAGED_OTLP_ENDPOINT`, `MANAGED_OTLP_PROTOCOL`, `authorized_repository_credential_with`, `load_repository_telemetry_credential`, and the existing exporter; no comments that narrate changes; Nix-wrapped Cargo only; automated tests never contact the production collector or use real credentials; probes use dummy credentials and never log real tokens; telemetry stays fail-open for hooks and fail-closed for authentication; repository authorization and consent enforcement are never weakened.
- **Non-goal:** a telemetry proxy, local collector, sidecar, or daemon; a new database, credential lifecycle, or readiness marker; a generic agent-telemetry plugin framework; a second credential-authorization implementation; JSONL-based reconstruction of native telemetry unless T01 proves essential metrics are unavailable through OTEL.

## Assumptions

- The plan name is `codex-native-otel-instrumentation`, mirroring `claude-native-otel-instrumentation`.
- The locally installed `codex-cli 0.172.0` is the released binary T01 probes; upstream source at `rust-v0.172.0` is the reference for behavior the probe cannot observe directly.
- Codex hook spans use a `sce.codex.*` attribute namespace that mirrors `sce.claude.*`. Claude attribute names stay unchanged.
- `sce hooks codex` and `sce hooks codex-mutation-scope` are Codex-only subcommands, so admission needs no caller environment marker, as with `CLAUDE_ONLY_HOOK_SUBCOMMANDS`. Shared `diff-trace` and `conversation-trace` admission stays Claude-only.
- `ManagedSpanKind::ClaudeHook` becomes a hook variant carrying a statically dispatched agent enum (`Claude | Codex`) only if that keeps the Claude export byte-identical; otherwise a sibling `CodexHook` variant is used. This is a reversible local choice for T02.
- The T03 executor has a Dash0 account and an enrolled repository, as the `claude-hook-trace-propagation` T05 executor did.
- T03 is an evidence-and-context task, not a validation-only task. It produces the live-verification record and the current-state parity documentation that the change itself requires.

## Task stack

- [x] T01: `Establish the Codex 0.172.0 native telemetry contract by probe` (status:done)
  - Task ID: T01
  - Scope: In — probes with the released Codex 0.172.0 binary, a scratchpad loopback OTLP/HTTP receiver, dummy credentials, and a scratchpad stub header helper; reading `rust-v0.172.0` source for `otel.*` config, header-helper execution, hook spawning, and requirements layering; recording every item listed in AC1; writing `context/sce/codex-native-telemetry-contract.md` with evidence tables, the implementation decision (helper-argument mechanism, launcher or no launcher, safe invocation modes, setup/doctor needs, trace-parenting verdict, identifier mapping, privacy config), and the parity matrix. Out — any change under `cli/`; real credentials; contact with the production collector; probe code left in the repository.
  - Dependencies: none
  - Done when: the contract file answers every AC1 question with recorded evidence or an explicit "not observable" verdict and its reason; similarly named Claude and Codex metrics are compared by observed semantics, not by name; the decision names the smallest safe T02 mechanism; `git status` shows only the new context file and this plan's status update.
  - Verify: inspection of the contract file against the AC1 list; `git ls-files | rg -i 'probe|otlp-receiver'` returns nothing new; `rg -n 'Bearer [A-Za-z0-9._-]{12,}' context/sce/codex-native-telemetry-contract.md` returns no match.
  - Completed: 2026-11-24
  - Files changed: `context/sce/codex-native-telemetry-contract.md` (new); `context/plans/codex-native-otel-instrumentation.md` (T01 record)
  - Result: Probed Codex 0.172.0 (`rust-v0.172.0`, commit `3d86e43b`) with `exec`, `exec resume`, `exec fork`, pty-interactive start, subagent spawn, helper failure/refresh, and scratch-`CODEX_HOME` precedence runs against a loopback OTLP/HTTP JSON receiver with a dummy-header stub helper. Findings: the helper runs as `sh -c` from `CODEX_HOME` with the inherited Codex environment and no repository context, and argv arguments are delivered; refresh is timer-only (default 29 min), and helper failure suspends export without replay; a helper reaches every unpinned `otlp-http` exporter (user config and profile observed), so all exporter keys must be pinned by `-c`; `exec resume` without `-C` runs in the stored session cwd (attribution hazard); hooks never receive current-span `TRACEPARENT`, only inherited environment; hook `session_id` = `conversation.id`, `turn_id` = `turn.id`, `tool_use_id` = `call_id`, subagent `agent_id` = child `conversation.id`; `codex.turn.token_usage` `input` includes cached input; no cost signal; metrics carry no session id; logs carry unredactable tool arguments and `user.email`. Decision: hook spans ship as independent roots with no Codex `TRACEPARENT` extraction; native export only via a thin `sce codex` launcher that pins the helper with an explicit repository argument, traces and metrics to `MANAGED_OTLP_ENDPOINT` per-signal paths (`binary`), logs `none`, content flags false; safe modes are new interactive, `exec`, and `exec fork`, plus resume only with `-C`; conflict skip on user, profile, or readable system/requirements `otel.*`; no setup change; one ADR in T02.
  - Verify results: contract inspected against every AC1 item (helper argv/cwd/env/stdout/refresh/failure/caching, precedence across `-c`/profile/user/requirements, non-canonical reach, interactive/exec/resume/fork/cwd/threads attribution, TRACEPARENT verdict, identifier mapping, token/cost/cache/tool/duration/model/repository/subagent semantics, privacy config, decision, parity matrix) — passed; `git ls-files | rg -i 'probe|otlp-receiver'` lists only pre-existing tracked fixtures and no new path (`git status` shows only the new contract file and this plan) — passed; `rg -n 'Bearer [A-Za-z0-9._-]{12,}|hvs\.|s\.[A-Za-z0-9]{20,}'` on the contract — no match, passed.
  - Context impact: localized-new-contract — adds `context/sce/codex-native-telemetry-contract.md`; no shipped behavior changed; root context unaffected until T02 ships the mechanism.
  - Deviations: enterprise requirements, MDM, cloud, and `app-server` layering were not probed live (root required or out of scope) and are recorded as source-only or unproven; Claude metric semantics in the parity matrix come from existing context and documentation and are deferred to T03 for live confirmation; probe runs created Codex session rollouts under the user's `~/.codex/sessions` (no config change).
  - Context synchronization: synced

- [x] T02: `Ship native Codex telemetry and Codex hook spans on the managed infrastructure` (status:done)
  - Task ID: T02
  - Scope: In — the T01-selected mechanism only: explicit repository context on the hidden helper (no-argument behavior unchanged), a thin `sce codex` launcher if and only if T01 requires it, and conflict detection for user-owned and enterprise Codex OTEL config; hook admission for `sce hooks codex` and `sce hooks codex-mutation-scope` in `managed_span_kind`; the hook-agent span kind and verified `sce.codex.*` attributes; reuse of `sce.mutation_scope`, `sce.git_snapshot`, and `sce.persistence` at existing Codex call sites; `TRACEPARENT` extraction for Codex only if T01 proved current-span propagation; setup and doctor changes only where T01 requires them, keeping configuration availability, credential authorization, and delivery as distinct states; focused unit and loopback-receiver tests; one ADR if a launcher ships. Out — live Dash0 verification; Claude span or settings changes; credential lifecycle changes; Pkl hook contract changes; unproven invocation modes.
  - Dependencies: T01
  - Done when: AC2 through AC7 hold in tests; the helper bypass in `managed_span_kind` still returns no span kind for `sce auth telemetry-headers` in all argument forms; every existing Claude telemetry, helper, and Codex hook test passes unchanged; no credential appears in launch arguments, written config, output, or diagnostics.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_span_kind`; `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`.
  - Completed: 2026-11-24
  - Files changed: `cli/src/app.rs`; `cli/src/cli_schema.rs`; `cli/src/command_surface.rs`; `cli/src/services/mod.rs`; `cli/src/services/command_registry.rs`; `cli/src/services/parse/command_runtime.rs`; `cli/src/services/auth_command/mod.rs`; `cli/src/services/auth_command/command.rs`; `cli/src/services/managed_telemetry/trace_export.rs`; `cli/src/services/hooks/codex/mod.rs`; `cli/src/services/hooks/codex_mutation_scope/lifecycle.rs`; `cli/src/services/hooks/codex_mutation_scope/mod.rs`; `cli/src/services/hooks/codex_mutation_scope/telemetry_tests.rs` (new); `cli/src/services/codex_telemetry/mod.rs` (new); `cli/src/services/codex_telemetry/command.rs` (new); `cli/src/services/codex_telemetry/tests.rs` (new); `context/plans/codex-native-otel-instrumentation.md` (T02 record)
  - Result: Added a sibling `ManagedSpanKind::CodexHook`; `sce hooks codex` and `sce hooks codex-mutation-scope` are admitted without a caller marker and export one root `sce.hook` span with `sce.agent=codex`, `sce.hook.command`/`event`/`tool`, and `sce.codex.session_id`/`turn_id`/`tool_use_id`/`model`/`agent_id`/`agent_type` from stdin; `upstream_parent_context` adopts `TRACEPARENT` only for `ClaudeHook`. The Codex mutation-scope seam runs inside `sce.mutation_scope`, and the existing coordinator supplies `sce.git_snapshot`/`sce.persistence`. The hidden helper accepts a hidden `--repo <path>` (`AuthSubcommand::TelemetryHeadersForRepository`), routing the same policy gate, destination tripwire, `auth.db` probe, repository binding, and deadline to that path; the no-argument variant and code path are unchanged. A public `sce codex [codex arguments...]` launcher (`services::codex_telemetry`) classifies the invocation (new interactive, `exec`, `exec fork`, and resume only with `-C`; everything else launches plainly), resolves the repository and access policy, skips with a one-line stderr reason on destination-environment, caller `-c otel.*`, user, profile, readable `/etc/codex/*` OTEL settings, unreadable config, or an unauthorized repository, and otherwise prepends process-local `-c` overrides (shell-quoted helper with `--repo`, 300 s refresh, `otel.exporter="none"`, traces and metrics `otlp-http` binary to the managed per-signal endpoints, three content flags false), then `exec`s `codex` (spawn-and-exit on non-Unix). `managed_span_kind` returns no span kind for `sce codex` and for every `sce auth telemetry-headers` argument form.
  - Verify results: `test … managed_telemetry` — 23 passed; `test … telemetry_headers` — 11 passed; `test … codex` — 59 passed; `test … managed_span_kind` — 3 passed; `clippy … --all-targets -- -D warnings` — clean; full `test --manifest-path cli/Cargo.toml` — 915 passed, 0 failed (existing Claude telemetry, helper, and Codex hook tests unchanged and passing). Manual launcher check with the built binary and a stub `codex` on `PATH`: stub `exit 7` → `sce codex` status 7; stub self-`SIGTERM` → status 143; arguments (including `--help`) forwarded verbatim; resume without `-C`, user `[otel]` exporter, foreign `OTEL_EXPORTER_OTLP_ENDPOINT`, caller `-c otel.*`, and an unauthorized repository each printed the expected one-line reason and launched without overrides; a missing `codex` binary returned `Failed to launch \`codex\``; `sce auth telemetry-headers [--repo <dir>]` without a credential failed closed with `Telemetry credential is unavailable.` and empty stdout. No credential appears in overrides (asserted by `codex_telemetry_overrides_pin_every_exporter_and_disable_content`).
  - Context impact: cross-domain — adds the public `sce codex` command and the hidden helper `--repo` flag, the `CodexHook` span kind and `sce.codex.*` attributes, and Codex-only trace-parent gating; affects `context/cli/cli-command-surface.md`, `context/sce/telemetry-credential.md`, `context/sce/cli-observability-contract.md`, `context/sce/codex-integration-runtime.md`, `context/cli/codex-mutation-scope-integration.md`, `context/cli/access-policy.md`, and root architecture/overview/glossary; the launcher is the ADR-worthy architectural choice.
  - Deviations: the helper repository argument ships as a separate `TelemetryHeadersForRepository` variant so the existing no-argument parse test and code path stay unchanged, which drops `Copy` from `AuthSubcommand`/`AuthRequest`; `otel.span_attributes` repository attribution was not added (T01 local choice); `plan_launch_with` takes an injected repository-root resolver because this machine has a stray `/tmp/.git` directory that made a temp-dir "not a repository" test non-hermetic; the ADR is left to the context-synchronization decision gate; SCE hook runs during the session wrote an untracked `cli/context/tmp/` log (not part of this change).
  - Context synchronization: synced

- [x] T03: `Record live Codex telemetry verification and the Claude/Codex parity contract in durable context` (status:done)
  - Task ID: T03
  - Scope: In — real Codex sessions through the shipped mechanism against `https://otel.crocoderlab.dev` with existing managed credentials; Dash0 observation of every AC8 item; a Claude regression session; the live-verification section, final parity matrix, upstream limitations, minimum verified Codex version, and control-plane follow-up list in `context/sce/codex-native-telemetry-contract.md`; synchronization of every file under Context sync. Out — code changes (a defect found here is recorded with its evidence and fixed in a new task appended to this plan); Control Plane or warehouse changes; recording real tokens, prompts, or tool content.
  - Dependencies: T02
  - Done when: every AC8 item is recorded as observed in the correct Dash0 dataset with trace IDs, or recorded as not verifiable with the exact reason; AC9 context matches the shipped code; the minimum verified Codex version is stated.
  - Verify: inspection of the live-verification section and the Context sync files against the code; `rg -n 'telemetry-headers' context/cli/cli-command-surface.md` shows no public listing; `rg -n 'Bearer [A-Za-z0-9._-]{12,}' context/` returns no match.
  - Completed: 2026-11-24
  - Files changed: `context/sce/codex-native-telemetry-contract.md`, `context/cli/cli-command-surface.md`, `context/cli/codex-mutation-scope-integration.md`
  - Result: Recorded Dash0 delivery in `sce-crocoder` for Codex CLI 0.172.0. Confirmed native traces, token, cache, model and tool series, independent SCE hook roots, the mutation child-span hierarchy, tool and subagent identifier joins, and Claude tool-hook parenting and native metrics. Confirmed zero native Codex log records in the check window. Real sessions for repository opt-out, process-local global opt-out and failed authorization had no matching spans. The launcher preserved the user-owned config in a scratch conflict check. Added the final parity matrix, smoke procedure, limitations, minimum verified version, and Control Plane follow-ups.
  - Verify results: Checked the live evidence and documented contracts against the launcher, credential and hook code. Dash0 trace IDs and scenario-specific evidence are in the native telemetry contract. The public command reference and `sce auth --help` omit the hidden helper. The context bearer scan, expanded contract credential scan, and `git diff --check` passed. The targeted Nix-wrapped `telemetry_credential_storage` and `codex_telemetry` suites passed 14 and 11 tests respectively.
  - Context impact: domain-local documentation. Updates the Codex telemetry evidence and command and mutation-scope references. The shipped launcher, managed exporter, and hook behavior are unaffected.
  - Deviations: Native log delivery is unavailable because the launcher disables it for privacy. Codex 0.172.0 supplies no native cost metric or per-session metric identifier. Disposable credentials for live rotation, expiry and logout, a second enrolled workspace, and enterprise-managed Codex config were unavailable. The contract records those limits and the corresponding local test evidence.
  - Context synchronization: synced

## Open questions

- Is launcher-only coverage worth its cost? If T01 proves that `CODEX_HOME` execution makes a user-level `config.toml` install unable to identify the repository, only `sce codex` launches will export native telemetry, and every plain `codex` launch stays dark. That is weaker than Claude parity, where a user-level install covers every launch. A smaller alternative is to ship only the T02 Codex hook spans, which need no launcher, and defer native export until upstream passes a working directory or repository context to the helper.
- Will Codex native metrics be usable without control-plane work? Codex metric names and units differ from `claude_code.*`, and this plan excludes warehouse and dashboard changes. Delivered Codex metrics may not appear in existing SCE views until the follow-ups T03 records are done.

## Validation Report

**Status:** failed

**Date:** 2026-11-24

### Commands run

- `nix flake check` -> exit 0 (all checks passed on `x86_64-linux`).
- `nix run .#pkl-check-generated` -> exit 0 (157 generated files verified).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry` -> exit 0 (23 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry` -> exit 0 (112 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex` -> exit 0 (60 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth` -> exit 0 (25 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers` -> exit 0 (11 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_span_kind` -> exit 0 (3 passed).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_telemetry` -> exit 0 (11 passed, including the visible conflict classes).
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml matches_telemetry_disabled_run` -> exit 0 (2 passed for refused and stalled receivers).
- `git ls-files` and `git diff --name-only a0562c0b --` with `probe|otlp.?receiver|telemetry.*fixture|(^|/)(tmp|scratch)(/|$)` path filters -> exit 0 (tracked fixtures predate the plan; the plan delta adds one temporary runtime log).
- `git ls-files --stage -- cli/context/tmp/sce-24_11_2026.log` -> exit 0 (the log is tracked and was introduced by `977ac744`).
- `nix run nixpkgs#ripgrep -- -l 'Bearer [A-Za-z0-9._-]{12,}|hvs\.|s\.[A-Za-z0-9]{20,}' context/sce/codex-native-telemetry-contract.md` -> exit 1 (expected, no matches).
- `nix run nixpkgs#ripgrep -- -n 'Bearer|OTEL_EXPORTER_OTLP_HEADERS' cli/src/services` -> exit 0 (matches include existing credential handling and tests; inspection confirmed no credential in launcher overrides or written Codex config).
- `nix run nixpkgs#ripgrep -- -n -C 4 'extract_parent_context' cli/src/services/managed_telemetry` -> exit 0 (extraction is Claude-only).
- `nix run nixpkgs#ripgrep -- -n 'telemetry-headers' context/cli/cli-command-surface.md` -> exit 1 (expected, no public listing).
- `codex --version`, `codex exec --help`, `codex exec fork --help`, and `codex exec resume --help` -> exit 0 (0.172.0; fork and resume accept `--worktree` and `--remote`).
- `git diff --check` and `git diff --cached --check` -> exit 0 (no whitespace errors).

### Success-criteria verification

- [x] AC1: Record the Codex 0.172.0 probe, implementation decision, and parity matrix -> The contract covers all requested topics with observations, source evidence, or explicit limits. The repository adds no probe or receiver fixture for this plan. The contract credential scan found no matches.
- [ ] AC2: Preserve the Claude helper and fail closed for invalid explicit repository context -> The unchanged Claude assertions, storage tests, and managed-telemetry bypass passed. The explicit-repository test also expects a header for an absent subdirectory under the enrolled root, contrary to the missing-path guarantee.
- [ ] AC3: Enable native telemetry only on repository-safe invocations -> Override pinning and forwarding tests passed. The T02 record confirms Unix exit status 7 and SIGTERM status 143. Classification stops at `exec fork` and ignores unknown options after `resume`, so accepted switches such as `--worktree` can pass the telemetry classifier. Only `exec resume -C` has recorded directory-probe evidence.
- [x] AC4: Keep credentials pinned to the managed exporter and preserve conflicting config -> Conflict tests passed for caller, user, profile, readable system, and environment settings. Override tests pin all three signal keys. T01 documents upstream helper isolation for requirements-layer exporters and the limits of inspecting MDM and cloud requirements.
- [ ] AC5: Export the specified Codex hook spans and identifiers while preserving Claude spans -> Loopback tests passed for `sce hooks codex`, tracked and untracked mutation-scope fixtures, and existing Claude spans. Live evidence confirms the operation hierarchy. The fixtures also assert `sce.codex.agent_id` and `sce.codex.agent_type`, beyond AC5’s stated identifier list.
- [x] AC6: Keep Codex hook spans as independent roots -> The parent extraction branch is Claude-only. Tests cover valid, invalid, all-zero, and absent inbound context, and loopback payloads show independent Codex roots.
- [ ] AC7: Preserve complete hook behavior and process lifetime across telemetry failures -> The refused and stalled receiver tests passed. They compare in-process return values, seam inputs, state-file names, and separate timing checks. They bypass credential admission and the production persistence seam and leave process stderr, exit status, permission decisions, database rows, persisted contents, and the combined time budget unverified.
- [ ] AC8: Record the required live Dash0 delivery and scenario coverage -> T03 records native traces and metrics, SCE hook spans, correlation, and Claude regression evidence in `sce-crocoder`. Native logs are disabled for privacy, Codex 0.172.0 exposes no cost signal or per-session metric ID, and live replacement, expiry, logout, second-workspace routing, and managed-config verification remain open.
- [ ] AC9: Align current-state context with the shipped implementation and keep the helper hidden -> The Context sync documents, launcher ADR, parity matrix, version, privacy settings, and follow-ups are present. The public command scan found no helper listing. The documented missing-path check and the statement that unrecognized launcher options always disable telemetry disagree with the implementation.

### Failed checks and follow-ups

- AC2: A missing explicit path can inherit an enrolled ancestor's credential. The test `telemetry_headers_bind_the_explicit_repository_and_fail_closed_elsewhere` expects success for a descendant it never creates. Enforce the intended explicit `--repo` path contract or revise that contract with an explicit decision.
- AC3: The launch classifier returns at `exec fork` before checking remaining options, and `classify_resume` ignores unrecognized options and continues past `--`. `--worktree` and `--remote` are available on fork and resume in Codex 0.172.0. Check mode-changing options and the argument terminator across the complete invocation. Establish repository attribution for interactive `resume -C` or keep it plain.
- AC5: The implementation and T01 decision include two subagent attributes outside the AC5 wording. Decide whether to include the verified subagent fields in the criterion or narrow the exporter.
- AC7: `cli/src/services/hooks/codex_mutation_scope/telemetry_tests.rs` checks selected outcomes at an injected seam. Add subprocess-level parity evidence for both Codex hook commands across lookup failure, expiry, receiver refusal, receiver stall, and exporter timeout, with production-equivalent persistence and the combined time bound.
- AC8: `context/sce/codex-native-telemetry-contract.md` and T03 record incomplete live outcomes. Decide how the criterion should represent privacy-disabled logs, unavailable cost and metric identifiers, and the one-workspace evidence. Obtain disposable credentials, a second enrolled workspace, and an enterprise fixture for the applicable outstanding checks.
- AC9: `context/sce/telemetry-credential.md` promises rejection of missing paths. `context/sce/codex-native-telemetry-launcher.md` promises unrecognized options disable native telemetry. Align these contracts with the resolution of AC2 and AC3.
- Temporary artifact: `cli/context/tmp/sce-24_11_2026.log` is the runtime log noted in T02 and was tracked by `977ac744`. Remove it from the delivered tree in a repair session.

### Residual risks

- A running Codex process can retain its launch-repository credential across an in-process thread or directory change and keeps cached headers until refresh.
- Unreadable MDM and cloud requirements can override local pins or logging settings. T01 documents the upstream requirements-layer helper isolation from source.
- Flake validation covered `x86_64-linux`. Nix omitted three incompatible systems.

### Retry

After repairs, rerun:

`/validate context/plans/codex-native-otel-instrumentation.md`
