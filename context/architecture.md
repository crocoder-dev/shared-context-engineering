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
- `.github/workflows/pkl-generated-parity.yml` (CI wrapper that runs the parity check for pushes to `main` and pull requests targeting `main`)
- `.github/workflows/agnix-config-validate-report.yml` (CI wrapper that runs `agnix validate` from `config/`, writes `context/tmp/ci-reports/agnix-validate-report.txt`, uploads it when non-info findings are present, and fails on any non-info finding)

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
- `cli/src/app.rs` provides a `lexopt`-based argument parser and dispatch loop with deterministic help, placeholder responses, and consistent `anyhow`-driven error exits.
- `cli/src/command_surface.rs` is the source of truth for top-level command contract metadata (`help`, `setup`, `mcp`, `hooks`, `sync`) and explicit implemented-vs-placeholder status.
- `cli/src/services/local_db.rs` provides the local Turso data adapter, including `Builder::new_local(...)` initialization and async execute/query smoke checks for in-memory and file-backed targets.
- `cli/src/services/setup.rs` defines the setup command contract (`SetupMode`, `SetupTarget`, CLI flag parser/validator), an `inquire`-backed interactive target prompter (`InquireSetupTargetPrompter`), setup dispatch outcomes (proceed/cancelled), plus a setup planning seam (`SetupService`) and placeholder implementation that returns deferred setup steps without performing setup mutations.
- `cli/src/services/mcp.rs` defines MCP file-cache capability contracts (`McpService`, transport/capability snapshots, cache policy) with non-runnable placeholder tool declarations.
- `cli/src/services/hooks.rs` defines hook-event and generated-region tracking contracts (`HookService`, `HookEventModel`) with placeholder-safe no-op recording.
- `cli/src/services/sync.rs` runs the local adapter through a tokio current-thread runtime and composes a placeholder cloud-sync abstraction (`CloudSyncGateway`) so local Turso validation and deferred cloud planning remain separated.
- `cli/src/services/` contains module boundaries for setup, MCP, hooks, sync, and local DB adapters with explicit trait seams for future implementations.
- `cli/README.md` is the crate-local onboarding and usage source of truth for placeholder behavior, safety limitations, and roadmap mapping back to service contracts.

This phase establishes compile-safe extension seams with a minimal dependency baseline (`anyhow`, `inquire`, `lexopt`, `tokio`, `turso`); local Turso connectivity smoke checks now exist, while broader runtime integrations remain deferred.

## Shared Context Drift parity mapping

Shared Context Drift has an explicit target-parity mapping for internal/subagent usage across generated outputs.

- Canonical agent source remains `shared.agents["shared-context-drift"]` in `config/pkl/base/shared-content.pkl`.
- OpenCode subagent behavior is declared in `config/pkl/renderers/opencode-metadata.pkl` via `agentBehaviorBlocks["shared-context-drift"]`, which emits `mode: subagent` and `hidden: true` into `config/.opencode/agent/Shared Context Drift.md`.
- Claude has no supported `hidden`/`mode` equivalent in this repo's generator contract, so parity is represented with supported fields only: delegated/internal guidance in `agentDescriptions["shared-context-drift"]` and `agentSystemPreambleBlocks["shared-context-drift"]` in `config/pkl/renderers/claude-metadata.pkl`, rendered to `config/.claude/agents/shared-context-drift.md`.
- This is an intentional capability-gap mapping: OpenCode uses explicit frontmatter controls; Claude uses instruction-level delegation and command/task routing guidance.
