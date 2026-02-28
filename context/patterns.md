# Patterns

## Config generation tooling

- Use the Nix dev shell as the canonical toolchain entrypoint for generation work.
- `flake.nix` includes `pkl` so contributors can run validation commands with `nix develop -c ...` without host-level installs.

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
- Keep generated-file safety markers in the renderer contract (`config/pkl/renderers/common.pkl`) so every generated Markdown artifact receives the same warning text.
- Validate each renderer module directly with `nix develop -c pkl eval <module-path>` before wiring output emission.

## Multi-file generation entrypoint

- Use `config/pkl/generate.pkl` as the single generation module for authored config outputs.
- Use `config/pkl/README.md` as the contributor-facing runbook for prerequisites, ownership boundaries, regeneration steps, and troubleshooting.
- Run multi-file generation with `nix develop -c pkl eval -m . config/pkl/generate.pkl` to emit to repository-root mapped paths.
- Run stale-output detection with `nix develop -c ./config/pkl/check-generated.sh`; it regenerates into a temporary directory and fails if generated-owned paths differ from committed outputs.
- For non-destructive verification during development, run `nix develop -c pkl eval -m context/tmp/t04-generated config/pkl/generate.pkl` and inspect emitted paths under `context/tmp/`.
- Keep `output.files` limited to generated-owned paths only (`config/{opencode_root}/{agent,command,skills,lib}` and `config/{claude_root}/{agents,commands,skills,lib}` where roots map to `.opencode` and `.claude`).
- Keep the shared drift library generated warning marker in `config/.opencode/lib/drift-collectors.js` (the canonical copied source) so both target library outputs inherit a stable, non-duplicating safety header.
