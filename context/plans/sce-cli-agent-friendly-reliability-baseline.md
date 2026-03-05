# Plan: sce-cli-agent-friendly-reliability-baseline

## 1) Change summary

Harden the `sce` CLI for agent-driven usage by making command output more machine-readable, error paths more actionable, and help/usage text example-rich while preserving current command intent. This plan targets a reliability baseline (not a full command-surface redesign) so agents can parse, retry, and compose command results with fewer ambiguities. It also addresses human/operator ergonomics by allowing setup config installation and required hook installation in a single `sce setup` invocation.

Assumptions locked from clarification:
- Scope preset: `Reliability baseline`.
- Keep existing top-level commands (`help`, `setup`, `doctor`, `mcp`, `hooks`, `sync`) and avoid breaking behavior contracts unless required for determinism/actionability.
- Prefer additive AI-friendly UX improvements (structured output, better errors, stronger help examples) over broad architectural changes.
- Remove the current `setup` mode split that forces separate runs for config target setup vs `--hooks` installation.

## 2) Success criteria

- Key read/report commands expose deterministic structured output via `--format json` with stable field names and explicit status/result fields.
- Human-readable output remains deterministic and consistent across runs for the same inputs.
- Common failure modes return actionable messages with explicit remediation (`Try:` guidance and/or valid alternative command/flags).
- Interactive friction is reduced for automation paths by ensuring all required inputs are flag-addressable and clearly documented in help text.
- `sce setup` supports combined config + hook installation in one run for both interactive and non-interactive target selection flows.
- `setup` provides explicit non-interactive control switches so CI/automation can fail fast instead of prompting.
- Command outputs follow a stable stdout/stderr contract so stdout remains pipe-safe and stderr carries diagnostics/errors.
- Each command has command-local `--help` usage/examples and the CLI exposes a machine-readable `version` command.
- Error messages include stable error codes alongside actionable remediation guidance.
- CLI configuration precedence is explicit and deterministic (`flags > env > config file > defaults`) with inspect/validate surfaces.
- Exit code behavior is stable and documented by failure class for automation-safe handling.
- Runtime observability supports structured logs and deterministic log levels without polluting command payload output.
- Optional file logging is supported with safe defaults (bounded file behavior/rotation policy or explicit truncation policy) and redaction-safe content.
- Resilience controls (timeouts/retry/backoff where relevant) are explicit, bounded, and surfaced via actionable failure messages.
- Security hardening covers secret redaction, strict path validation, and safe file-permission handling on write/install flows.
- Shell completion artifacts are generated/documented and aligned with current command/flag docs.
- `--help` output includes concrete examples that an agent can copy/modify directly, including at least one JSON-output example.
- New parser/output/error behavior is covered by focused unit tests that lock output contracts.

## 3) Constraints and non-goals

Constraints:
- Do not introduce a full command taxonomy rewrite; keep current command ownership and routing model in `cli/src/app.rs` and `cli/src/services/*`.
- Maintain deterministic output ordering and avoid nondeterministic text fragments.
- Preserve placeholder safety boundaries for `mcp` and `sync` (no accidental production behavior expansion).
- Preserve backward-compatible `setup` behavior for existing single-purpose invocations (`sce setup --hooks` and `sce setup --opencode|--claude|--both`).
- Keep task slicing one-task/one-atomic-commit.

Non-goals:
- Implementing real MCP execution or cloud sync execution.
- Replacing the full setup interaction model with a new wizard or TUI.
- Large refactors unrelated to output/error/help reliability.

## 4) Task stack (`T01..T21`)

- [x] T01: Define deterministic config model and precedence (status:done)
  - Task ID: T01
  - Goal: Add a config contract that resolves values in deterministic order (`flags > env > config file > defaults`) and expose inspect/validate entrypoints.
  - Boundaries (in/out of scope): In: config model/types, parser integration, env mapping, config-file load/validation, and command help/docs; Out: remote config services.
  - Done when: precedence is codified and tested, `sce config show` and `sce config validate` (or equivalent) return deterministic text/JSON output, and existing commands can consume resolved config without behavior drift.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T02: Establish stable exit-code contract (status:done)
  - Task ID: T02
  - Goal: Define and enforce fixed exit-code classes for parse/validation/runtime/dependency failures.
  - Boundaries (in/out of scope): In: top-level run/dispatch failure mapping and docs/tests for code meanings; Out: shell-specific wrapper behavior.
  - Done when: representative failure paths return documented stable exit codes and tests assert mappings.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T03: Add structured observability contract (status:done)
  - Task ID: T03
  - Goal: Introduce deterministic logging modes/levels (for example plain and JSON logs) that are separate from command result payloads.
  - Boundaries (in/out of scope): In: logging facade/options and service integration points; Out: external telemetry backends.
  - Done when: operators can set log level/format predictably, logs include stable event identifiers, and stdout payload contracts remain unchanged.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T21: Add OpenTelemetry setup baseline (status:done)
  - Task ID: T21
  - Goal: Add an OpenTelemetry-based observability setup path for the CLI so structured events can be exported through standard OTEL tooling while preserving command payload contracts.
  - Boundaries (in/out of scope): In: OTEL bootstrap wiring, deterministic env/flag configuration for exporter mode, and tests/docs for setup behavior; Out: hosted telemetry backend provisioning and production collector deployment.
  - Done when: `sce` can initialize OTEL instrumentation deterministically, export path configuration is explicit/actionable, and stdout/stderr payload boundaries remain contract-safe.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.
  - User-provided implementation context (locked):
    - Instrumentation stack should use Rust `tracing` + `tracing-subscriber` with `tracing-opentelemetry` bridging into OpenTelemetry OTLP export (`opentelemetry`, `opentelemetry-sdk`, `opentelemetry-otlp`).
    - Prefer app-embedded instrumentation and exporter wiring in the CLI runtime (not a separate "OTel CLI" binary).
    - Baseline setup should follow standard flow: initialize tracer provider, attach OpenTelemetry layer to subscriber registry, run command with spans/events, and flush/shutdown provider before process exit.
     - Endpoint/config should be env-addressable (for example `OTEL_EXPORTER_OTLP_ENDPOINT`), with deterministic defaults and actionable validation errors for invalid configuration.
     - Keep command payload contract safe: observability output/export path must not pollute stdout command result payloads.

- [x] T22: Add global config discovery aligned with Agent Trace state root (status:done)
  - Task ID: T22
  - Goal: Add deterministic user-global config discovery for `sce config` and merge global+local config in memory, with local values overriding global values per key.
  - Boundaries (in/out of scope): In: platform-aware global path derivation, in-memory global+local merge behavior, explicit precedence integration with `--config` and `SCE_CONFIG_FILE`, clear source/merge reporting in `config show/validate`, and focused tests/docs/context updates; Out: config schema expansion beyond existing keys and migration tooling.
  - Done when: when no explicit config path/env override is provided, `sce config` discovers both global and local files (when present), merges them in memory with local stronger than global per key, then applies env and flags on top; output contracts/tests document merged-source behavior deterministically.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml services::config::tests`; `cargo test --manifest-path cli/Cargo.toml app::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T04: Add file logging mode with safe defaults (status:done)
  - Task ID: T04
  - Goal: Support optional log sink to file path with deterministic behavior and safe permission handling.
  - Boundaries (in/out of scope): In: CLI flags/config for file logging, file-open/write/rotation-or-truncation policy, and tests; Out: remote log shipping.
  - Done when: file logging can be enabled explicitly, output location/policy is documented, writes are deterministic, and failure cases are actionable.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T05: Add resilience policy for retries/timeouts/backoff (status:done)
  - Task ID: T05
  - Goal: Define bounded retry/timeout behavior for eligible operations and surface retry outcomes clearly.
  - Boundaries (in/out of scope): In: operation-level resilience wrappers for IO/process/database hotspots and user-facing retry diagnostics; Out: unbounded retry loops or hidden automatic mutation.
  - Done when: targeted operations use deterministic timeout/retry settings, retries are observable in logs, and terminal failures provide actionable next steps.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T06: Apply security hardening for CLI interfaces (status:done)
  - Task ID: T06
  - Goal: Harden user input/output and filesystem interaction surfaces with secret redaction, path safety checks, and permission validation.
  - Boundaries (in/out of scope): In: error/log redaction rules, path canonicalization/allow checks (including `--repo`), and install/write permission checks; Out: network auth redesign.
  - Done when: sensitive values are redacted from diagnostics/logs, unsafe paths are rejected deterministically, and security-focused tests cover core threat paths.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T07: Add explicit non-interactive setup controls (status:done)
  - Task ID: T07
  - Goal: Add `setup` flags that let operators opt out of prompts deterministically (for example fail-fast non-interactive mode) while preserving existing target-flag behavior.
  - Boundaries (in/out of scope): In: setup parser/dispatch and setup usage text in `cli/src/app.rs` + `cli/src/services/setup.rs`; Out: replacing the interactive prompt engine or adding unrelated setup features.
  - Done when: automation can run `setup` without ever entering prompts, and prompt-required paths return actionable non-interactive guidance.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests services::setup::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T08: Enforce stdout/stderr output contract (status:done)
  - Task ID: T08
  - Goal: Establish and apply a deterministic stream contract where primary result payloads go to stdout and diagnostics/errors go to stderr.
  - Boundaries (in/out of scope): In: top-level run/error handling and affected command output paths; Out: changing core command semantics.
  - Done when: success payloads are pipe-safe from stdout, and non-success diagnostics are emitted consistently via stderr.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T09: Add command-local help surfaces with examples (status:done)
  - Task ID: T09
  - Goal: Ensure each command supports `--help` with concise usage and examples (including one JSON example where format applies).
  - Boundaries (in/out of scope): In: parser/help surfaces for `setup`, `doctor`, `mcp`, `hooks`, and `sync`; Out: broad doc-site generation.
  - Done when: `sce <command> --help` works consistently and examples are deterministic/copy-ready.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [x] T10: Add machine-readable version command (status:done)
  - Task ID: T10
  - Goal: Introduce `sce version` with stable text and JSON output fields for runtime identification.
  - Boundaries (in/out of scope): In: command surface, parser/dispatch, and version payload wiring (for example version/build metadata fields); Out: release pipeline redesign.
  - Done when: `sce version` and `sce version --format json` return deterministic version metadata with tests locking field names.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests command_surface::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T11: Introduce stable error-code taxonomy (status:todo)
  - Task ID: T11
  - Goal: Add stable error identifiers to actionable user-facing errors so automation can branch on codes and operators can search remediation docs quickly.
  - Boundaries (in/out of scope): In: top-level parse/invocation error strings and selected service-validation failures; Out: internationalization or deep logging system changes.
  - Done when: core user-facing errors include stable codes plus `Try:` guidance and tests lock representative code/message pairs.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests services::setup::tests services::hooks::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T12: Add shell completion generation and docs alignment (status:todo)
  - Task ID: T12
  - Goal: Provide shell completion artifacts (Bash/Zsh/Fish) and align command docs/help so completion, usage, and README examples match current flags/subcommands.
  - Boundaries (in/out of scope): In: CLI completion command/surface and documentation alignment in `cli/README.md`; Out: external package-manager integration.
  - Done when: completion outputs are generated deterministically, install/use instructions are documented, and docs/examples align with actual parser behavior.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T13: Add shared output-format contract and parser wiring (status:todo)
  - Task ID: T13
  - Goal: Introduce a single CLI-level output format contract (text/json) and route supported commands through it.
  - Boundaries (in/out of scope): In: parsing/wiring in `cli/src/app.rs`, command-surface/help exposure, and any small shared output-type helpers; Out: changing command business logic beyond format selection.
  - Done when: supported commands accept `--format <text|json>` deterministically, invalid format values fail with actionable guidance, and default format remains stable.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T14: Enable single-run `setup` flow for config targets plus hooks (status:todo)
  - Task ID: T14
  - Goal: Refactor setup option parsing/dispatch so operators can install target config assets and required hooks in one invocation, including interactive default path and non-interactive target flags.
  - Boundaries (in/out of scope): In: setup option model and dispatch in `cli/src/app.rs` and `cli/src/services/setup.rs`, setup usage text, and setup tests; Out: replacing `inquire` prompt technology or changing hook install semantics.
  - Done when: commands like `sce setup --opencode --hooks` and interactive `sce setup` can complete both config install and hook install deterministically in one run, while legacy one-purpose invocations continue to work.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests services::setup::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T15: Implement deterministic JSON/text dual output for `doctor` (status:todo)
  - Task ID: T15
  - Goal: Extend doctor reporting to return a stable machine-readable JSON form while preserving readable text output.
  - Boundaries (in/out of scope): In: `cli/src/services/doctor.rs` report rendering, JSON schema shaping, and tests; Out: changing doctor readiness semantics or required-hook policy.
  - Done when: `doctor --format json` emits stable object structure (readiness, hook path source, repository/hook paths, hook states, diagnostics), and text output remains deterministic.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml services::doctor::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T16: Standardize placeholder command output contracts for agent parsing (status:todo)
  - Task ID: T16
  - Goal: Make `mcp` and `sync` placeholder responses emit structured status payloads in JSON format and deterministic text summaries.
  - Boundaries (in/out of scope): In: `cli/src/services/mcp.rs`, `cli/src/services/sync.rs`, and related tests; Out: enabling non-placeholder runtime behavior.
  - Done when: both commands support `--format json` with stable fields indicating placeholder state, capabilities/checkpoints, and actionable next-step messaging.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml services::mcp::tests services::sync::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T17: Make parser and invocation errors consistently actionable (status:todo)
  - Task ID: T17
  - Goal: Normalize high-frequency parse/invocation errors to include explicit remediation examples (required flag guidance, valid alternatives, and targeted help pointers).
  - Boundaries (in/out of scope): In: top-level parser and command-specific validation errors in `cli/src/app.rs` and relevant service parsers; Out: changing exit-code policy or introducing interactive recovery prompts.
  - Done when: unknown command/option, missing required args, and incompatible-flag failures all provide deterministic actionable guidance suitable for automated retry.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml app::tests services::setup::tests services::hooks::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T18: Strengthen help/usage content with agent-oriented examples (status:todo)
  - Task ID: T18
  - Goal: Upgrade help text and setup usage docs with concise examples showing non-interactive usage, JSON output, and composable command flows.
  - Boundaries (in/out of scope): In: `cli/src/command_surface.rs`, setup usage text in `cli/src/services/setup.rs`, and `cli/README.md`; Out: large documentation restructuring outside CLI scope.
  - Done when: `sce --help` and `sce setup --help` include clear usage blocks + concrete examples, including one-run setup+hooks examples, and README mirrors canonical examples without contradiction.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml command_surface::tests services::setup::tests`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T19: Add output-contract regression tests for determinism (status:todo)
  - Task ID: T19
  - Goal: Lock the new output and error contracts with targeted tests to prevent accidental format drift.
  - Boundaries (in/out of scope): In: parser/service tests for JSON shape, required keys, deterministic field ordering expectations where applicable, and error text assertions; Out: introducing snapshot frameworks or broad new test infrastructure.
  - Done when: deterministic contract tests exist for representative success/failure paths across updated commands and pass reliably.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`; `cargo check --manifest-path cli/Cargo.toml`.

- [ ] T20: Validation and cleanup (status:todo)
  - Task ID: T20
  - Goal: Run full verification, ensure no temporary scaffolding remains, and sync context artifacts to final current-state behavior.
  - Boundaries (in/out of scope): In: final CLI verification pass, plan status updates, and context sync checks/updates for changed command contracts; Out: new feature work.
  - Done when: all verification checks pass, plan task statuses are current, and context documentation reflects final command/output/error contracts.
  - Verification notes (commands or checks): `cargo fmt --manifest-path cli/Cargo.toml --all -- --check`; `cargo test --manifest-path cli/Cargo.toml`; `cargo build --manifest-path cli/Cargo.toml`; `nix run .#pkl-check-generated`; `nix flake check`.

## 5) Open questions (if any)

- None at plan time; scope is constrained to the reliability baseline selected during clarification.
