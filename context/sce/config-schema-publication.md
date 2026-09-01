# Config schema publication

## Scope

The canonical `sce/config.json` JSON Schema is authored in
`config/pkl/base/sce-config-schema.pkl`, generated ephemerally to
`config/schema/sce-config.schema.json`, and embedded into the CLI for config
validation. Release snapshots are checked in under
`schema/v<version>/config.json`; the versioned snapshot is the SCE config
schema, not the separate Agent Trace schema.

## Current behavior

- The schema's `$id` and the config `$schema` property use
  `https://sce.crocoder.dev/v<version>/config.json`, where `<version>` is the
  CLI release version.
- `sce setup` writes that same versioned declaration into a newly created
  repo-local `.sce/config.json`.
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
