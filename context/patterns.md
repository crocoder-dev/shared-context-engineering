# Patterns

## Config generation tooling

- Use the Nix dev shell as the canonical toolchain entrypoint for generation work.
- `flake.nix` includes `pkl` so contributors can run validation commands with `nix develop -c ...` without host-level installs.

## Verification guidance

- Prefer `nix flake check` for repository-level verification/check flows in contributor guidance.
- Keep direct Cargo verification commands as secondary targeted-debugging tools rather than the default repo-validation path.
- Keep `cargo fmt` available for explicit autofix formatting flows; do not present it as the preferred verification command.

## Root Biome scoping

- Keep Biome configuration at the repository root when one formatter/linter contract spans multiple JS package areas.
- Scope root `biome.json` explicitly to the approved JS surfaces only; the current approved scope is `npm/**` and `config/lib/bash-policy-plugin/**`.
- Exclude package-local install artifacts such as `node_modules/**` from root Biome coverage.
- Provide Biome through the root Nix dev shell so contributors can run `nix develop -c biome ...` without a host-installed binary or package-local setup.
- When exposing JS validation through `nix flake check`, split Bun test, Biome lint/check, and Biome format verification into separately named derivations per target directory so failures stay tool- and surface-specific.

## Flake app entrypoints

- Expose operational workflows as flake apps so commands are stable and system-mapped across supported `flake-utils` default systems.
- Current repo command contracts:
- For flake app outputs, include `meta.description` so `nix flake check` app validation stays warning-free.
- When install/integration coverage is heavier than the default repository validation baseline, expose it as an explicit opt-in flake app instead of adding it to `checks.<system>` prematurely.

## First-wave install/distribution rollout

- Treat the approved first-wave channel set for the current implementation stage as closed: repo-flake Nix, Cargo, and npm only; `Homebrew` remains deferred until a later plan stage restores it explicitly.
- Standardize new install-facing surfaces on the canonical `sce` name; remove or explicitly map legacy `sce-editor` references when they are touched.
- Keep Nix-managed build/release entrypoints as the source of truth for downstream install channels.
- Treat repo-root `.version` as the canonical checked-in release version source for GitHub Releases, Cargo publication, and npm publication.
- Expose shared CLI release packaging through root-flake apps so local verification and GitHub release automation consume the same commands (`nix run .#release-artifacts`, `nix run .#release-manifest`, `nix run .#release-npm-package`).
- Keep GitHub Releases as the canonical publication surface for signed release archives and manifest/checksum assets.
- Keep crates.io and npm registry publication as separate downstream publish stages that consume already-versioned checked-in package metadata rather than inventing workflow-side version bumps.
- Keep `.github/workflows/publish-crates.yml` scoped to crates.io publication only: it should validate `.version`, `cli/Cargo.toml`, and the release tag before running `cargo publish`, and real publication must require an explicit `CARGO_REGISTRY_TOKEN` secret while manual dispatch can stay on a dry-run path.
- Keep `.github/workflows/publish-npm.yml` scoped to npm publication only: it should validate `.version`, `npm/package.json`, and the release tag before publish, download the canonical `sce-v<version>-npm.tgz` GitHub release asset instead of rebuilding or mutating package metadata, and require an explicit `NPM_TOKEN` secret only for real publication while manual dispatch can stay on a dry-run path.
- Keep CLI release workflows split by platform in separate workflow files, with one thin orchestrator workflow calling those reusable per-platform jobs rather than mixing `sce` release logic into unrelated release pipelines.
- For release-manifest signing, keep the private key outside the repository and feed it to `release-manifest` through `SCE_RELEASE_MANIFEST_SIGNING_KEY` or an explicit key file path; publish only the detached manifest signature artifact.
- For the npm channel, keep the package thin: download the merged release manifest plus detached signature, verify the manifest with the bundled npm public key before trusting any manifest-provided checksum, then download the already-built native release archive for the matching supported target and verify its published SHA-256 without adding a second build pipeline inside npm packaging.
- For published Cargo crate builds, prepare any generated setup/config payload required at compile time into an ephemeral crate-local mirror (`cli/assets/generated/**`) immediately before build/package/publish work, while keeping `config/` as the only committed source of truth.
- When Cargo publish would otherwise require `--allow-dirty` because of ephemeral generated assets, publish from a temporary clean repository copy instead of weakening the workflow contract.

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
- Keep CI parity enforcement aligned with local workflow by running the same command in `.github/workflows/pkl-generated-parity.yml` for pushes to `main` and pull requests targeting `main`.
- Treat `nix run .#pkl-check-generated` and `nix flake check` as the lightweight post-task verification baseline and run both after each completed task.
- For non-destructive verification during development, run `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` and inspect emitted paths under `context/tmp/`.
- Keep `output.files` limited to generated-owned paths only (`config/{opencode_root}/{agent,command,skills,lib,plugins}`, generated `config/{opencode_root}/package.json`, and `config/{claude_root}/{agents,commands,skills,lib,hooks,settings.json}`, where roots map to `.opencode` and `.claude`).
- For OpenCode pre-execution tool policy hooks, keep the plugin entrypoint thin (`plugins/*.ts`) and move normalization, config loading, and policy matching logic into `lib/*.ts` so manual and automated profiles regenerate identical enforcement behavior from one canonical TypeScript source.

## Internal subagent parity mapping

- Encode internal-agent parity by target capability, not by forcing unsupported frontmatter keys.
- For OpenCode agents that must be internal, set behavior flags in `config/pkl/renderers/opencode-metadata.pkl` (`agentBehaviorBlocks`) and render those directly into frontmatter.
- For Claude agents, represent equivalent intent using supported metadata and body guidance in `config/pkl/renderers/claude-metadata.pkl` (for example description + preamble blocks for delegated command/task routing).
- Keep parity decisions reproducible by validating generated outputs directly.

## Placeholder CLI scaffolding

- Keep production CLI path ownership centralized in `cli/src/services/default_paths.rs`; new non-test path literals or path-shape definitions should be added there as named accessors/constants instead of becoming new path owners in other modules.
- Prefer localized `#[allow(dead_code)]` on intentionally shared path/setup helper items over file-level dead-code suppression so lint scope stays narrow while keeping catalog seams available to tests and future consumers.
- For early CLI foundation tasks, keep the real top-level command catalog/help metadata centralized in one canonical seam (`cli/src/cli_schema.rs` in the current architecture) and let custom top-level help renderers consume that seam instead of maintaining a second parallel command list.
- Keep top-level help intentionally curated: command visibility on `sce`, `sce help`, and `sce --help` may differ from parser availability when a command should remain directly invocable but temporarily hidden from operator-facing help.
- Keep wrapper-only help rows or banner rendering logic outside the clap catalog, but do not duplicate the real command visibility/purpose metadata in those renderers.
- Keep placeholder or deferred state explicit in runtime responses and command-local docs rather than relying on top-level help status badges.
- Parse CLI args with `clap` derive macros, classify top-level failures into stable exit-code classes (`parse`, `validation`, `runtime`, `dependency`), and keep user-facing failures deterministic/actionable.
- Emit user-facing CLI diagnostics with stable class-based error IDs (`SCE-ERR-PARSE`, `SCE-ERR-VALIDATION`, `SCE-ERR-RUNTIME`, `SCE-ERR-DEPENDENCY`) using deterministic `Error [<code>]: ...` stderr formatting, and auto-append class-default `Try:` remediation only when the message does not already provide one.
- Keep CLI observability separate from command payloads: emit deterministic lifecycle logs to `stderr` only with stable `event_id` values, and preserve `stdout` for command result payloads.
- For baseline runtime observability controls, resolve logging and OTEL settings through the shared config resolver first, preserving deterministic precedence (`flags > env > config file > defaults`) and fail-fast validation on invalid env/config inputs.
- For optional observability file sinks, gate enablement behind explicit `SCE_LOG_FILE`, require `SCE_LOG_FILE_MODE` only when file sink is set, default write policy to deterministic `truncate`, and enforce owner-only file permissions (`0600`) on Unix.
- For OTEL baseline wiring, keep exporter bootstrap opt-in (`SCE_OTEL_ENABLED`), keep exporter mode env-addressable (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`), and validate invalid endpoint/protocol values as invocation validation failures before command dispatch.
- Mirror lifecycle logger events into tracing events and attach OTEL subscriber context only around command execution so stdout payload contracts remain unchanged.
- For runtime CLI configuration, keep precedence deterministic and explicit (`flags > env > config file > defaults`) and expose inspect/validate command entrypoints with stable text/JSON outputs.
- For commands that support text/JSON dual output, centralize `--format <text|json>` parsing in one shared contract and pass command-specific `--help` guidance into invalid-value errors instead of duplicating parser logic per command.
- For setup-style command contracts, keep interactive mode as the zero-flag default and enforce mutually-exclusive explicit target flags for non-interactive automation.
- For security-sensitive CLI UX, redact common secret-bearing token/value forms before emitting diagnostics/log lines, including app-level errors, setup git stderr diagnostics, and observability sink output.
- For user-supplied setup repository paths (`sce setup --hooks --repo <path>`), canonicalize/validate the path as an existing directory before git command execution, and run deterministic write-permission probes on setup write targets before staging/swap operations.
- For interactive setup flows, isolate prompt handling behind a service-layer prompter seam so selection mapping and cancellation behavior can be tested without a live TTY.
- When setup or path-catalog modules grow dense, extract focused internal support seams (for example install-flow, prompt-flow, or root-resolution helpers) before adding new behavior so orchestration files stay navigable without changing command contracts.
- Treat setup prompt cancellation/interrupt as a non-destructive exit path with explicit user messaging (no file mutations and no partial side effects).
- For setup install prep, generate compile-time embedded asset manifests from `config/.opencode/**`, `config/.claude/**`, and `cli/assets/hooks/**` in `cli/build.rs`, keep relative paths normalized to forward-slash form, and expose target-scoped iterators/lookups from the setup service layer for installer wiring.
- For setup install execution, write selected embedded assets into a per-target staging directory first, then remove the existing target and swap staged content into place; on swap failure, clean temporary staging paths and return deterministic recovery guidance (recover from version control). No backup artifacts are created.
- For required-hook setup execution, resolve repository root and effective hooks directory from git (`rev-parse --show-toplevel`, `rev-parse --git-path hooks`), then apply deterministic per-hook outcomes (`Installed`, `Updated`, `Skipped`) with staged writes, executable-bit enforcement, and remove-and-replace behavior that removes existing hooks before swapping staged content.
- For hook setup CLI UX, allow `--hooks` as both hooks-only and composable target+hooks execution (optional `--repo <path>`), enforce deterministic option compatibility (`--repo` requires `--hooks`; target flags stay mutually exclusive), and emit stable section-ordered setup/hook status lines for automation-friendly logs.
- For setup command messaging, emit deterministic completion output that includes selected target(s) and per-target install counts.
- Keep module seams for future domains present and compile-safe even when behavior is deferred.
- Keep dependency additions explicit and minimal in `cli/Cargo.toml`, and anchor dependency intent in domain-owned service types/tests rather than a separate compile-time dependency snapshot module.
- Route local Turso access through a dedicated adapter module (`cli/src/services/local_db.rs`) so command handlers do not expose low-level `turso` API details.
- For current local DB flows, route initialization through the dedicated adapter (`cli/src/services/local_db.rs`) and invoke it from approved orchestration surfaces such as setup or doctor rather than exposing a partial user command before its contract is approved.
- For transient local IO/database hotspots, apply bounded resilience wrappers with explicit retry count, timeout, and capped backoff (`cli/src/services/resilience.rs`) and surface terminal failures with deterministic `Try:` remediation guidance.
- For SCE operator-health commands, prefer deterministic local diagnostics over implicit pass/fail behavior: report the inspected environment scope, stable problem categories, severity/fixability classes, actionable remediation text, and any path/location facts needed to repair the issue; when repair mode exists, keep outcome vocabulary deterministic and idempotent (`cli/src/services/doctor/mod.rs`, with focused diagnosis/render/fix helpers under `cli/src/services/doctor/`).
- For repo-scoped hook-health diagnostics, resolve effective hooks location from git truth, distinguish git-unavailable vs outside-repo vs bare-repo failure modes explicitly, and compare required hook payload bytes against the canonical embedded hook assets so stale SCE-managed hook content is reported deterministically (`cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/setup.rs`).
- For future CLI domains, define trait-first service contracts with request/plan models in `cli/src/services/*` and keep placeholder implementations explicitly non-runnable until production behavior is approved.
- Model deferred integration boundaries with concrete event/capability data structures (for example hook-runtime attribution snapshots/policies and cloud-sync checkpoints) so later tasks can implement behavior without reshaping public seams.
- For the current local-hook baseline, keep `pre-commit`, `post-commit`, and `post-rewrite` as deterministic no-op entrypoints; keep `diff-trace` as an explicit STDIN intake path with deterministic required-field validation and collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` persistence using atomic create-new retry semantics.
- For commit-msg co-author policy seams, gate canonical trailer insertion on runtime controls (`SCE_DISABLED` plus the shared attribution-hooks enablement gate), and enforce idempotent dedupe so allowed cases end with exactly one `Co-authored-by: SCE <sce@crocoder.dev>` trailer.
- For local hook attribution flows, resolve the top-level enablement gate through the shared config precedence model (`SCE_ATTRIBUTION_HOOKS_ENABLED` over `policies.attribution_hooks.enabled`, default `false`) so commit-msg attribution stays disabled by default without adding hook-specific config parsing.
- Do not assume post-commit persistence, retry replay, remap ingestion, or rewrite trace transformation are active in the current local-hook runtime; those paths are removed from the current baseline.
- For the current local DB baseline, resolve one deterministic per-user persistent DB target (Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/local.db`; platform-equivalent state roots elsewhere), keep the path neutral rather than Agent Trace-branded, create parent directories before first use, reset only the known retired placeholder Agent Trace table shape as disposable, fail fast on unknown incompatible schema shapes, run embedded SQL migrations on initialization (`LocalDb::new`) so required local tables exist idempotently, and use `LocalDb::insert_agent_trace(&AgentTrace)` rather than ad hoc SQL for normalized Agent Trace writes.
- For hosted event intake seams, verify provider signatures before payload parsing (GitHub `sha256=<hex>` HMAC over body, GitLab token-equality secret check), resolve old/new heads from provider payload fields, and derive deterministic reconciliation run idempotency keys from provider+event+repo+head tuple material.
- For hosted rewrite mapping seams, resolve candidates deterministically in strict precedence order (patch-id exact, then range-diff score, then fuzzy score), classify top-score ties as `ambiguous`, enforce low-confidence unresolved behavior below `0.60`, and preserve stable outcome ordering via canonical candidate SHA sorting.
- For hosted reconciliation observability, publish run-level mapped/unmapped counts, confidence histogram buckets, runtime timing, and normalized error-class labels so retry/quality drift can be monitored without requiring a full dashboard surface.
- Keep crate-local onboarding docs in `cli/README.md` and sanity-check command examples against actual `sce` output whenever command messaging changes.
- Keep Rust verification in flake checks under stable named derivations re-exported by the root flake: `checks.<system>.cli-tests`, `checks.<system>.cli-clippy`, `checks.<system>.cli-fmt`, `checks.<system>.integrations-install-tests`, `checks.<system>.integrations-install-clippy`, and `checks.<system>.integrations-install-fmt`.
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
