# Patterns

## Config generation tooling

- Use the Nix dev shell as the canonical toolchain entrypoint for generation work.
- `flake.nix` includes `pkl` so contributors can run validation commands with `nix develop -c ...` without host-level installs.

## Verification guidance

- Prefer `nix flake check` for repository-level verification/check flows in contributor guidance.
- Keep direct Cargo verification commands as secondary targeted-debugging tools rather than the default repo-validation path.
- Keep `cargo fmt` available for explicit autofix formatting flows; do not present it as the preferred verification command.
- Gate workflow YAML edits through `checks.<system>.workflow-actionlint` inside root `nix flake check`; run `nix run nixpkgs#actionlint -- .github/workflows/*.yml` for targeted workflow lint during CI workflow edits.
- Keep PR validation in `.github/workflows/pr-ci.yml` (`Nix CI`): triggers on `pull_request`, `push` to `main`, and `workflow_dispatch`; `ubuntu-latest` + `macos-latest` matrix with `fail-fast: false`; pinned `DeterminateSystems/nix-installer-action@v22` with `DeterminateSystems/magic-nix-cache-action@v14` immediately after install; workflow-level concurrency (`${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress: true`) and job `timeout-minutes: 90`; quality gates are timed `nix flake check --print-build-logs`, timed `nix build .#default --out-link result --print-build-logs`, and smoke tests against `./result/bin/sce` from the already-built default package (no diagnostic `nix flake metadata` step and no separate `nix run` smoke evaluations); pin third-party Actions to explicit version tags, not `@main`.

## Root Biome scoping

- Keep Biome configuration at the repository root when one formatter/linter contract spans multiple JS package areas.
- Scope root `biome.json` explicitly to the approved JS surfaces only; the current approved scope is `npm/**` and the shared `config/lib/**` plugin package root.
- Exclude package-local install artifacts such as `node_modules/**` from root Biome coverage.
- Keep repository-owned OpenCode plugin support code under one shared `config/lib/` Bun/TypeScript package root; package metadata and lockfile ownership live at `config/lib/package.json` and `config/lib/bun.lock`, not under individual plugin subdirectories.
- Provide Biome through the root Nix dev shell so contributors can run `nix develop -c biome ...` without a host-installed binary or package-local setup.
- When exposing JS validation through `nix flake check`, split Bun test, Biome lint/check, and Biome format verification into separately named derivations per target directory so failures stay tool- and surface-specific.
- When `config/lib` tests depend on shared repo fixtures, keep the Nix check source repo-shaped and run tools from `config/lib/` instead of flattening the package root and breaking repo-relative fixture paths.

## Flake app entrypoints

- Expose operational workflows as flake apps so commands are stable and system-mapped across supported `flake-utils` default systems.
- Current repo command contracts:
- For flake app outputs, include `meta.description` so `nix flake check` app validation stays warning-free.
- Do not keep obsolete flake app entrypoints as compatibility shims after an integration runner is removed; remove the app and any dedicated checks together unless a replacement implementation is explicitly approved.
- For Flatpak local/release packaging, see `context/sce/flatpak-distribution-patterns.md` for the full Flatpak-specific pattern catalog.

## Install/distribution rollout

- Treat the approved channel set for the current implementation stage as closed: repo-flake Nix, Cargo, npm, and source-built Flatpak (`dev.crocoder.sce`) only; `Homebrew` remains deferred.
- Standardize new install-facing surfaces on the canonical `sce` name.
- Keep Nix-managed build/release entrypoints as the source of truth for binary downstream install channels.
- Treat repo-root `.version` as the canonical checked-in release version source for GitHub Releases, Cargo publication, and npm publication.
- Keep GitHub Releases as the canonical publication surface for signed release archives, manifest/checksum assets, npm package assets, and approved Flatpak source-manifest and bundle assets.
- Keep crates.io and npm registry publication as separate downstream publish stages that consume already-versioned checked-in package metadata.
- For Flatpak-specific distribution patterns (manifest generation, flake app surface, release assets, host-git bridge), see `context/sce/flatpak-distribution-patterns.md`.

## Dev-shell fallback shims for unavailable nixpkgs tools

- When required CLI tools are not available as direct nixpkgs attrs, use the least-friction dev-shell fallback that keeps commands usable in `nix develop`.
- `shellHook` prints a version banner for `bun`, `pkl`, `tsc`, `typescript-language-server`, and `rustc` so shell state is visible on entry.
- Keep repository-root `.envrc` invalidation targeted to flake- and Cargo-lock inputs (`flake.nix`, `flake.lock`, `cli/Cargo.lock`) so unrelated file edits do not trigger unnecessary direnv/Nix shell reevaluation.

## Pkl renderer layering

- Keep target-agnostic canonical content organized by concern in `config/pkl/base/shared-content-{common,plan,code,commit}.pkl` (manual) and `config/pkl/base/shared-content-automated-{common,plan,code,commit}.pkl` (automated); the aggregation surfaces `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl` import from these grouped modules for downstream renderers.
- Keep cross-target generated-config primitives in focused base modules under `config/pkl/base/` and re-export them through `config/pkl/renderers/common.pkl` when multiple renderers need the same contract.
- Keep the grouped shared-content modules synchronized with canonical authored instruction bodies (currently mirrored from the OpenCode source tree under `config/{opencode_root}` for `agent`, `command`, and `skills`, with frontmatter removed) before regenerating targets.
- When two or more generated agent bodies share baseline doctrine, extract that doctrine into reusable canonical constants in `config/pkl/base/shared-content-common.pkl` and compose via interpolation instead of duplicating prose per agent.
- Implement target-specific formatting in dedicated renderer modules under `config/pkl/renderers/`.
- Keep shared renderer contracts and only truly shared description maps in `config/pkl/renderers/common.pkl`.
- Keep per-target metadata tables in dedicated modules (`opencode-metadata.pkl`, `opencode-automated-metadata.pkl`, `claude-metadata.pkl`), including target-specific skill descriptions, and import them into target renderer modules.
- When OpenCode commands need machine-readable orchestration metadata, add it in `config/pkl/renderers/opencode-content.pkl` as frontmatter fields that are explicitly scoped to the targeted commands, and keep non-target commands unchanged unless the contract expands deliberately.
- Add and run `config/pkl/renderers/metadata-coverage-check.pkl` as a fail-fast metadata completeness guard whenever shared slugs or metadata tables change.
- In renderer modules, produce per-item document objects with explicit `frontmatter`, `body`, and combined `rendered` fields to keep formatting deterministic and easy to map in a later output stage.
- Keep the Markdown renderer contract in `config/pkl/renderers/common.pkl` limited to deterministic `frontmatter + body` assembly without injected generated-file marker text.
- Validate each renderer module directly with `nix develop -c pkl eval <module-path>` before wiring output emission.

## Thin command orchestration

- Keep SCE command bodies thin when phase skills already define detailed contracts.
- For `/next-task`, retain only sequencing and confirmation gates in the command body and delegate phase details to `sce-plan-review`, `sce-task-execution`, and `sce-context-sync`.
- For `/change-to-plan`, retain wrapper-level plan output/handoff obligations in the command body and delegate clarification and plan-shape contracts (including one-task/one-atomic-commit task slicing) to `sce-plan-authoring`.
- For `/commit`, keep the command body thin and profile-aware: manual generated commands retain staging-confirmation and proposal-only gates, while the automated OpenCode command skips staging confirmation, generates exactly one staged commit message, and executes one staged `git commit`; delegate commit-message grammar, the single-message contract, and the staged-plan rule (cite affected plan slug(s) and updated task ID(s) when `context/plans/*.md` is staged, otherwise stop for clarification) to `sce-atomic-commit`.
- Preserve mandatory gates (readiness confirmation, implementation stop, final-task validation trigger) while removing duplicated procedural prose from command text.

## Multi-file generation entrypoint

- Use `config/pkl/generate.pkl` as the single generation module for authored config outputs.
- Use `config/pkl/README.md` as the contributor-facing runbook for prerequisites, ownership boundaries, regeneration steps, and troubleshooting.
- Run multi-file generation with `nix develop -c pkl eval -m . config/pkl/generate.pkl` to emit to repository-root mapped paths.
- Run stale-output detection through the flake app entrypoint `nix run .#pkl-check-generated`; it wraps `nix develop -c ./config/pkl/check-generated.sh`, regenerates into a temporary directory, and fails if generated-owned paths differ from committed outputs.
- Keep generated-output parity anchored to `nix run .#pkl-check-generated` and the root `nix flake check` `pkl-parity` derivation; no dedicated generated-parity workflow is currently checked in.
- Treat `nix run .#pkl-check-generated` and `nix flake check` as the lightweight post-task verification baseline and run both after each completed task.
- For non-destructive verification during development, run `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` and inspect emitted paths under `context/tmp/`.
- Keep `output.files` limited to generated-owned paths only (`config/{opencode_root}/{agent,command,skills,lib,plugins}`, generated `config/{opencode_root}/package.json`, `config/{claude_root}/{agents,commands,skills,hooks,settings.json}`, and `config/{pi_root}/{prompts,skills,extensions}`, where roots map to `.opencode`, `.claude`, and `.pi`).
- For OpenCode pre-execution bash-policy hooks, keep the generated plugin entrypoint thin (`plugins/sce-bash-policy.ts`) and delegate policy evaluation to the Rust `sce policy bash --input normalized --output json` command so OpenCode and Claude share one evaluator.

## Internal subagent parity mapping

- Encode internal-agent parity by target capability, not by forcing unsupported frontmatter keys.
- For OpenCode agents that must be internal, set behavior flags in `config/pkl/renderers/opencode-metadata.pkl` (`agentBehaviorBlocks`) and render those directly into frontmatter.
- For Claude agents, represent equivalent intent using supported metadata and body guidance in `config/pkl/renderers/claude-metadata.pkl` (for example description + preamble blocks for delegated command/task routing).
- Keep parity decisions reproducible by validating generated outputs directly.

## Placeholder CLI scaffolding

- Keep production CLI path ownership centralized in `cli/src/services/default_paths.rs`; new non-test path literals or path-shape definitions should be added there as named accessors/constants instead of becoming new path owners in other modules.
- Keep SCE-owned web URI construction centralized in `services::agent_trace`; production Rust code should use its helpers instead of repeating `https://sce.crocoder.dev`, host-only variants, or derived path prefixes.
- Prefer localized `#[allow(dead_code)]` on intentionally shared path/setup helper items over file-level dead-code suppression so lint scope stays narrow while keeping catalog seams available to tests and future consumers.
- For early CLI foundation tasks, keep the real top-level command catalog/help metadata centralized in one canonical seam (`cli/src/cli_schema.rs` in the current architecture) and let custom top-level help renderers consume that seam instead of maintaining a second parallel command list.
- Keep top-level help intentionally curated: command visibility on `sce`, `sce help`, and `sce --help` may differ from parser availability when a command should remain directly invocable but temporarily hidden from operator-facing help.
- Keep wrapper-only help rows or banner rendering logic outside the clap catalog, but do not duplicate the real command visibility/purpose metadata in those renderers.
- Keep placeholder or deferred state explicit in runtime responses and command-local docs rather than relying on top-level help status badges.
- Parse CLI args with `clap` derive macros, classify top-level failures into stable exit-code classes (`parse`, `validation`, `runtime`, `dependency`), and keep user-facing failures deterministic/actionable.
- Keep command payload structs and execution methods in service-owned `command.rs` modules; keep the static `RuntimeCommand` enum and deterministic command-name catalog in `services/command_registry.rs`; keep clap-to-runtime conversion in `services/parse/command_runtime.rs`; `app.rs` should stay focused on startup lifecycle and thin parse/execute/render orchestration rather than owning command-specific runtime handlers or parse conversion details. Interactive commands such as `sce trace db shell` may perform scoped direct stdio handoff inside the service command, returning an empty payload so the app renderer preserves stdout ownership without duplicating transcript output.
- Emit user-facing CLI diagnostics with stable class-based error IDs (`SCE-ERR-PARSE`, `SCE-ERR-VALIDATION`, `SCE-ERR-RUNTIME`, `SCE-ERR-DEPENDENCY`) using deterministic `Error [<code>]: ...` stderr formatting, and auto-append class-default `Try:` remediation only when the message does not already provide one.
- Keep CLI observability separate from command payloads: emit deterministic lifecycle logs to `stderr` only with stable `event_id` values, and preserve `stdout` for command result payloads.
- For baseline runtime observability controls, resolve logging settings through the shared config resolver first, preserving deterministic precedence (`flags > env > config file > defaults`) and fail-fast validation on invalid env/config inputs.
- For optional observability file sinks, gate enablement behind explicit `SCE_LOG_FILE`, require `SCE_LOG_FILE_MODE` only when file sink is set, default write policy to deterministic `truncate`, and enforce owner-only file permissions (`0600`) on Unix.

- For runtime CLI configuration, keep precedence deterministic and explicit (`flags > env > config file > defaults`) and expose inspect/validate command entrypoints with stable text/JSON outputs.
- For commands that support text/JSON dual output, centralize `--format <text|json>` parsing in one shared contract and pass command-specific `--help` guidance into invalid-value errors instead of duplicating parser logic per command.
- For setup-style command contracts, keep interactive mode as the zero-flag default and enforce mutually-exclusive explicit target flags for non-interactive automation.
- For security-sensitive CLI UX, redact common secret-bearing token/value forms before emitting diagnostics/log lines, including app-level errors, setup git stderr diagnostics, and observability sink output.
- For user-supplied setup repository paths (`sce setup --hooks --repo <path>`), canonicalize/validate the path as an existing directory before git command execution, and run deterministic write-permission probes on setup write targets before staging/swap operations.
- For interactive setup flows, isolate prompt handling behind a service-layer prompter seam so selection mapping and cancellation behavior can be tested without a live TTY.
- When setup or path-catalog modules grow dense, extract focused internal support seams (for example install-flow, prompt-flow, or root-resolution helpers) before adding new behavior so orchestration files stay navigable without changing command contracts.
- Treat setup prompt cancellation/interrupt as a non-destructive exit path with explicit user messaging (no file mutations and no partial side effects).
- For setup install prep, generate compile-time embedded asset manifests from `config/.opencode/**`, `config/.claude/**`, `config/.pi/**`, and `cli/assets/hooks/**` in `cli/build.rs`, keep relative paths normalized to forward-slash form, and expose target-scoped iterators/lookups from the setup service layer for installer wiring.
- For CLI database migration prep, keep SQL files under immediate `cli/migrations/<db-name>/` directories named `NNN_description.sql`; `cli/build.rs` discovers those files at compile time, sorts by the numeric prefix before `_`, and writes deterministic `cli/src/generated_migrations.rs` constants with `include_str!` references for service `DbSpec` consumers.
- For setup install execution, write selected embedded assets into a per-target staging directory first, then remove the existing target and swap staged content into place; on swap failure, clean temporary staging paths and return deterministic recovery guidance (recover from version control). No backup artifacts are created.
- For required-hook setup execution, resolve repository root and effective hooks directory from git (`rev-parse --show-toplevel`, `rev-parse --git-path hooks`), then apply deterministic per-hook outcomes (`Installed`, `Updated`, `Skipped`) with staged writes, executable-bit enforcement, and remove-and-replace behavior that removes existing hooks before swapping staged content.
- For hook setup CLI UX, allow `--hooks` as both hooks-only and composable target+hooks execution (optional `--repo <path>`), enforce deterministic option compatibility (`--repo` requires `--hooks`; target flags stay mutually exclusive), and emit stable section-ordered setup/hook status lines for automation-friendly logs.
- For setup command messaging, emit deterministic completion output that includes selected target(s) and per-target install counts.
- Keep module seams for future domains present and compile-safe even when behavior is deferred.
- Keep dependency additions explicit and minimal in `cli/Cargo.toml`, and anchor dependency intent in domain-owned service types/tests rather than a separate compile-time dependency snapshot module.
- Route local Turso access through service adapters so command handlers do not expose low-level `turso` API details. New Turso-backed services should build on `cli/src/services/db/mod.rs` (`DbSpec` + `TursoDb<M>`, or `EncryptedTursoDb<M>` when at-rest encryption is required) for runtime, connection, per-database `__sce_migrations` tracking, and migration infrastructure, then expose domain-specific methods from their own service modules.
- For current local DB flows, route initialization through the dedicated adapter (`cli/src/services/local_db/mod.rs`) and invoke it from approved orchestration surfaces such as setup or doctor rather than exposing a partial user command before its contract is approved.
- For Turso-backed services with setup/doctor ownership, add service-owned lifecycle providers that reuse shared DB path-health and parent-bootstrap helpers, then register them through `lifecycle_providers()` instead of adding command-local database checks.
- For transient local IO/database hotspots, apply bounded resilience wrappers with explicit retry count, timeout, and capped backoff (`cli/src/services/resilience.rs`) and surface terminal failures with deterministic `Try:` remediation guidance. Use async `run_with_retry` for async operations and sync `run_with_retry_sync` for pure blocking contexts where a Tokio sleep/timeout future cannot be awaited. For Turso database constructors/openers, wrap only the local open/connect operation in retry; keep migration execution outside retry because schema changes must not be replayed. Use `TursoDb<M>::new()` for setup/lifecycle-owned schema initialization and `TursoDb<M>::open_without_migrations()` only for hot runtime paths that verify required schema separately before query/write work. For `TursoDb<M>` and `EncryptedTursoDb<M>` operation retry, convert params to owned cloneable Turso params before retrying `execute()`/`query()`, retry `query_map()` query plus row-fetch failures, and keep caller row-mapping outside retry. Retry policies for both connection-open and query operations can now be configured per database via `policies.database_retry` in `sce/config.json`, parsed and resolved in `cli/src/services/config/mod.rs`, with fallback to hardcoded defaults when the config key is absent.
- For SCE operator-health commands, prefer deterministic local diagnostics over implicit pass/fail behavior: report the inspected environment scope, stable problem categories, severity/fixability classes, actionable remediation text, and any path/location facts needed to repair the issue; when repair mode exists, keep outcome vocabulary deterministic and idempotent (`cli/src/services/doctor/mod.rs`, with focused diagnosis/render/fix helpers under `cli/src/services/doctor/`).
- For service-owned operator health, keep command modules as thin aggregators over `ServiceLifecycle` providers once a lifecycle slice is wired: providers own diagnosis/fix problem production through narrow capability accessors, while command-specific report builders preserve existing output facts and rendering contracts.
- Keep static lifecycle provider-list construction centralized in the lifecycle service layer so doctor/setup choose provider inclusion without maintaining parallel concrete provider lists.
- Keep `ServiceLifecycle` trait signatures lifecycle-owned and capability-narrow; adapt lifecycle health/fix/setup records into doctor/setup-owned output records at command orchestration boundaries rather than making provider contracts depend on command modules or the full production context type.
- For repo-scoped hook-health diagnostics, resolve effective hooks location from git truth, distinguish git-unavailable vs outside-repo vs bare-repo failure modes explicitly, and compare required hook payload bytes against the canonical embedded hook assets so stale SCE-managed hook content is reported deterministically (`cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/setup/mod.rs`).
- For cross-service CLI dependencies exposed through the borrowed `AppContext` view, prefer shared capability/accessor traits over one-off per-service abstractions; keep production wrappers thin over `std::fs` and `git` process execution until call-site migration tasks approve deeper service refactors, and keep command execution generic over the narrow accessors each command needs where practical.
- For future CLI domains, define trait-first service contracts with request/plan models in `cli/src/services/*` and keep placeholder implementations explicitly non-runnable until production behavior is approved.
- Model deferred integration boundaries with concrete event/capability data structures (for example hook-runtime attribution snapshots/policies and cloud-sync checkpoints) so later tasks can implement behavior without reshaping public seams.
- For the current local-hook baseline, keep `pre-commit` and `post-rewrite` as deterministic no-op entrypoints; keep `post-commit` as the active bounded recent-diff-trace intersection entrypoint with validated `--remote-url` plumbed through Agent Trace flow and any direct diagnostics printed to stderr; keep `diff-trace` as an explicit STDIN intake path with deterministic required-field validation for `sessionID`, `diff`, `time`, `tool_name`, optional `model_id` (absent/`null` → `None`), and `tool_version` (present and either `null` or non-empty string), same-tool-idempotent stored `diff_traces.session_id` prefixing (`oc_` for OpenCode, `cc_` for Claude), non-lossy AgentTraceDb `time_ms` conversion, and AgentTraceDb insertion whose failure is logged and reflected in deterministic success text without creating a `context/tmp` artifact fallback; keep `conversation-trace` as the active message/part intake path. `session-model` is no longer a supported hook intake path.
- For `diff-trace` hook intake, keep producer-facing failure behavior fail-open: STDIN read, parse/validation, and setup/persistence failures are logged with `sce.hooks.diff_trace.error` and converted into command success; preserve the existing valid-payload success text and the AgentTraceDb write-warning success path.
- For `conversation-trace` hook intake, keep producer-facing failure behavior fail-open: STDIN read, top-level parse/validation, unsupported raw Claude hook events, and AgentTraceDb setup/persistence failures are logged with `sce.hooks.conversation_trace.error` and converted into command success; preserve valid-payload mixed-batch accounting, skipped-item logging, and batch-insert warning behavior.
- For diff-trace attribution persistence, persist direct payload `model_id` and `tool_version` values as-is; missing attribution fields are stored as `NULL` in `diff_traces`. The former `session_models` fallback lookup was removed.
- For commit-msg co-author policy seams, gate canonical trailer insertion on runtime controls (`SCE_DISABLED` plus the shared attribution-hooks enablement gate) plus the staged-diff AI-overlap evidence gate (`StagedDiffAiOverlapResult::Overlap` maps to `ai_contribution_present = true`; `NoOverlap` and `Error` both map to `false`), and enforce idempotent dedupe so allowed cases end with exactly one `Co-authored-by: SCE <sce@crocoder.dev>` trailer.
- For local hook attribution flows, resolve the top-level enablement gate through the shared config precedence model (`SCE_ATTRIBUTION_HOOKS_DISABLED` opt-out env over `policies.attribution_hooks.enabled`, default `true`) so commit-msg attribution is enabled by default while explicit config `enabled = false` and truthy env opt-out still suppress it without adding hook-specific config parsing.
- Do not assume conversation-trace retry/backfill/artifact persistence, retry replay, remap ingestion, or rewrite trace transformation are active in the current local-hook runtime; those paths are removed from or deferred beyond the current baseline.
- For the current local DB baseline, resolve one deterministic per-user persistent DB target (Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/local.db`; platform-equivalent state roots elsewhere), keep the path neutral rather than Agent Trace-branded, create parent directories before first use, and route initialization through `LocalDb::new()`. As database services split, keep path/migration ownership in each `DbSpec`: `LocalDbSpec` owns the neutral local DB path with zero migrations, `AuthDbSpec` owns encrypted `<state_root>/sce/auth.db` plus ordered auth migrations, `AgentTraceDbSpec` owns `<state_root>/sce/agent-trace.db` plus ordered Agent Trace migrations for `diff_traces`, `post_commit_patch_intersections`, `agent_traces`, `messages`, and `parts` plus supporting indexes and triggers (migration `015_create_session_models` was removed from fresh schema; current schema uses `015_add_diff_traces_payload_type`), and shared Turso mechanics plus migration metadata stay in `TursoDb<M>` / `EncryptedTursoDb<M>`.
- For hosted event intake seams, verify provider signatures before payload parsing (GitHub `sha256=<hex>` HMAC over body, GitLab token-equality secret check), resolve old/new heads from provider payload fields, and derive deterministic reconciliation run idempotency keys from provider+event+repo+head tuple material.
- For hosted rewrite mapping seams, resolve candidates deterministically in strict precedence order (patch-id exact, then range-diff score, then fuzzy score), classify top-score ties as `ambiguous`, enforce low-confidence unresolved behavior below `0.60`, and preserve stable outcome ordering via canonical candidate SHA sorting.
- For hosted reconciliation observability, publish run-level mapped/unmapped counts, confidence histogram buckets, runtime timing, and normalized error-class labels so retry/quality drift can be monitored without requiring a full dashboard surface.
- Keep crate-local onboarding docs in `cli/README.md` and sanity-check command examples against actual `sce` output whenever command messaging changes.
- Keep Rust verification in flake checks under stable named derivations re-exported by the root flake: `checks.<system>.cli-tests`, `checks.<system>.cli-clippy`, `checks.<system>.cli-fmt`, and `checks.<system>.workflow-actionlint`.
- Keep cheap flake-check sources as narrow as their behavior allows: formatting checks should not depend on package-only generated assets, and parity/static checks should copy only the authoring inputs and committed outputs they inspect.
- Keep Rust package/check sources as narrow as behavior allows: package builds should include the Cargo tree plus required embedded config/schema assets, not unrelated config authoring or plugin sources that are not read by `cli/build.rs`.
- In `flake.nix`, select the Rust toolchain via an explicit Rust overlay (`rust-overlay`) and thread that toolchain through Crane package/check derivations so CLI builds and checks do not rely on implicit nixpkgs Rust defaults.
- For installable CLI release surfaces in the root flake, expose an explicit named package plus default alias (`packages.sce` and `packages.default = packages.sce`) and pair it with a runnable app output (`apps.sce`) that points to the packaged binary path.
- For root-flake CLI release metadata, source the package/check version from repo-root `.version` and trim it at eval time so packaged outputs stay aligned without hardcoded semver strings in `flake.nix`.
- For Cargo CLI distribution, keep crate metadata publication-ready, document the supported Cargo install paths in `cli/README.md` (`cargo install shared-context-engineering --locked`, git install with `--locked`, and local `cargo install --path cli --locked`), and verify at least the repo-local build/check path through the Nix-managed validation baseline.

## Unit testing in Nix sandbox

- Unit tests must not depend on filesystem directories, temporary directories, or databases that could fail in Nix sandbox environments.
- Tests that require filesystem I/O, git repository operations, or database connections belong in integration tests, not unit tests.
- When a unit test needs filesystem, git, or database behavior that is not safe for `nix flake check`, delete it from the unit-test suite and reintroduce that coverage later as an integration test instead of keeping ignored tests in-tree.
- Pure unit tests should test in-memory logic, parsing, validation, and data transformations without external dependencies.
- The `TestTempDir` helper and similar filesystem fixtures should only be used in integration tests, not unit tests.
- In-memory database tests (e.g., `LocalDatabaseTarget::InMemory`) are acceptable for unit tests since they don't touch the filesystem.
- When adding new tests, prefer mocking/faking external dependencies over creating real filesystem or database state.
