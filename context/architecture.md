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
- `.github/workflows/pkl-generated-parity.yml` (CI wrapper that runs the parity check for pushes to `main` and pull requests targeting `main`)

The scaffold provides stable canonical content-unit identifiers and reusable target-agnostic text primitives for all planned authored generated classes (agents, commands, skills, shared library file).

Renderer modules apply target-specific metadata/frontmatter rules while reusing canonical content bodies:

- OpenCode renderer emits frontmatter with `agent`/`permission`/`compatibility: opencode` conventions.
- Claude renderer emits frontmatter with `allowed-tools`/`model`/`compatibility: claude` conventions.
- Shared renderer contracts (`RenderedTargetDocument`, command descriptions, skill descriptions) live in `config/pkl/renderers/common.pkl`.
- Target-specific metadata tables are isolated in `config/pkl/renderers/opencode-metadata.pkl` and `config/pkl/renderers/claude-metadata.pkl`.
- Metadata key coverage is enforced by `config/pkl/renderers/metadata-coverage-check.pkl`, which resolves all required lookup keys for both targets and fails evaluation on missing entries.
- Both renderers expose per-class rendered document objects (`agents`, `commands`, `skills`) consumed by `config/pkl/generate.pkl`.
- `config/pkl/generate.pkl` emits deterministic `output.files` mappings for all authored generated targets: OpenCode/Claude agents, commands, skills, and `lib/drift-collectors.js` in both trees.
- Generated-file safety markers are part of emitted artifacts: Markdown outputs include an HTML warning comment after frontmatter, and the shared library output carries a leading JS generated warning header.
- `config/pkl/check-generated.sh` is intentionally dev-shell scoped (`nix develop -c ...`): it requires `IN_NIX_SHELL`, runs `pkl eval -m <tmp> config/pkl/generate.pkl`, and fails when generated-owned paths drift.

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
