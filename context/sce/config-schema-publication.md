# Config schema publication

## Scope

The canonical `sce/config.json` JSON Schema is authored in
`config/pkl/base/sce-config-schema.pkl`, generated ephemerally to
`config/schema/sce-config.schema.json`, and embedded into the CLI for config
validation. Release snapshots are checked in under
`schema/v<version>/config.json`; the versioned snapshot is the SCE config
schema, not the separate Agent Trace schema.

## Current behavior

- The schema's `$id` is `https://sce.crocoder.dev/v<version>/config.json`,
  where `<version>` is the CLI release version.
- The config `$schema` property accepts any canonical
  `https://sce.crocoder.dev/v<major>.<minor>.<patch>[-pre][+build]/config.json`
  declaration in the CLI's `major.minor` release series through an anchored
  `pattern`. `config/pkl/base/sce-config-schema-url.pkl` derives the concrete
  URL and the series pattern from `.version`. Other series, malformed or foreign
  URLs, and non-string values are rejected. See
  [the series-acceptance decision](../decisions/2026-11-10-config-schema-accepts-release-series-declarations.md).
- Snapshots published before this rule keep their exact-version `const` and are
  never rewritten. The next `nix run .#bump-version` publishes the first
  series-pattern snapshot.
- `sce setup` writes that same versioned declaration into a newly created
  repo-local `.sce/config.json`. It also upgrades an older-series declaration
  in the existing canonical repository and global config, and changes only
  `$schema`. It never downgrades a newer series or rewrites an unrecognized
  value. See
  [the setup schema upgrade decision](../decisions/2026-11-10-setup-upgrades-config-schema-declarations.md).
- The generated working-tree schema remains ephemeral. Its version is read
  from the root `.version` file during Pkl generation. Repository builds and
  packaged builds consume the validated generated payload through Cargo
  `OUT_DIR` or the packaging fallback.
- `nix run .#bump-version` generates the current SCE config schema after
  updating the version and writes it to `schema/v<version>/config.json`; an
  existing snapshot is refreshed when it differs.
- `sce.crocoder.dev` remains the SCE web application host. Agent Trace
  conversation, session, and trace URL construction continues to use the
  shared Rust URL owner in `cli/src/services/agent_trace.rs`.

The former subdomain publication workflow was removed when the canonical
config declaration returned to the SCE web application URL. The historical
plan remains in `context/plans/host-config-schema-subdomain.md`.
