# Config schema publication

## Scope

The canonical `sce/config.json` JSON Schema is authored in
`config/pkl/base/sce-config-schema.pkl`, generated ephemerally, and embedded
into the CLI for config validation. This file records that there is currently
no separate public schema-hosting workflow; the canonical config declaration
uses the SCE web application URL.

## Current behavior

- The schema's `$id` and the config `$schema` property use
  `https://sce.crocoder.dev/config.json`.
- `sce setup` writes that same declaration into a newly created repo-local
  `.sce/config.json`.
- The generated schema is not committed. Repository builds and packaged builds
  consume the validated generated payload through Cargo `OUT_DIR` or the
  packaging fallback.
- `sce.crocoder.dev` remains the SCE web application host. Agent Trace
  conversation, session, and trace URL construction continues to use the
  shared Rust URL owner in `cli/src/services/agent_trace.rs`.

The former subdomain publication workflow was removed when the canonical
config declaration returned to the SCE web application URL. The historical
plan remains in `context/plans/host-config-schema-subdomain.md`.
