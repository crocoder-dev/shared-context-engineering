# Patterns

## Config generation tooling

- Use the Nix dev shell as the canonical toolchain entrypoint for generation work.
- `flake.nix` includes `pkl` so contributors can run validation commands with `nix develop -c ...` without host-level installs.

## Flake app entrypoints

- Expose operational workflows as flake apps so commands are stable and system-mapped across supported `flake-utils` default systems.
- Current repo command contract: `nix run .#sync-opencode-config` is the canonical entrypoint for staged regeneration/replacement of `config/` and replacement of repository-root `.opencode/` from regenerated `config/.opencode/`.
- For destructive config replacement flows, regenerate into a temporary staged `config/` first, validate required generated directories exist, and only then swap live `config/`.
- For destructive root `.opencode/` replacement flows, keep exclusions explicit (for example `node_modules`), use backup-and-restore around swap, and run a source/target tree parity check with the same exclusions.
- Keep command help available via `nix run .#sync-opencode-config -- --help` to provide deterministic usage checks during incremental implementation.

## Dev-shell fallback shims for unavailable nixpkgs tools

- When required CLI tools are not available as direct nixpkgs attrs, use the least-friction dev-shell fallback that keeps commands usable in `nix develop`.
- Current repo behavior: include `cargo` and `rustc` in `devShells.default`, export `~/.cargo/bin` on `PATH`, and auto-run `cargo install --locked agnix-cli` in `shellHook` when `agnix` is missing.
- `agnix-lsp` currently remains shim-based: use `AGNIX_LSP_BIN` when set and executable, otherwise use `~/.cargo/bin/agnix-lsp` when present, otherwise print manual install guidance and exit non-zero.
- `shellHook` prints a version banner for `bun`, `pkl`, `tsc`, `typescript-language-server`, `rustc`, and `agnix` so shell state is visible on entry.

## Pkl renderer layering

- Keep target-agnostic canonical content in `config/pkl/base/shared-content.pkl`.
- Keep `config/pkl/base/shared-content.pkl` synchronized with the canonical authored instruction bodies (currently mirrored from the OpenCode source tree under `config/{opencode_root}` for `agent`, `command`, and `skills`, with frontmatter removed) before regenerating targets.
- Implement target-specific formatting in dedicated renderer modules under `config/pkl/renderers/`.
- Keep shared renderer contracts and shared description maps in `config/pkl/renderers/common.pkl`.
- Keep per-target metadata tables in dedicated modules (`opencode-metadata.pkl`, `claude-metadata.pkl`) and import them into target renderer modules.
- Add and run `config/pkl/renderers/metadata-coverage-check.pkl` as a fail-fast metadata completeness guard whenever shared slugs or metadata tables change.
- In renderer modules, produce per-item document objects with explicit `frontmatter`, `body`, and combined `rendered` fields to keep formatting deterministic and easy to map in a later output stage.
- Keep the Markdown renderer contract in `config/pkl/renderers/common.pkl` limited to deterministic `frontmatter + body` assembly without injected generated-file marker text.
- Validate each renderer module directly with `nix develop -c pkl eval <module-path>` before wiring output emission.

## Multi-file generation entrypoint

- Use `config/pkl/generate.pkl` as the single generation module for authored config outputs.
- Use `config/pkl/README.md` as the contributor-facing runbook for prerequisites, ownership boundaries, regeneration steps, and troubleshooting.
- Run multi-file generation with `nix develop -c pkl eval -m . config/pkl/generate.pkl` to emit to repository-root mapped paths.
- Run stale-output detection with `nix develop -c ./config/pkl/check-generated.sh`; the script is a dev-shell integration test, exits non-zero outside `nix develop`, regenerates into a temporary directory, and fails if generated-owned paths differ from committed outputs.
- Keep CI parity enforcement aligned with local workflow by running the same command in `.github/workflows/pkl-generated-parity.yml` for pushes to `main` and pull requests targeting `main`.
- Keep agnix config validation on the same trigger contract (`push`/`pull_request` to `main`) in `.github/workflows/agnix-config-validate-report.yml` with job defaults pinned to `working-directory: config`.
- In the agnix CI workflow, capture command output to `context/tmp/ci-reports/agnix-validate-report.txt`, treat `warning:`/`error:`/`fatal:` findings as non-info gate failures, and upload the captured report as a GitHub artifact (`agnix-validate-report`) only when non-info findings are present.
- Do not run `evals/` test suites autonomously during plan-task execution; run them only when the user explicitly requests eval coverage.
- For non-destructive verification during development, run `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` and inspect emitted paths under `context/tmp/`.
- Keep `output.files` limited to generated-owned paths only (`config/{opencode_root}/{agent,command,skills,lib}` and `config/{claude_root}/{agents,commands,skills,lib}` where roots map to `.opencode` and `.claude`).
- Keep the shared drift library source marker-free in `config/.opencode/lib/drift-collectors.js` so generated `lib/drift-collectors.js` outputs stay behavior-only and deterministic across both targets.

## Internal subagent parity mapping

- Encode internal-agent parity by target capability, not by forcing unsupported frontmatter keys.
- For OpenCode agents that must be internal, set behavior flags in `config/pkl/renderers/opencode-metadata.pkl` (`agentBehaviorBlocks`) and render those directly into frontmatter.
- For Claude agents, represent equivalent intent using supported metadata and body guidance in `config/pkl/renderers/claude-metadata.pkl` (for example description + preamble blocks for delegated command/task routing).
- Keep parity decisions reproducible by validating generated outputs directly (for Shared Context Drift: `config/.opencode/agent/Shared Context Drift.md` and `config/.claude/agents/shared-context-drift.md`).
