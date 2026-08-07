# Config schema publication

## Scope

How the canonical `sce/config.json` JSON Schema reaches a public HTTPS URL that
editors can resolve. Authoring and the accepted `$schema` declaration values
belong to [`../cli/config-precedence-contract.md`](../cli/config-precedence-contract.md);
this file owns only the hosting and deploy path.

## Hosted surface

- `config.sce.crocoder.dev` is a single-document host. It serves the generated
  SCE config JSON Schema at `/config.json`, and `/` rewrites to that same
  document. It is not a docs site or a general-purpose static host.
- It is distinct from `sce.crocoder.dev`, the SCE web application, which lives in
  another repository and is reached through `SCE_WEB_BASE_URL`. Nothing about
  Agent Trace conversation, session, or trace URL construction is involved.

## Publication path

`.github/workflows/deploy-config-schema.yml` owns publication.

- Triggers: `push` to `main` limited to `config/pkl/**` and the workflow file
  itself, plus `workflow_dispatch`. There is no pull-request trigger, so a
  deploy only ever runs from merged `main` state or a deliberate manual run.
- The job holds `permissions: contents: read` and a non-cancelling
  `deploy-config-schema` concurrency group, so overlapping deploys queue rather
  than race for the production alias.
- It fails fast with the missing names when any of `VERCEL_TOKEN`,
  `VERCEL_ORG_ID`, or `VERCEL_PROJECT_ID` is unset, rather than letting the
  Vercel CLI fail later with a less specific error.
- The schema is produced through the canonical Nix/Pkl path,
  `nix run .#pkl-generate -- <output-dir>`, and read from
  `<output-dir>/config/schema/sce-config.schema.json` — the same toolchain and
  the same document the CLI embeds at compile time. There is no checked-in copy
  and no bespoke generation script.
- The job stages that file as `config.json` in a temporary directory beside an
  ephemeral `vercel.json` declaring a `/` → `/config.json` rewrite, then runs a
  pinned `vercel deploy --prod` against the staged directory. Neither the schema
  nor the `vercel.json` is committed, which keeps the generated-path inventory
  enforced by `nix run .#pkl-check-generated` unchanged.

## Operational boundaries

- The Vercel project, the `config.sce.crocoder.dev` domain attachment, and the
  three repository secrets are manual one-time setup performed outside this
  repository. The workflow's secret guard fails the run when they are absent.
- Deploys go straight to production; there is no staging alias or preview step.
  The document's correctness is guarded upstream instead, by the Pkl authoring
  source and the flake's generated-output checks.
- The workflow is covered by the root `nix flake check` `workflow-actionlint`
  derivation like every other file under `.github/workflows/`.
