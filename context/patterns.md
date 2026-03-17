# Patterns

## Config generation tooling

- Use the Nix dev shell as the canonical toolchain entrypoint for generation work.
- `flake.nix` includes `pkl` so contributors can run validation commands with `nix develop -c ...` without host-level installs.

## Flake app entrypoints

- Expose operational workflows as flake apps so commands are stable and system-mapped across supported `flake-utils` default systems.
- Current repo command contracts:
  - `nix run .#sync-opencode-config` is the canonical entrypoint for staged regeneration/replacement of `config/` and replacement of repository-root `.opencode/` from regenerated `config/.opencode/`.
  - `nix run .#token-count-workflows` is the canonical root entrypoint for static workflow token counting (wrapping `bun run token-count-workflows` from `evals/` through `nix develop`).
- For flake app outputs, include `meta.description` so `nix flake check` app validation stays warning-free.
- For destructive config replacement flows, regenerate into a temporary staged `config/` first, validate required generated directories exist, and only then swap live `config/`.
- For destructive root `.opencode/` replacement flows, keep exclusions explicit (for example `node_modules`), use backup-and-restore around swap, and run a source/target tree parity check with the same exclusions.
- Keep command help available via `nix run .#sync-opencode-config -- --help` to provide deterministic usage checks during incremental implementation.

## Dev-shell fallback shims for unavailable nixpkgs tools

- When required CLI tools are not available as direct nixpkgs attrs, use the least-friction dev-shell fallback that keeps commands usable in `nix develop`.
- `shellHook` prints a version banner for `bun`, `pkl`, `tsc`, `typescript-language-server`, and `rustc` so shell state is visible on entry.

## Pkl renderer layering

- Keep target-agnostic canonical content in `config/pkl/base/shared-content.pkl`.
- Keep `config/pkl/base/shared-content.pkl` synchronized with the canonical authored instruction bodies (currently mirrored from the OpenCode source tree under `config/{opencode_root}` for `agent`, `command`, and `skills`, with frontmatter removed) before regenerating targets.
- When two or more generated agent bodies share baseline doctrine, extract that doctrine into reusable canonical constants in `config/pkl/base/shared-content.pkl` and compose via interpolation instead of duplicating prose per agent.
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
- For `/commit`, retain staging-confirmation and proposal-only gates in the command body and delegate commit grammar plus atomic split guidance to `sce-atomic-commit`.
- Preserve mandatory gates (readiness confirmation, implementation stop, final-task validation trigger) while removing duplicated procedural prose from command text.

## Multi-file generation entrypoint

- Use `config/pkl/generate.pkl` as the single generation module for authored config outputs.
- Use `config/pkl/README.md` as the contributor-facing runbook for prerequisites, ownership boundaries, regeneration steps, and troubleshooting.
- Run multi-file generation with `nix develop -c pkl eval -m . config/pkl/generate.pkl` to emit to repository-root mapped paths.
- Run stale-output detection through the flake app entrypoint `nix run .#pkl-check-generated`; it wraps `nix develop -c ./config/pkl/check-generated.sh`, regenerates into a temporary directory, and fails if generated-owned paths differ from committed outputs.
- Keep CI parity enforcement aligned with local workflow by running the same command in `.github/workflows/pkl-generated-parity.yml` for pushes to `main` and pull requests targeting `main`.
- Keep token-count CI aligned with the flake app contract by running `nix run .#token-count-workflows` in `.github/workflows/workflow-token-count.yml` on pushes/pull requests targeting `main`, and upload artifacts from `context/tmp/token-footprint/`.
- Treat `nix run .#pkl-check-generated` and `nix flake check` as the lightweight post-task verification baseline and run both after each completed task.
- Do not run `evals/` test suites autonomously during plan-task execution; run them only when the user explicitly requests eval coverage.
- For non-destructive verification during development, run `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` and inspect emitted paths under `context/tmp/`.
- Keep `output.files` limited to generated-owned paths only (`config/{opencode_root}/{agent,command,skills,lib,plugins}`, generated `config/{opencode_root}/package.json`, and `config/{claude_root}/{agents,commands,skills,lib}` where roots map to `.opencode` and `.claude`).
- Keep the shared drift library source marker-free in `config/.opencode/lib/drift-collectors.js` so generated `lib/drift-collectors.js` outputs stay behavior-only and deterministic across both targets.
- For OpenCode pre-execution tool policy hooks, keep the plugin entrypoint thin (`plugins/*.js`) and move normalization, config loading, and policy matching logic into `lib/*.js` so manual and automated profiles regenerate identical enforcement behavior from one canonical source.
- For Claude pre-execution bash-policy enforcement, keep `.claude/settings.json` limited to hook registration and path wiring, keep the command hook thin under `.claude/hooks/`, and reuse the same shared `lib/bash-policy-runtime.js` + `lib/bash-policy-presets.json` logic so allow/block decisions and denial text stay parity-aligned with OpenCode.

## Internal subagent parity mapping

- Encode internal-agent parity by target capability, not by forcing unsupported frontmatter keys.
- For OpenCode agents that must be internal, set behavior flags in `config/pkl/renderers/opencode-metadata.pkl` (`agentBehaviorBlocks`) and render those directly into frontmatter.
- For Claude agents, represent equivalent intent using supported metadata and body guidance in `config/pkl/renderers/claude-metadata.pkl` (for example description + preamble blocks for delegated command/task routing).
- Keep parity decisions reproducible by validating generated outputs directly (for Shared Context Drift: `config/.opencode/agent/Shared Context Drift.md` and `config/.claude/agents/shared-context-drift.md`).

## Placeholder CLI scaffolding

- For early CLI foundation tasks, keep implemented behavior and planned behavior explicitly separated in a single command contract table.
- Mark placeholder commands in help output and runtime responses so scaffolding cannot be confused with production capability.
- Parse CLI args with `clap` derive macros, classify top-level failures into stable exit-code classes (`parse`, `validation`, `runtime`, `dependency`), and keep user-facing failures deterministic/actionable.
- Emit user-facing CLI diagnostics with stable class-based error IDs (`SCE-ERR-PARSE`, `SCE-ERR-VALIDATION`, `SCE-ERR-RUNTIME`, `SCE-ERR-DEPENDENCY`) using deterministic `Error [<code>]: ...` stderr formatting, and auto-append class-default `Try:` remediation only when the message does not already provide one.
- Keep CLI observability separate from command payloads: emit deterministic lifecycle logs to `stderr` only with stable `event_id` values, and preserve `stdout` for command result payloads.
- For baseline runtime observability controls, use deterministic env switches (`SCE_LOG_LEVEL`, `SCE_LOG_FORMAT`) with strict allowed values and fail-fast validation on invalid inputs.
- For optional observability file sinks, gate enablement behind explicit `SCE_LOG_FILE`, require `SCE_LOG_FILE_MODE` only when file sink is set, default write policy to deterministic `truncate`, and enforce owner-only file permissions (`0600`) on Unix.
- For OTEL baseline wiring, keep exporter bootstrap opt-in (`SCE_OTEL_ENABLED`), keep exporter mode env-addressable (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`), and validate invalid endpoint/protocol values as invocation validation failures before command dispatch.
- Mirror lifecycle logger events into tracing events and attach OTEL subscriber context only around command execution so stdout payload contracts remain unchanged.
- For runtime CLI configuration, keep precedence deterministic and explicit (`flags > env > config file > defaults`) and expose inspect/validate command entrypoints with stable text/JSON outputs.
- For commands that support text/JSON dual output, centralize `--format <text|json>` parsing in one shared contract and pass command-specific `--help` guidance into invalid-value errors instead of duplicating parser logic per command.
- For setup-style command contracts, keep interactive mode as the zero-flag default and enforce mutually-exclusive explicit target flags for non-interactive automation.
- For security-sensitive CLI UX, redact common secret-bearing token/value forms before emitting diagnostics/log lines, including app-level errors, setup git stderr diagnostics, and observability sink output.
- For user-supplied setup repository paths (`sce setup --hooks --repo <path>`), canonicalize/validate the path as an existing directory before git command execution, and run deterministic write-permission probes on setup write targets before staging/swap operations.
- For interactive setup flows, isolate prompt handling behind a service-layer prompter seam so selection mapping and cancellation behavior can be tested without a live TTY.
- Treat setup prompt cancellation/interrupt as a non-destructive exit path with explicit user messaging (no file mutations and no partial side effects).
- For setup install prep, generate compile-time embedded asset manifests from `config/.opencode/**`, `config/.claude/**`, and `cli/assets/hooks/**` in `cli/build.rs`, keep relative paths normalized to forward-slash form, and expose target-scoped iterators/lookups from the setup service layer for installer wiring.
- For setup install execution, write selected embedded assets into a per-target staging directory first, then swap into repository-root `.opencode/`/`.claude/` with backup-and-replace semantics; when swap fails after backup creation, restore the original target path from backup and clean staging directories.
- For required-hook setup execution, resolve repository root and effective hooks directory from git (`rev-parse --show-toplevel`, `rev-parse --git-path hooks`), then apply deterministic per-hook outcomes (`Installed`, `Updated`, `Skipped`) with staged writes, executable-bit enforcement, and backup-and-restore rollback on swap failures.
- For hook setup CLI UX, allow `--hooks` as both hooks-only and composable target+hooks execution (optional `--repo <path>`), enforce deterministic option compatibility (`--repo` requires `--hooks`; target flags stay mutually exclusive), and emit stable section-ordered setup/hook status lines for automation-friendly logs.
- For setup command messaging, emit deterministic completion output that includes selected target(s), per-target install counts, and whether backup was created.
- Keep module seams for future domains present and compile-safe even when behavior is deferred.
- Keep dependency additions explicit and minimal in `cli/Cargo.toml`, and anchor dependency intent in domain-owned service types/tests rather than a separate compile-time dependency snapshot module.
- Route local Turso access through a dedicated adapter module (`cli/src/services/local_db.rs`) so command handlers do not expose low-level `turso` API details.
- For placeholder commands that need real infrastructure checks, use a lazily initialized shared tokio current-thread runtime wrapper in the service layer (`cli/src/services/sync.rs`) and keep user-facing output explicit about remaining placeholder scope.
- For transient local IO/database hotspots, apply bounded resilience wrappers with explicit retry count, timeout, and capped backoff (`cli/src/services/resilience.rs`) and surface terminal failures with deterministic `Try:` remediation guidance.
- For SCE operator-health commands, prefer deterministic local diagnostics over implicit pass/fail behavior: report the inspected environment scope, stable problem categories, severity/fixability classes, actionable remediation text, and any path/location facts needed to repair the issue; when repair mode exists, keep outcome vocabulary deterministic and idempotent (`cli/src/services/doctor.rs`).
- For repo-scoped hook-health diagnostics, resolve effective hooks location from git truth, distinguish git-unavailable vs outside-repo vs bare-repo failure modes explicitly, and compare required hook payload bytes against the canonical embedded hook assets so stale SCE-managed hook content is reported deterministically (`cli/src/services/doctor.rs`, `cli/src/services/setup.rs`).
- For future CLI domains, define trait-first service contracts with request/plan models in `cli/src/services/*` and keep placeholder implementations explicitly non-runnable until production behavior is approved.
- Model deferred integration boundaries with concrete event/capability data structures (for example MCP file-cache snapshots/policies and cloud-sync checkpoints) so later tasks can implement behavior without reshaping public seams.
- For pre-commit attribution finalization seams, keep pending staged and unstaged ranges explicitly separated in input models and finalize from staged ranges only, while carrying index/tree anchors for deterministic commit-time attribution binding.
- For commit-msg co-author policy seams, gate canonical trailer insertion on runtime controls (`SCE_DISABLED`, `SCE_COAUTHOR_ENABLED`) plus staged SCE-attribution presence, and enforce idempotent dedupe so allowed cases end with exactly one `Co-authored-by: SCE <sce@crocoder.dev>` trailer.
- For post-commit trace finalization seams, treat commit SHA as the idempotency identity, perform notes + DB writes in the same finalize pass when available, and enqueue retry-fallback entries that explicitly capture failed persistence targets for replay-safe recovery.
- For retry replay seams, process fallback queue entries in bounded batches, avoid same-pass duplicate trace processing, retry only failed targets, emit per-attempt runtime + persistence error-class metrics for operational visibility, and run a bounded replay pass from production post-commit/post-rewrite hook runtime with deterministic summary output.
- For post-rewrite remap ingestion seams, parse `<old_sha> <new_sha>` pairs from hook input strictly, ignore empty/no-op self-mapping rows, normalize rewrite method labels to lowercase (`amend`/`rebase` when recognized), and derive deterministic per-pair idempotency keys before dispatching remap requests.
- For rewrite trace transformation seams, materialize rewritten records through the canonical Agent Trace builder path, require finite confidence in `[0.0, 1.0]`, normalize confidence to two-decimal metadata strings, map quality thresholds to `final` (`>= 0.90`), `partial` (`0.60..0.89`), and `needs_review` (`< 0.60`), and preserve notes+DB dual-write plus retry-fallback parity.
- For local persistence rollout, ship core schema changes as idempotent `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` statements so migration reapplication is upgrade-safe across empty and preexisting local Turso DB states.
- For local hooks production writes, resolve one deterministic per-user persistent DB target (Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/agent-trace/local.db`; platform-equivalent state roots elsewhere), create parent directories before first use, and run schema bootstrap before DB write attempts.
- For hosted rewrite reconciliation persistence, extend the same migration seam (`apply_core_schema_migrations`) with deterministic schema/index statements and per-repository idempotency uniqueness for run/mapping replay safety.
- For hosted event intake seams, verify provider signatures before payload parsing (GitHub `sha256=<hex>` HMAC over body, GitLab token-equality secret check), resolve old/new heads from provider payload fields, and derive deterministic reconciliation run idempotency keys from provider+event+repo+head tuple material.
- For hosted rewrite mapping seams, resolve candidates deterministically in strict precedence order (patch-id exact, then range-diff score, then fuzzy score), classify top-score ties as `ambiguous`, enforce low-confidence unresolved behavior below `0.60`, and preserve stable outcome ordering via canonical candidate SHA sorting.
- For hosted reconciliation observability, publish run-level mapped/unmapped counts, confidence histogram buckets, runtime timing, and normalized error-class labels so retry/quality drift can be monitored without requiring a full dashboard surface.
- Keep crate-local onboarding docs in `cli/README.md` and sanity-check command examples against actual `sce` output whenever command messaging changes.
- Keep targeted CLI command-surface verification in flake checks: `checks.<system>.cli-setup-command-surface` runs from `cli/` and executes `cargo fmt --check` plus focused setup-related tests (`help_text_mentions_setup_target_flags`, `parser_routes_setup`, `run_setup_reports`).
- In `cli/flake.nix`, select the Rust toolchain via an explicit Rust overlay (`rust-overlay`) and thread that toolchain through `makeRustPlatform` so CLI check/build derivations do not rely on implicit nixpkgs Rust defaults.
- When the root flake imports a nested path flake that requires additional inputs (for example `rust-overlay` in `cli/flake.nix`), mirror those inputs in the root `inputs` block and wire `cli.inputs.<name>.follows` so root-level checks do not fail from missing nested flake arguments.
- For installable CLI release surfaces in nested flakes, expose an explicit named package plus default alias (`packages.sce` and `packages.default = packages.sce`) and pair it with a runnable app output (`apps.sce`) that points to the packaged binary path.
- For nested CLI flake release metadata, source the package/check version from repo-root `.version` and trim it at eval time so packaged outputs stay aligned without hardcoded semver strings in `cli/flake.nix`.
- For Cargo-based local CLI installation, document and verify `cargo install --path cli --locked` alongside a release build check (`cargo build --manifest-path cli/Cargo.toml --release`), and keep `publish = false` until explicit first-publish approval.

## Unit testing in Nix sandbox

- Unit tests must not depend on filesystem directories, temporary directories, or databases that could fail in Nix sandbox environments.
- Tests that require filesystem I/O, git repository operations, or database connections belong in integration tests, not unit tests.
- When a unit test needs filesystem, git, or database behavior that is not safe for `nix flake check`, delete it from the unit-test suite and reintroduce that coverage later as an integration test instead of keeping ignored tests in-tree.
- Pure unit tests should test in-memory logic, parsing, validation, and data transformations without external dependencies.
- The `TestTempDir` helper and similar filesystem fixtures should only be used in integration tests, not unit tests.
- In-memory database tests (e.g., `LocalDatabaseTarget::InMemory`) are acceptable for unit tests since they don't touch the filesystem.
- When adding new tests, prefer mocking/faking external dependencies over creating real filesystem or database state.
