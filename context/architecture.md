# Architecture

## Config generation boundary (current approved design)

The repository keeps two parallel config target trees:

- `config/.opencode`
- `config/.claude`

For authored config content, generation is standardized around one canonical Pkl source model with target-specific rendering applied later in the pipeline.

Current scaffold location for canonical shared content primitives:

- `config/pkl/base/shared-content.pkl` (manual profile aggregation surface)
- `config/pkl/base/shared-content-common.pkl` (shared primitives)
- `config/pkl/base/shared-content-plan.pkl` (plan-focused content)
- `config/pkl/base/shared-content-code.pkl` (code-focused content)
- `config/pkl/base/shared-content-commit.pkl` (commit-focused content)
- `config/pkl/base/shared-content-automated.pkl` (automated profile aggregation surface)
- `config/pkl/base/shared-content-automated-common.pkl`
- `config/pkl/base/shared-content-automated-plan.pkl`
- `config/pkl/base/shared-content-automated-code.pkl`
- `config/pkl/base/shared-content-automated-commit.pkl`
- `config/pkl/base/opencode.pkl`
- `config/pkl/base/sce-config-schema.pkl`

Current target renderer helper modules:

- `config/pkl/renderers/opencode-content.pkl`
- `config/pkl/renderers/opencode-automated-content.pkl`
- `config/pkl/renderers/claude-content.pkl`
- `config/pkl/renderers/common.pkl`
- `config/pkl/renderers/opencode-metadata.pkl`
- `config/pkl/renderers/opencode-automated-metadata.pkl`
- `config/pkl/renderers/claude-metadata.pkl`
- `config/pkl/renderers/metadata-coverage-check.pkl`
- `config/pkl/generate.pkl` (single multi-file generation entrypoint)
- `config/pkl/check-generated.sh` (dev-shell integration stale-output detection against committed generated files)
- `nix flake check` / `checks.<system>.{cli-tests,cli-clippy,cli-fmt,integrations-install-tests,integrations-install-clippy,integrations-install-fmt,npm-bun-tests,npm-biome-check,npm-biome-format,config-lib-bun-tests,config-lib-biome-check,config-lib-biome-format}` (root-flake check derivations for the current CLI, `integrations/install` runner, and JS validation inventory)
- `.github/workflows/pkl-generated-parity.yml` (CI wrapper that runs the parity check for pushes to `main` and pull requests targeting `main`)

The scaffold provides stable canonical content-unit identifiers and reusable target-agnostic text primitives for all planned authored generated classes (agents, commands, skills, shared runtime assets, OpenCode plugin entrypoints, and generated OpenCode package manifests).

Renderer modules apply target-specific metadata/frontmatter rules while reusing canonical content bodies:

- OpenCode renderer emits frontmatter with `agent`/`permission`/`compatibility: opencode` conventions; targeted SCE commands also emit machine-readable `entry-skill` and ordered `skills` metadata when the renderer explicitly defines that mapping.
- Claude renderer emits frontmatter with `allowed-tools`/`model`/`compatibility: claude` conventions.
- Shared renderer contracts (`RenderedTargetDocument`, command descriptions) live in `config/pkl/renderers/common.pkl`.
- The canonical OpenCode plugin-registration source for generated SCE plugins lives in `config/pkl/base/opencode.pkl`; `config/pkl/renderers/common.pkl` re-exports the shared plugin list and JSON-ready paths for OpenCode renderers, and the current generated registration scope is limited to SCE-managed plugins emitted by this repo.
- Target-specific metadata tables, including skill frontmatter descriptions, are isolated in `config/pkl/renderers/opencode-metadata.pkl`, `config/pkl/renderers/opencode-automated-metadata.pkl`, and `config/pkl/renderers/claude-metadata.pkl`.
- Metadata key coverage is enforced by `config/pkl/renderers/metadata-coverage-check.pkl`, which resolves all required lookup keys for both targets and fails evaluation on missing entries.
- Both renderers expose per-class rendered document objects (`agents`, `commands`, `skills`) consumed by `config/pkl/generate.pkl`.
- `config/pkl/generate.pkl` emits deterministic `output.files` mappings for all authored generated targets: OpenCode/Claude agents, commands, skills, shared bash-policy runtime and preset assets under `lib/`, the OpenCode bash-policy plugin entrypoint under `plugins/`, generated OpenCode `package.json` and `opencode.json` manifests for manual and automated profiles, and the generated `sce/config.json` schema artifact at `config/schema/sce-config.schema.json`.
- Generated-file warning markers are not injected by the generator: Markdown outputs render deterministic frontmatter + body, and shared library outputs are emitted without a leading generated warning header.
- `config/pkl/check-generated.sh` is intentionally dev-shell scoped (`nix develop -c ...`): it requires `IN_NIX_SHELL`, runs `pkl eval -m <tmp> config/pkl/generate.pkl`, and fails when generated-owned paths drift.

Generated authored classes:

- agent definitions
- command definitions
- skill definitions
- shared runtime library files
- OpenCode plugin entrypoints
- generated OpenCode package manifests

Explicitly excluded from generation ownership:

- runtime dependency artifacts (for example `node_modules`)
- lockfiles and install outputs

See `context/decisions/2026-02-28-pkl-generation-architecture.md` for the full matrix and ownership table used by the plan task implementation.

## Placeholder SCE CLI boundary

The repository includes a new placeholder Rust binary crate at `cli/`.

## First-wave CLI install/distribution boundary

- The current implemented first-wave install/distribution surface for the `sce` CLI is limited to repo-flake Nix, Cargo, and npm; `Homebrew` is deferred from the active implementation stage.
- Nix-managed build/release entrypoints are the source of truth for first-wave build outputs and release automation.
- The root flake now also owns an opt-in install-channel integration-test app surface, `apps.install-channel-integration-tests`, which provides the execution contract for npm/Bun/Cargo install verification without enrolling that heavier coverage in default `checks.<system>`.
- Repo-root `.version` is the canonical checked-in release version authority across GitHub Releases, Cargo publication, and npm publication.
- GitHub Releases are the canonical publication surface for release archives, checksums, and release-manifest assets.
- Cargo/crates.io and npm registry publication belong to separate downstream publish stages that consume already-versioned checked-in package metadata rather than inventing release versions during workflow execution.
- npm is a downstream consumer of the shared release artifact contract rather than a separate build owner.
- `flake.nix` now exposes dedicated release-packaging apps for this rollout: `apps.release-artifacts` packages the current-platform `sce` archive/checksum/manifest-fragment set from `packages.default` and fails if `.version`, `cli/Cargo.toml`, `npm/package.json`, or the built CLI version disagree; `apps.release-manifest` merges per-platform metadata fragments into release-level manifest/checksum outputs and signs the merged manifest with a non-repo private key provided via env/file input; and `apps.release-npm-package` packs the checked-in npm launcher package into a release-ready `sce-v<version>-npm.tgz` tarball plus npm metadata JSON while refusing mismatched checked-in version metadata.
- CLI release automation is split from Tessl release automation: `.github/workflows/release-sce.yml` orchestrates `sce` GitHub release asset assembly, validates `.version`/Cargo/npm parity before tagging or releasing, and now calls three reusable per-platform workflow files (`release-sce-linux.yml`, `release-sce-linux-arm.yml`, `release-sce-macos-arm.yml`) before assembling the release manifest and publishing GitHub release assets; `.github/workflows/release-agents.yml` stays scoped to agent-file release assets.
- Downstream registry publication is now implemented in dedicated workflows: `.github/workflows/publish-crates.yml` publishes the checked-in crate version from a published release or manual dispatch after `.version`/tag/Cargo parity checks, and `.github/workflows/publish-npm.yml` publishes the checked-in npm package version from a published release or manual dispatch after `.version`/tag/npm parity checks plus verification of the canonical `sce-v<version>-npm.tgz` GitHub release asset.
- The npm distribution implementation lives under `npm/`: `package.json` defines the `sce` package surface, `bin/sce.js` launches the package-local native binary, `lib/install.js` resolves the current package version against the release manifest, verifies `sce-v<version>-release-manifest.json.sig` with the bundled public key before trusting manifest contents, and then installs the checksum-verified native archive for supported macOS/Linux targets, while `test/platform.test.js` and `test/install.test.js` cover platform selection plus signed-manifest installer behavior.

- `cli/src/main.rs` is the executable entrypoint (`sce`) and delegates to `app::run`.
- `cli/src/cli_schema.rs` defines the clap-based CLI schema using derive macros for all top-level commands and subcommands, and renders command-local help text for the `auth` command tree (`auth`, `auth login`, `auth logout`, `auth status`).
- `cli/src/app.rs` provides the clap-based argument dispatch loop with deterministic help/setup execution, bare-command help routing for `sce auth` and `sce config`, centralized stream routing (`stdout` success payloads, `stderr` redacted diagnostics), stable class-based exit-code mapping (`2` parse, `3` validation, `4` runtime, `5` dependency), and stable class-based stderr diagnostic codes (`SCE-ERR-PARSE`, `SCE-ERR-VALIDATION`, `SCE-ERR-RUNTIME`, `SCE-ERR-DEPENDENCY`) with default `Try:` remediation injection when missing.
- The app runtime now moves through explicit startup phases in `cli/src/app.rs`: dependency bootstrapping (`perform_dependency_check`), startup context construction (`build_startup_context`), runtime initialization (`initialize_runtime`), command parse/execute inside telemetry subscriber context (`run_command_lifecycle`, `parse_command_phase`, `execute_command_phase`), and final output rendering (`render_run_outcome`). `AppRuntime` is the named runtime context that carries logger, telemetry, and startup-diagnostic state across those phases.
- Command parse-time conversion and run-time handling are now separated by an internal `RuntimeCommand` seam in `cli/src/app.rs`. `parse_command_phase` delegates clap-output conversion to the nested `command_runtime` module, which returns boxed command objects that own their own `name()` and `execute(...)` behavior. `execute_command_phase` now emits lifecycle logs around `command.execute(...)` instead of switching over one central app-level dispatch `match`.
- Startup observability bootstrapping in `cli/src/app.rs` still tolerates invalid default-discovered config files by continuing with degraded defaults plus `sce.config.invalid_config` warn-level logs, but the warning/logging work is now isolated behind the startup-context and runtime-initialization phases rather than one inline startup function.
- `cli/src/services/observability.rs` provides deterministic runtime observability controls and rendering for app lifecycle logs, including shared config-resolved threshold/format and OTEL/file-sink inputs with precedence `env > config file > defaults` for non-flag observability keys, optional file sink controls (`SCE_LOG_FILE`, `SCE_LOG_FILE_MODE` with deterministic truncate-or-append policy), optional OTEL export bootstrap (`SCE_OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`), stable event identifiers, severity filtering, the forced-emission warning path used for invalid discovered config startup diagnostics, stderr-only primary emission with optional mirrored file writes, and redaction-safe emission through the shared security helper.
- `cli/src/services/observability.rs` no longer owns duplicate log/OTEL enums or parsing helpers; it consumes the canonical primitive seam from `cli/src/services/config.rs` and stays focused on logger and telemetry runtime behavior.
- `cli/src/command_surface.rs` is the source of truth for top-level command contract metadata (`help`, `config`, `setup`, `doctor`, `auth`, `hooks`, `sync`, `version`, `completion`), including implementation status for command classification and per-command visibility on the slim top-level help surface.
- `cli/src/services/default_paths.rs` is the canonical production path catalog for the CLI: it resolves config/state/cache roots with platform-aware XDG or `dirs` fallbacks, exposes named default paths plus an explicit persisted-artifact inventory for the current default artifacts (global config, auth tokens, local DB), and owns canonical repo-relative, embedded-asset, install/runtime, hook, and context-path accessors so non-test production path definitions have one shared owner. Current production consumers such as config discovery, doctor reporting, setup/install flows, and local hook runtime path resolution consume this shared catalog rather than defining owned path literals in their own modules.
- `cli/src/services/config.rs` defines `sce config` parser/runtime contracts (`show`, `validate`, `--help`), with bare `sce config` routed by `cli/src/app.rs` to the same help payload as `sce config --help`, deterministic config-file selection, explicit value precedence (`flags > env > config file > defaults`), strict config-file validation (`$schema`, `log_level`, `log_format`, `log_file`, `log_file_mode`, `timeout_ms`, `workos_client_id`, nested `otel`, `policies.bash`, and `policies.attribution_hooks.enabled`), compile-time embedding of the canonical generated schema artifact and bash-policy preset catalog from the ephemeral crate-local `cli/assets/generated/config/**` mirror prepared from canonical `config/` outputs before Cargo packaging/builds, runtime parity between that schema and the Rust-side top-level allowed-key gate so startup config discovery accepts the canonical `"$schema": "https://sce.crocoder.dev/config.json"` declaration, graceful fallback for parse/schema/top-level-object failures in default-discovered config files (collected as `validation_errors` while defaults continue to resolve), explicit-path hard failures for `--config` and `SCE_CONFIG_FILE`, shared auth-key resolution with optional baked defaults starting at `workos_client_id`, repo-configured bash-policy preset/custom validation and merged reporting from discovered config files, a shared attribution-hooks runtime gate resolved from `SCE_ATTRIBUTION_HOOKS_ENABLED` or `policies.attribution_hooks.enabled` with disabled-by-default semantics, shared observability-runtime resolution for logging and OTEL config keys (`log_level`, `log_format`, `log_file`, `log_file_mode`, `otel.enabled`, `otel.exporter_otlp_endpoint`, `otel.exporter_otlp_protocol`) with deterministic source-aware env-over-config precedence reused by `cli/src/app.rs`, and deterministic text/JSON output rendering where `show` includes resolved observability/auth/policy values with provenance while `validate` now returns only validation status plus issues/warnings.
- `cli/src/services/output_format.rs` defines the canonical shared CLI output-format contract (`OutputFormat`) for supporting commands, with deterministic `text|json` parsing and command-scoped actionable invalid-value guidance.
- `cli/src/services/config.rs` is also the canonical owner for the shared runtime/config primitive seam used by the CLI: `LogLevel`, `LogFormat`, `LogFileMode`, `OtlpProtocol`, the observability env-key constants, and the OTEL/bool parsing helpers used by both config resolution and observability bootstrap.
- `cli/src/services/auth_command.rs` defines the implemented auth command surface for `sce auth login|renew|logout|status`, including device-flow login, stored-token renewal (`--force` supported for renew), logout, and status rendering in text/JSON formats.
- `cli/src/services/local_db.rs` provides the local Turso data adapter, including `Builder::new_local(...)` initialization, deterministic persistent runtime DB target resolution through the shared default-path seam, a neutral per-user local DB baseline without schema bootstrap, and async execute/query smoke checks for in-memory and file-backed targets.
- `cli/src/test_support.rs` provides a shared test-only temp-directory helper (`TestTempDir`) used by service tests that need filesystem fixtures.
- `cli/src/services/setup.rs` defines the setup command contract (`SetupMode`, `SetupTarget`, `SetupRequest`, CLI flag parser/validator), an `inquire`-backed interactive target prompter (`InquireSetupTargetPrompter`), setup dispatch outcomes (proceed/cancelled), compile-time embedded asset access (`EmbeddedAsset`, target-scoped iterators, required-hook asset iterators/lookups) generated by `cli/build.rs` from the ephemeral crate-local `cli/assets/generated/config/{opencode,claude}/**` mirror plus `cli/assets/hooks/**`, a target-scoped install engine/orchestrator that stages embedded files and uses a unified remove-and-replace policy (removing existing targets before swapping staged content, with deterministic recovery guidance on swap failure and no backup artifact creation), and formats deterministic completion messaging, plus required-hook install orchestration (`install_required_git_hooks`) that follows the same remove-and-replace policy (removing existing hooks before swapping staged content, with deterministic recovery guidance on swap failure), along with command-surface setup request resolution helpers (`run_setup_hooks`, `resolve_setup_request`) used by hooks-only and composable target+hooks setup invocations with deterministic option compatibility validation, canonicalized/validated repo targeting, write-permission probes, and stable section-ordered setup/hook outcome messaging.
- `cli/src/services/security.rs` provides shared security utilities for deterministic secret redaction (`redact_sensitive_text`) and directory write-permission probes (`ensure_directory_is_writable`) used by app/setup/observability surfaces.
- `cli/src/services/doctor.rs` now defines the current doctor request/report surface: explicit `DoctorMode` (`diagnose` vs `fix`), stable text/JSON problem records with category/severity/fixability/remediation fields, deterministic fix-result reporting in fix mode, current global operator-environment checks (shared state-root resolution, shared global-config-path resolution, repo-local config validation through the generated `sce/config.json` schema contract, local DB path/health, and DB-parent readiness barriers), repo/hook-integrity diagnostics that distinguish git-unavailable vs non-repo vs bare-repo states, an empty default repo-database section reflecting the current state where no repo-owned SCE database exists, repair-mode reuse of `cli/src/services/setup.rs::install_required_git_hooks` for missing/stale/non-executable required hooks and missing hooks directories, and a bounded doctor-owned repair routine that bootstraps the canonical SCE-owned local DB parent directory only when the resolved path matches the expected owned location.
- `cli/src/services/version.rs` defines the version command parser/rendering contract (`parse_version_request`, `render_version`) with deterministic text output and stable JSON runtime-identification fields.
- `cli/src/services/completion.rs` defines completion parser/rendering contract (`parse_completion_request`, `render_completion`) with deterministic Bash/Zsh/Fish script output aligned to current parser-valid command/flag surfaces.
- `cli/src/services/hooks.rs` defines the current minimal local hook runtime parsing/dispatch (`HookSubcommand`, `run_hooks_subcommand`) plus a commit-msg co-author policy seam (`apply_commit_msg_coauthor_policy`) that injects one canonical SCE trailer only when the disabled-default attribution-hooks config/env control is enabled and `SCE_DISABLED` is false. The other hook entrypoints (`pre-commit`, `post-commit`, `post-rewrite`) are deterministic no-op surfaces in the current attribution-only baseline.
- `cli/src/services/resilience.rs` defines bounded retry/timeout/backoff execution policy (`RetryPolicy`, `run_with_retry`) for transient operation hardening with deterministic failure messaging and retry observability.
- `cli/src/services/sync.rs` runs the local adapter through a lazily initialized shared tokio current-thread runtime, applies bounded resilience policy to the local smoke operation, and composes a placeholder cloud-sync abstraction (`CloudSyncGateway`) so local Turso validation and deferred cloud planning remain separated.
- `cli/src/services/` contains module boundaries for config, setup, doctor, hooks, sync, version, completion, and local DB adapters with explicit trait seams for future implementations.
- `cli/README.md` is the crate-local onboarding and usage source of truth for placeholder behavior, safety limitations, and roadmap mapping back to service contracts.
- `flake.nix` applies `rust-overlay` (`oxalica/rust-overlay`) to nixpkgs, pins `rust-bin.stable.1.93.1.default` with `rustfmt` + `clippy`, reads the package/check version from repo-root `.version`, builds `packages.sce` through Crane (`buildDepsOnly` -> `buildPackage`) with a filtered repo-root source that preserves the Cargo tree plus `cli/assets/hooks`, then injects generated OpenCode/Claude config payloads and schema inputs into a temporary `cli/assets/generated/` mirror during derivation unpack so `cli/build.rs` can package the crate without requiring committed generated crate assets, runs `cli-tests`, `cli-clippy`, and `cli-fmt` plus the dedicated `integrations-install-tests`, `integrations-install-clippy`, and `integrations-install-fmt` derivations through Crane-backed paths so both Rust crates have first-class default-flake verification, exposes directory-scoped JS validation derivations for both `npm/` and `config/lib/bash-policy-plugin/`, and also exposes the non-default `apps.install-channel-integration-tests` flake app for install-channel integration coverage outside the default check set. `.github/workflows/publish-crates.yml` follows the same asset-preparation rule but runs Cargo packaging from a temporary clean repository copy so crates.io publish no longer needs `--allow-dirty`.
- `flake.nix` exposes release install/run surfaces as `packages.sce` (`packages.default = packages.sce`) plus `apps.sce` and `apps.default`, all targeting `${packages.sce}/bin/sce`; this keeps repo-local and remote flake run/install flows (`nix run .`, `nix run github:crocoder-dev/shared-context-engineering`, `nix profile install github:crocoder-dev/shared-context-engineering`) aligned to the same packaged CLI output.
- `biome.json` at the repository root is the canonical Biome configuration for the current JS tooling slice and deliberately scopes coverage to `npm/**` plus `config/lib/bash-policy-plugin/**` while excluding package-local `node_modules/**`; `flake.nix` exposes Biome through the default dev shell rather than through package-local installs.
- `cli/Cargo.toml` now keeps crates.io publication-ready package metadata for the `shared-context-engineering` crate, and `cli/README.md` is the Cargo install surface for crates.io (`cargo install shared-context-engineering --locked`), git (`cargo install --git https://github.com/crocoder-dev/shared-context-engineering shared-context-engineering --locked`), and local checkout (`cargo install --path cli --locked`) guidance. The published crate installs the `sce` binary. Tokio remains intentionally constrained (`default-features = false`) with current-thread runtime usage plus timer-backed bounded resilience wrappers for retry/timeout behavior.
- `cli/Cargo.toml` now declares Tokio's `time` feature directly alongside the existing constrained current-thread runtime setup (`rt`, `io-util`, `time`) instead of relying on transitive enablement.

This phase establishes compile-safe extension seams with a dependency baseline (`anyhow`, `clap`, `clap_complete`, `dirs`, `hmac`, `inquire`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `reqwest`, `serde`, `serde_json`, `sha2`, `tokio`, `tracing`, `tracing-opentelemetry`, `tracing-subscriber`, `turso`); local Turso connectivity smoke checks now exist, while broader runtime integrations remain deferred.

## SCE plan/code role boundary

Shared Context Plan and Shared Context Code remain separate architectural roles.

- Shared Context Plan owns planning and approval-readiness in `context/plans/` and does not execute implementation edits.
- Shared Context Code owns exactly one approved task execution, validation, and mandatory `context/` synchronization.
- `/change-to-plan` and `/next-task` remain separate command entrypoints aligned to those roles.
- Reuse is handled through shared canonical guidance blocks and skill-owned phase contracts, not by collapsing both roles into one agent.
- Shared baseline doctrine for both agents is centralized in reusable constants in `config/pkl/base/shared-content-common.pkl` and interpolated into each role body at generation time; the aggregation surfaces `config/pkl/base/shared-content.pkl` (manual) and `config/pkl/base/shared-content-automated.pkl` (automated) import from grouped `plan`, `code`, and `commit` modules for downstream renderers.
- `/next-task` is a thin orchestration wrapper: it owns gate sequencing, while phase-detail contracts stay canonical in `sce-plan-review`, `sce-task-execution`, and `sce-context-sync`.
- `/change-to-plan` is a thin orchestration wrapper: it delegates clarification and plan-shape ownership to `sce-plan-authoring` (including one-task/one-atomic-commit task slicing) while retaining wrapper-level plan creation confirmation and `/next-task` handoff obligations.
- `/commit` is a thin orchestration wrapper: manual generated commands retain staged-changes confirmation and proposal-only behavior, while the automated OpenCode command skips the staging-confirmation gate, generates exactly one commit message through `sce-atomic-commit`, and runs `git commit` for the staged diff; commit grammar and plan-aware body rules stay canonical in `sce-atomic-commit`.
