# Architecture

## Config generation boundary (current approved design)

The repository keeps two parallel config target trees:

- `config/.opencode`
- `config/.claude`

For authored config content, generation is standardized around one canonical Pkl source model with target-specific rendering applied later in the pipeline.

Current scaffold location for canonical shared content primitives:

- `config/pkl/base/shared-content.pkl`

Current target renderer helper modules:

- `config/pkl/renderers/opencode-content.pkl`
- `config/pkl/renderers/claude-content.pkl`
- `config/pkl/renderers/common.pkl`
- `config/pkl/renderers/opencode-metadata.pkl`
- `config/pkl/renderers/claude-metadata.pkl`
- `config/pkl/renderers/metadata-coverage-check.pkl`
- `config/pkl/generate.pkl` (single multi-file generation entrypoint)
- `config/pkl/check-generated.sh` (dev-shell integration stale-output detection against committed generated files)
- `nix run .#sync-opencode-config` (flake app entrypoint for config regeneration and sync workflow)
- `nix run .#token-count-workflows` (flake app entrypoint for static workflow token-count execution via `evals/token-count-workflows.ts`)
- `nix flake check` / `checks.<system>.cli-setup-command-surface` (flake check derivation that runs targeted CLI setup command-surface verification from `cli/`)
- `.github/workflows/pkl-generated-parity.yml` (CI wrapper that runs the parity check for pushes to `main` and pull requests targeting `main`)
- `.github/workflows/agnix-config-validate-report.yml` (CI wrapper that runs `agnix validate` from `config/`, writes `context/tmp/ci-reports/agnix-validate-report.txt`, uploads it when non-info findings are present, and fails on any non-info finding)
- `.github/workflows/workflow-token-count.yml` (CI wrapper that runs `nix run .#token-count-workflows` for pushes/pull requests targeting `main` and uploads token-footprint artifacts from `context/tmp/token-footprint/`)

The scaffold provides stable canonical content-unit identifiers and reusable target-agnostic text primitives for all planned authored generated classes (agents, commands, skills, shared library file).

Renderer modules apply target-specific metadata/frontmatter rules while reusing canonical content bodies:

- OpenCode renderer emits frontmatter with `agent`/`permission`/`compatibility: opencode` conventions.
- Claude renderer emits frontmatter with `allowed-tools`/`model`/`compatibility: claude` conventions.
- Shared renderer contracts (`RenderedTargetDocument`, command descriptions, skill descriptions) live in `config/pkl/renderers/common.pkl`.
- Target-specific metadata tables are isolated in `config/pkl/renderers/opencode-metadata.pkl` and `config/pkl/renderers/claude-metadata.pkl`.
- Metadata key coverage is enforced by `config/pkl/renderers/metadata-coverage-check.pkl`, which resolves all required lookup keys for both targets and fails evaluation on missing entries.
- Both renderers expose per-class rendered document objects (`agents`, `commands`, `skills`) consumed by `config/pkl/generate.pkl`.
- `config/pkl/generate.pkl` emits deterministic `output.files` mappings for all authored generated targets: OpenCode/Claude agents, commands, skills, and `lib/drift-collectors.js` in both trees.
- Generated-file warning markers are not injected by the generator: Markdown outputs render deterministic frontmatter + body, and shared library outputs are emitted without a leading generated warning header.
- `config/pkl/check-generated.sh` is intentionally dev-shell scoped (`nix develop -c ...`): it requires `IN_NIX_SHELL`, runs `pkl eval -m <tmp> config/pkl/generate.pkl`, and fails when generated-owned paths drift.

Current sync-command state:

- `sync-opencode-config` is exported as a flake app from `flake.nix` and is runnable through `nix run .#sync-opencode-config`.
- The app regenerates generated-owned `config/` outputs in a staging workspace, validates expected generated directories, and only then replaces live `config/`.
- After `config/` replacement, the app replaces repository-root `.opencode/` from staged `config/.opencode/` using explicit runtime exclusions.
- Root replacement uses backup-and-restore safety semantics plus post-copy parity verification (`diff -rq` with exclusion filters) before finalizing.

Generated authored classes:

- agent definitions
- command definitions
- skill definitions
- shared drift collector library file

Explicitly excluded from generation ownership:

- runtime dependency artifacts (for example `node_modules`)
- lockfiles and install outputs
- package/tool manifests not listed in generated authored scope

See `context/decisions/2026-02-28-pkl-generation-architecture.md` for the full matrix and ownership table used by the plan task implementation.

## Placeholder SCE CLI boundary

The repository includes a new placeholder Rust binary crate at `cli/`.

- `cli/src/main.rs` is the executable entrypoint (`sce`) and delegates to `app::run`.
- `cli/src/app.rs` provides a `lexopt`-based argument parser and dispatch loop with deterministic help/setup execution, centralized stream routing (`stdout` success payloads, `stderr` redacted diagnostics), and stable class-based exit-code mapping (`2` parse, `3` validation, `4` runtime, `5` dependency).
- `cli/src/services/observability.rs` provides deterministic runtime observability controls and rendering for app lifecycle logs, including env-configured threshold/format (`SCE_LOG_LEVEL`, `SCE_LOG_FORMAT`), optional file sink controls (`SCE_LOG_FILE`, `SCE_LOG_FILE_MODE` with deterministic truncate-or-append policy), optional OTEL export bootstrap (`SCE_OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`), stable event identifiers, severity filtering, stderr-only primary emission with optional mirrored file writes, and redaction-safe emission through the shared security helper.
- `cli/src/command_surface.rs` is the source of truth for top-level command contract metadata (`help`, `config`, `setup`, `doctor`, `mcp`, `hooks`, `sync`) and explicit implemented-vs-placeholder status.
- `cli/src/services/config.rs` defines `sce config` parser/runtime contracts (`show`, `validate`, `--help`), deterministic config-file selection, explicit value precedence (`flags > env > config file > defaults`), strict config-file validation (`log_level`, `timeout_ms`), and deterministic text/JSON output rendering.
- `cli/src/services/local_db.rs` provides the local Turso data adapter, including `Builder::new_local(...)` initialization, deterministic persistent runtime DB target resolution/bootstrap (`ensure_agent_trace_local_db_ready_blocking`), async execute/query smoke checks for in-memory and file-backed targets, and idempotent migration application for Agent Trace persistence foundations (`repositories`, `commits`, `trace_records`, `trace_ranges`), reconciliation ingestion entities (`reconciliation_runs`, `rewrite_mappings`, `conversations`), and T14 retry/observability storage (`trace_retry_queue`, `reconciliation_metrics`) with replay/query indexes.
- `cli/src/test_support.rs` provides a shared test-only temp-directory helper (`TestTempDir`) used by service tests that need filesystem fixtures.
- `cli/src/services/setup.rs` defines the setup command contract (`SetupMode`, `SetupTarget`, CLI flag parser/validator), an `inquire`-backed interactive target prompter (`InquireSetupTargetPrompter`), setup dispatch outcomes (proceed/cancelled), compile-time embedded asset access (`EmbeddedAsset`, target-scoped iterators, required-hook asset iterators/lookups) generated by `cli/build.rs` from `config/.opencode/**`, `config/.claude/**`, and `cli/assets/hooks/**`, a target-scoped install engine/orchestrator that stages embedded files, performs backup-and-replace with rollback restoration on swap failure, and formats deterministic completion messaging, plus required-hook install orchestration (`install_required_git_hooks`) and command-surface hook mode helpers (`run_setup_hooks`, `resolve_setup_hooks_repository`) used by `sce setup --hooks [--repo <path>]` with deterministic option compatibility validation, canonicalized/validated repo targeting, write-permission probes, and per-hook outcome messaging.
- `cli/src/services/security.rs` provides shared security utilities for deterministic secret redaction (`redact_sensitive_text`) and directory write-permission probes (`ensure_directory_is_writable`) used by app/setup/observability surfaces.
- `cli/src/services/doctor.rs` defines hook rollout health validation (`run_doctor`) that resolves effective git hook-path source (default, local `core.hooksPath`, global `core.hooksPath`) and validates required hook files (`pre-commit`, `commit-msg`, `post-commit`) for presence and executable permissions.
- `cli/src/services/agent_trace.rs` defines the Agent Trace schema adapter and builder contracts (`adapt_trace_payload`, `build_trace_payload`), including fixed git VCS identity, reserved reverse-domain metadata keys, and deterministic AI `model_id` normalization before schema-compliance validation.
- `cli/src/services/mcp.rs` defines MCP file-cache capability contracts (`McpService`, transport/capability snapshots, cache policy) with non-runnable placeholder tool declarations.
- `cli/src/services/hooks.rs` defines production local hook runtime parsing/dispatch (`HookSubcommand`, `parse_hooks_subcommand`, `run_hooks_subcommand`) plus a pre-commit staged-checkpoint finalization seam (`finalize_pre_commit_checkpoint`) that enforces staged-only attribution and carries index/tree anchors with explicit no-op guard states, a commit-msg co-author policy seam (`apply_commit_msg_coauthor_policy`) that injects one canonical SCE trailer only for allowed attributed commits, a post-commit trace finalization seam (`finalize_post_commit_trace`) that performs notes+DB dual writes with idempotency ledger guards and retry-queue fallback capture, a retry replay seam (`process_trace_retry_queue`) that re-attempts only failed persistence targets and emits per-attempt runtime/error-class metrics, bounded operational retry replay invocation from post-commit/post-rewrite flows (`process_runtime_retry_queue`), a post-rewrite remap-ingestion seam (`finalize_post_rewrite_remap`) that parses old->new SHA pairs and derives deterministic replay keys for remap dispatch, and a rewrite trace transformation seam (`finalize_rewrite_trace`) that emits rewritten-SHA Agent Trace records with rewrite metadata plus confidence-based quality status.
- `cli/src/services/hosted_reconciliation.rs` defines hosted intake/orchestration seams (`ingest_hosted_rewrite_event`, `ReconciliationRunStore`) that verify provider signatures (GitHub HMAC-SHA256 and GitLab token equality), parse provider payload old/new heads, normalize deterministic idempotency-backed reconciliation run requests, resolve deterministic old->new rewrite mappings (`map_rewritten_commit`) with patch-id exact precedence, range-diff/fuzzy fallback scoring, and explicit unresolved classifications, and summarize mapped/unmapped confidence/runtime/error-class telemetry (`summarize_reconciliation_metrics`).
- `cli/src/services/resilience.rs` defines bounded retry/timeout/backoff execution policy (`RetryPolicy`, `run_with_retry`) for transient operation hardening with deterministic failure messaging and retry observability.
- `cli/src/services/sync.rs` runs the local adapter through a lazily initialized shared tokio current-thread runtime, applies bounded resilience policy to the local smoke operation, and composes a placeholder cloud-sync abstraction (`CloudSyncGateway`) so local Turso validation and deferred cloud planning remain separated.
- `cli/src/services/` contains module boundaries for config, setup, doctor, MCP, hooks, sync, and local DB adapters with explicit trait seams for future implementations.
- `cli/README.md` is the crate-local onboarding and usage source of truth for placeholder behavior, safety limitations, and roadmap mapping back to service contracts.
- `cli/flake.nix` applies `rust-overlay` (`oxalica/rust-overlay`) to nixpkgs, selects `rust-bin.stable.latest.default` with `rustfmt`, and routes CLI check/build derivations through `makeRustPlatform` so toolchain selection is explicit and deterministic.
- `cli/flake.nix` exposes release install/run surfaces as `packages.sce` (`packages.default = packages.sce`) and `apps.sce` targeting `${packages.sce}/bin/sce`, enabling packaged CLI build/run via `nix build ./cli#default` and `nix run ./cli#sce -- ...`.
- `flake.nix` (root) keeps nested CLI input wiring aligned by forwarding `nixpkgs`, `flake-utils`, and `rust-overlay` into the `cli` path input so repository-level `nix flake check` can evaluate nested CLI checks deterministically.
- `cli/Cargo.toml` keeps crates.io-ready package metadata populated while `publish = false` remains the current policy; local Cargo release/install verification targets `cargo build --manifest-path cli/Cargo.toml --release` and `cargo install --path cli --locked`. Tokio remains intentionally constrained (`default-features = false`) with current-thread runtime usage plus timer-backed bounded resilience wrappers for retry/timeout behavior.

This phase establishes compile-safe extension seams with a minimal dependency baseline (`anyhow`, `hmac`, `inquire`, `lexopt`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `serde_json`, `sha2`, `tokio`, `tracing`, `tracing-opentelemetry`, `tracing-subscriber`, `turso`); local Turso connectivity smoke checks now exist, while broader runtime integrations remain deferred.

## Shared Context Drift parity mapping

Shared Context Drift has an explicit target-parity mapping for internal/subagent usage across generated outputs.

- Canonical agent source remains `shared.agents["shared-context-drift"]` in `config/pkl/base/shared-content.pkl`.
- OpenCode subagent behavior is declared in `config/pkl/renderers/opencode-metadata.pkl` via `agentBehaviorBlocks["shared-context-drift"]`, which emits `mode: subagent` and `hidden: true` into `config/.opencode/agent/Shared Context Drift.md`.
- Claude has no supported `hidden`/`mode` equivalent in this repo's generator contract, so parity is represented with supported fields only: delegated/internal guidance in `agentDescriptions["shared-context-drift"]` and `agentSystemPreambleBlocks["shared-context-drift"]` in `config/pkl/renderers/claude-metadata.pkl`, rendered to `config/.claude/agents/shared-context-drift.md`.
- This is an intentional capability-gap mapping: OpenCode uses explicit frontmatter controls; Claude uses instruction-level delegation and command/task routing guidance.

## SCE plan/code role boundary

Shared Context Plan and Shared Context Code remain separate architectural roles.

- Shared Context Plan owns planning and approval-readiness in `context/plans/` and does not execute implementation edits.
- Shared Context Code owns exactly one approved task execution, validation, and mandatory `context/` synchronization.
- `/change-to-plan` and `/next-task` remain separate command entrypoints aligned to those roles.
- Reuse is handled through shared canonical guidance blocks and skill-owned phase contracts, not by collapsing both roles into one agent.
- Shared baseline doctrine for both agents is centralized in reusable constants in `config/pkl/base/shared-content.pkl` and interpolated into each role body at generation time.
- `/next-task` is a thin orchestration wrapper: it owns gate sequencing, while phase-detail contracts stay canonical in `sce-plan-review`, `sce-task-execution`, and `sce-context-sync`.
- `/change-to-plan` is a thin orchestration wrapper: it delegates clarification and plan-shape ownership to `sce-plan-authoring` (including one-task/one-atomic-commit task slicing) while retaining wrapper-level plan creation confirmation and `/next-task` handoff obligations.
- `/commit` is a thin orchestration wrapper: it retains staged-changes confirmation and no-auto-commit constraints, while commit grammar and atomic split logic stay canonical in `sce-atomic-commit`.
