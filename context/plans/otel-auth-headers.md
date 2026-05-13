# Plan: otel-auth-headers

## Change summary

Add standard OpenTelemetry OTLP header authentication support for the `sce` CLI so hosted collectors such as Dash0 can receive authenticated spans without storing secrets in repo-local `.sce/config.json`.

The implementation should support env-only `OTEL_EXPORTER_OTLP_HEADERS` resolution, parse standard comma-separated `key=value` header pairs, apply those headers to both OTLP gRPC and HTTP/protobuf exporters when OTEL is enabled, and keep all credential values redacted from logs, diagnostics, config inspection output, and context artifacts.

## Success criteria

- `OTEL_EXPORTER_OTLP_HEADERS` is accepted as the first supported OTLP auth mechanism.
- Header values are read from environment only; no config-file key is added for secret header values.
- Supported input format is standard OTLP header syntax: comma-separated `key=value` pairs, including `Authorization=Bearer <token>`.
- Invalid header syntax fails startup/config resolution with deterministic, actionable validation guidance when OTEL is enabled.
- Parsed headers are applied to both `grpc` and `http/protobuf` OTLP exporters.
- `sce config show` can indicate header configuration presence/provenance without printing secret values.
- Observability logs, stderr diagnostics, file sinks, and tests do not expose raw header values or tokens.
- Existing stdout/stderr contracts and existing OTEL endpoint/protocol behavior remain unchanged.
- `nix run .#pkl-check-generated` and `nix flake check` pass before plan completion.

## Constraints and non-goals

- Do not store OTLP auth secrets in `.sce/config.json`, generated schema files, context files, or tests.
- Do not add Dash0-specific config keys in the initial implementation.
- Do not introduce a new observability backend or tracing framework.
- Do not require a live Dash0 account or real external collector in automated tests.
- Do not change the existing OTEL enablement gate: exporter setup remains opt-in through `SCE_OTEL_ENABLED` / `otel.enabled`.
- Preserve existing precedence for current observability keys; `OTEL_EXPORTER_OTLP_HEADERS` is env-only and should not imply config-file fallback.
- Preserve command payload stdout separation and stderr-only diagnostics/logging behavior.

## Assumptions

- Auth support targets the standard OTLP env var `OTEL_EXPORTER_OTLP_HEADERS`.
- The first supported header format is standard comma-separated OTLP header pairs such as `Authorization=Bearer <token>`.
- It is acceptable for `sce config show` to report only whether headers are set, their source, and/or a redacted placeholder, not the raw header value.
- Any real Dash0 token remains operator-provided outside the repository, for example in the shell environment or a local secret manager.

## Task stack

- [x] T01: Freeze env-only OTLP header auth contract (status:done)
  - Task ID: T01
  - Goal: Update the current observability/config contracts to define `OTEL_EXPORTER_OTLP_HEADERS` as an env-only secret-bearing OTLP auth input.
  - Boundaries (in/out of scope): In - `context/sce/cli-observability-contract.md`, `context/cli/config-precedence-contract.md`, `context/glossary.md` if terminology is needed, and `context/context-map.md` if navigation text needs a focused update. Define syntax, precedence, redaction, config-show behavior, validation failure behavior, and non-goals. Out - Rust code changes, schema changes for secret values, and collector-specific Dash0 docs beyond generic hosted-collector examples.
  - Done when: Future implementation tasks have an explicit contract for env-only OTLP headers, including safe operator visibility and forbidden secret leakage surfaces.
  - Verification notes (commands or checks): Context review against this plan and existing observability/config contracts; no shell command required for context-only edits unless the implementing agent chooses to run markdown or generated-parity checks.
  - Completed: 2026-05-13
  - Files changed: `context/sce/cli-observability-contract.md`, `context/cli/config-precedence-contract.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/otel-auth-headers.md`
  - Evidence: Context review completed against this plan and existing observability/config contracts; no shell command required for context-only contract edits.
  - Notes: Defines env-only `OTEL_EXPORTER_OTLP_HEADERS` syntax, precedence exclusion from config files, redacted operator visibility, validation behavior, exporter application target, and secret-leakage exclusions for follow-up implementation tasks.

- [x] T02: Add OTLP header env resolution and redacted inspection surface (status:done)
  - Task ID: T02
  - Goal: Extend the config/runtime observability resolver to recognize `OTEL_EXPORTER_OTLP_HEADERS` as an env-only optional value and expose safe provenance in `sce config show`.
  - Boundaries (in/out of scope): In - env constant ownership in `cli/src/services/config/mod.rs`, resolved observability runtime shape, config-show text/JSON reporting that indicates header presence/source with redacted or boolean-only value, and focused resolver tests. Out - adding a `.sce/config.json` key, generated JSON schema changes for secret header values, exporter wiring, and live network verification.
  - Done when: With `OTEL_EXPORTER_OTLP_HEADERS` set, `sce config show` reports header auth as configured without printing raw header names/values that may be sensitive; with it unset, output remains deterministic and backwards compatible.
  - Verification notes (commands or checks): Targeted Rust tests for config resolution and show rendering; prefer `nix develop -c sh -c 'cd cli && cargo test config -- --exact'` only if an exact test exists or use the narrowest matching config tests, then rely on final `nix flake check` in T05.
  - Completed: 2026-05-13
  - Files changed: `cli/src/services/config/mod.rs`, `context/plans/otel-auth-headers.md`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed. Direct targeted Cargo test was blocked by the repo bash policy that prefers `nix flake check`.
  - Notes: Added env-only `OTEL_EXPORTER_OTLP_HEADERS` ownership to the config service, redacted configured/unset inspection in `sce config show` text and JSON output, and focused tests that assert placeholder header content is not emitted.

- [x] T03: Parse and apply OTLP headers to exporters (status:done)
  - Task ID: T03
  - Goal: Parse `OTEL_EXPORTER_OTLP_HEADERS` using standard OTLP comma-separated `key=value` syntax and apply the resulting headers to gRPC and HTTP/protobuf span exporters.
  - Boundaries (in/out of scope): In - parser/helper functions in the observability/config-owned seam, deterministic validation errors for malformed pairs, duplicate/empty key handling per the T01 contract, exporter builder integration in `TelemetryRuntime`, and unit tests that avoid real tokens. Out - collector-specific token acquisition, external network tests, retry/backoff changes, and broad telemetry architecture refactors.
  - Done when: Valid header strings are passed into both exporter builder variants, malformed strings fail before command dispatch with actionable guidance, and tests prove raw header values are not emitted by errors or log output.
  - Verification notes (commands or checks): Targeted Rust tests for header parsing, exporter config construction where practical, and redaction-sensitive error cases; `nix develop -c sh -c 'cd cli && cargo check'` for compile confidence after exporter builder changes.
  - Completed: 2026-05-13
  - Files changed: `cli/Cargo.toml`, `cli/Cargo.lock`, `cli/src/services/config/mod.rs`, `cli/src/services/observability.rs`, `context/plans/otel-auth-headers.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt && cargo check'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed. Targeted `nix develop -c sh -c 'cd cli && cargo test otlp_headers'` was blocked by the repo bash policy that prefers `nix flake check`.
  - Notes: Added redaction-safe OTLP header parsing gated on OTEL enablement, duplicate/empty/malformed-pair rejection without echoing raw header input, parsed header propagation into `TelemetryRuntime`, and gRPC/HTTP exporter builder header application tests using placeholder-only values.

- [x] T04: Add operator-facing OTLP auth examples and safeguards (status:done)
  - Task ID: T04
  - Goal: Document how operators should provide hosted-collector OTLP auth headers safely, including Dash0-style usage, without committing secrets.
  - Boundaries (in/out of scope): In - focused docs/context examples showing env-var usage with placeholder tokens, config validation/show expectations, and reminders that `.sce/config.json` should not contain tokens. Out - real Dash0 token examples, dashboard setup, collector deployment guides, and generated-agent documentation unless a context review shows it is required.
  - Done when: Operators can run `sce` with `SCE_OTEL_ENABLED=true`, Dash0 endpoint/protocol settings, and `OTEL_EXPORTER_OTLP_HEADERS='Authorization=Bearer <token>'` while understanding how to verify redacted configuration locally.
  - Verification notes (commands or checks): Review docs for placeholder-only secrets and consistency with T01 contract; no live collector required.
  - Completed: 2026-05-13
  - Files changed: `context/sce/otel-auth-operator-usage.md`, `context/context-map.md`, `context/sce/cli-observability-contract.md`, `context/plans/otel-auth-headers.md`
  - Evidence: Placeholder-only docs review completed (`Authorization=Bearer <token>` only; no real collector tokens); `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - Notes: Added hosted-collector/Dash0-style env-var examples for HTTP/protobuf and gRPC without vendor-specific config keys, redacted `sce config show` verification expectations, and safeguards against storing OTLP auth secrets in `.sce/config.json`, schemas, tests, context, or temporary artifacts.

- [x] T05: Validate, cleanup, and sync context (status:done)
  - Task ID: T05
  - Goal: Run final repo validation, remove temporary scaffolding, and sync durable context with the implemented OTLP auth state.
  - Boundaries (in/out of scope): In - full validation, generated-output parity check if schema/generated sources changed, cleanup of temporary log/test artifacts, and current-state context updates for accepted behavior. Out - adding new auth mechanisms, changing Dash0 endpoint config, or broad observability event expansion beyond OTLP auth headers.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass, no test tokens or temporary runtime artifacts are staged, and context accurately reflects the env-only OTLP header auth behavior.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect changed files for accidental secret values or generated drift.
  - Completed: 2026-05-13
  - Files changed: `context/plans/otel-auth-headers.md`, `context/overview.md`, `context/glossary.md`
  - Evidence: `nix run .#pkl-check-generated` passed with generated outputs up to date; `nix flake check` passed with all checks passing; `git diff --cached --check` and `git diff --check` passed; changed-file review found only placeholder header examples (`Bearer placeholder` / `<token>`) and no staged runtime scaffolding or untracked artifacts.
  - Notes: Final context sync verified root context and feature docs represent env-only OTLP header auth, redacted `sce config show` visibility, enabled-OTEL validation, gRPC/HTTP exporter application, and the updated direct `http`/`tonic` dependency baseline.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all checks passed.
- `git diff --cached --check && git diff --check` -> exit 0; no diff whitespace/errors reported.
- `git ls-files --others --exclude-standard` -> exit 0; no untracked nonignored files reported.

### Success-criteria verification

- [x] `OTEL_EXPORTER_OTLP_HEADERS` accepted as the first supported OTLP auth mechanism -> confirmed in `cli/src/services/config/mod.rs` and `cli/src/services/observability.rs`.
- [x] Header values are env-only and no config-file key is added -> confirmed by generated parity pass and context/schema review.
- [x] Standard comma-separated `key=value` syntax, including `Authorization=Bearer <token>`, is documented and tested with placeholders only.
- [x] Invalid syntax fails deterministically when OTEL is enabled and does not echo raw header material -> confirmed by parser/resolution tests in `cli/src/services/config/mod.rs`.
- [x] Parsed headers are applied to both `grpc` and `http/protobuf` exporters -> confirmed by exporter conversion tests in `cli/src/services/observability.rs`.
- [x] `sce config show` reports header auth only through redacted presence/provenance -> confirmed by implementation and context contracts.
- [x] Logs, diagnostics, file sinks, tests, docs, and context avoid raw secrets -> changed-file review found placeholders only and no real token material.
- [x] Existing stdout/stderr and endpoint/protocol behavior remain covered by `nix flake check`.
- [x] Required final checks passed: `nix run .#pkl-check-generated` and `nix flake check`.

### Failed checks and follow-ups

- None.

### Residual risks

- No live hosted collector was exercised; this remains intentionally out of scope for automated validation.

## Open questions

None. The user clarified env-only standard `OTEL_EXPORTER_OTLP_HEADERS` support as the initial auth target.
