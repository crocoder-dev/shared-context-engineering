# Plan: host-config-schema-subdomain

## Change summary

The canonical `sce/config.json` JSON Schema is authored in `config/pkl/base/sce-config-schema.pkl`, generated ephemerally to payload-relative `config/schema/sce-config.schema.json`, and embedded into the CLI at compile time. Its `$id` and the `$schema` property `const` both declare `https://sce.crocoder.dev/config.json`, and `sce setup` bootstraps every repo-local `.sce/config.json` with that same URL — but nothing serves it, so editors resolving the declaration get nothing back. This plan publishes the generated schema at a new `config.sce.crocoder.dev` host and moves the canonical declaration there.

This adds one GitHub Actions workflow that regenerates the schema from canonical Pkl on `main` and deploys it as a static artifact to a dedicated Vercel project, then repoints the canonical URL. The generated JSON stays uncommitted: `main` remains the source of truth and CI produces the hosted copy with the same toolchain the CLI embeds. The URL move is additive for existing repositories — the schema keeps accepting the current `https://sce.crocoder.dev/config.json` declaration so already-bootstrapped configs continue to validate. `SCE_WEB_BASE_URL` and the Agent Trace conversation/session/trace URLs it builds are untouched.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `https://config.sce.crocoder.dev/config.json` returns the generated SCE config JSON Schema, byte-identical to a local `nix run .#pkl-generate` of the same commit.
  - Validate: `curl -fsS https://config.sce.crocoder.dev/config.json | diff - "$GEN/config/schema/sce-config.schema.json"` where `$GEN` is a fresh `nix run .#pkl-generate -- "$(mktemp -d)"` output directory.
- [ ] AC2: `https://config.sce.crocoder.dev/` returns the same schema document as `/config.json`.
  - Validate: `diff <(curl -fsS https://config.sce.crocoder.dev/) <(curl -fsS https://config.sce.crocoder.dev/config.json)`.
- [ ] AC3: A newly bootstrapped repo-local config declares the new URL and validates.
  - Validate: in a scratch git repo, `sce setup --bootstrap-context` / normal setup writes `.sce/config.json` containing `"$schema": "https://config.sce.crocoder.dev/config.json"`, and `sce config validate` reports valid.
- [ ] AC4: A config file still declaring `https://sce.crocoder.dev/config.json` validates without error.
  - Validate: `sce config validate --config <file declaring the old URL>` reports valid; the corresponding Rust schema test covers both accepted values.
- [ ] AC5: The deploy workflow is lint-clean and triggers only on `main` pushes touching canonical schema inputs, plus manual dispatch.
  - Validate: `nix flake check` (`workflow-actionlint` derivation) passes; inspect the `on:` block of `.github/workflows/deploy-config-schema.yml`.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/cli/config-precedence-contract.md` — canonical `$schema` accepted values and the hosted schema location.
- `context/sce/setup-repo-local-config-bootstrap.md` — the bootstrap payload URL.
- `context/glossary.md` — `setup local bootstrap` entry quoting the bootstrap payload.
- `context/overview.md` — the sentence recording the accepted canonical `$schema` declaration, and the new deploy workflow in the workflow topology.
- `context/architecture.md` — Rust ownership of SCE web URLs, now including the config-schema host.

## Constraints and non-goals

- **In scope:** `.github/workflows/deploy-config-schema.yml`; `config/pkl/base/sce-config-schema.pkl`; `cli/src/services/agent_trace.rs`; `cli/src/services/setup/mod.rs`; `cli/src/services/config/schema.rs` tests; this repository's `.sce/config.json`; schema-URL mentions in repo docs.
- **Out of scope:** the Vercel account, project creation, domain attachment, and repository secret provisioning (manual, one-time, outside this repo); the `sce.crocoder.dev` web application itself; any change to Agent Trace conversation/session/trace URL construction; committing the generated schema.
- **Constraints:** generated targets stay ephemeral — the deploy job must produce the schema through the canonical Nix/Pkl path, not a checked-in copy. `nix run .#pkl-check-generated` enforces an exact generated-path inventory, so no new generated artifact may be introduced. New workflow files must pass the `workflow-actionlint` flake check.
- **Non-goal:** a general-purpose static site or docs host at `config.sce.crocoder.dev`. It serves one document.
- **Non-goal:** redirecting or serving `/config.json` from the existing `sce.crocoder.dev` app, which lives in another repository.

## Assumptions

- The schema is produced by `nix run .#pkl-generate -- <output-dir>` and read from `<output-dir>/config/schema/sce-config.schema.json`; there is no `scripts/generate-config-schema` and none is added.
- The new host constant lives beside `SCE_WEB_BASE_URL` in `cli/src/services/agent_trace.rs`, per the existing consolidation of SCE web URL ownership into that module.
- The deploy workflow follows the repository's existing Actions conventions: `DeterminateSystems/nix-installer-action@v22` plus `DeterminateSystems/magic-nix-cache-action@v14`, least-privilege `permissions`, and a concurrency group.
- Backwards compatibility is implemented by widening the `$schema` `const` to an `enum` of the new and old URLs, with the new URL as `$id`.

## Task stack

- [x] T01: `Deploy the generated config schema to config.sce.crocoder.dev` (status:done)
  - Task ID: T01
  - Goal: A GitHub Actions workflow regenerates the canonical SCE config JSON Schema on `main` and publishes it to the dedicated Vercel project serving `config.sce.crocoder.dev`, at both `/config.json` and `/`.
  - Boundaries (in/out of scope): In — `.github/workflows/deploy-config-schema.yml`, its Nix-based generation step, the ephemeral `vercel.json` rewrite it writes, and the Vercel CLI deploy using `VERCEL_TOKEN` / `VERCEL_ORG_ID` / `VERCEL_PROJECT_ID` secrets. Out — creating the Vercel project, attaching the domain, storing the secrets, and any change to the schema's contents or declared URL.
  - Dependencies: none
  - Done when: the workflow triggers on `push` to `main` limited to `config/pkl/**` and the workflow file itself plus `workflow_dispatch`; it generates the schema via `nix run .#pkl-generate`, stages it as `config.json` with a `/` → `/config.json` rewrite, and runs a production Vercel deploy; a manual dispatch run publishes the schema at `https://config.sce.crocoder.dev/config.json`.
  - Verification notes (commands or checks): `nix flake check` (specifically the `workflow-actionlint` derivation); locally reproduce the generation step with `nix run .#pkl-generate -- "$(mktemp -d)"` and confirm `config/schema/sce-config.schema.json` exists in the output; after merge, trigger the workflow manually and `curl -fsS https://config.sce.crocoder.dev/config.json`.
  - Completed: 2026-08-07
  - Files changed: `.github/workflows/deploy-config-schema.yml`
  - Evidence: `nix run .#pkl-generate -- "$(mktemp -d)"` produced `config/schema/sce-config.schema.json` at the path the workflow reads; `actionlint .github/workflows/deploy-config-schema.yml` passed; `nix build .#checks.x86_64-linux.workflow-actionlint` passed with the new file intent-added so the flake source includes it.
  - Notes: The workflow triggers on `main` pushes touching `config/pkl/**` or itself plus `workflow_dispatch`, guards the three Vercel secrets, generates the schema through the canonical Nix/Pkl path, stages it as `config.json` beside an ephemeral `vercel.json` carrying a `/` -> `/config.json` rewrite, and runs a pinned `vercel deploy --prod`. Nothing generated is committed. The live done check (`curl` against `https://config.sce.crocoder.dev/config.json` after a manual dispatch) was waived by the user for this task: it requires the workflow on `main` and the out-of-scope Vercel project, domain attachment, and secrets, which the user confirmed exist. AC1 and AC2 still cover the live behavior at `/validate` time.

- [ ] T02: `Repoint the canonical config schema URL at the config subdomain` (status:todo)
  - Task ID: T02
  - Goal: Newly written and generated configs declare `https://config.sce.crocoder.dev/config.json`, while configs declaring the previous `https://sce.crocoder.dev/config.json` keep validating.
  - Boundaries (in/out of scope): In — the schema `$id` and `$schema` accepted values in `config/pkl/base/sce-config-schema.pkl`; a config-schema URL constant in `cli/src/services/agent_trace.rs`; the bootstrap payload in `cli/src/services/setup/mod.rs`; schema-validation tests covering both accepted URLs; this repository's `.sce/config.json`; schema-URL mentions in `README.md`, `AGENTS.md`, and `cli/README.md` where they refer to the config schema. Out — `SCE_WEB_BASE_URL` and every Agent Trace URL built from it; the deploy workflow from T01; durable `context/` updates, which `/validate` synchronizes.
  - Dependencies: T01
  - Done when: `sce setup` writes `{"$schema": "https://config.sce.crocoder.dev/config.json"}`; the generated schema's `$id` is the new URL and its `$schema` property accepts both the new and old URLs; `sce config validate` passes for a config declaring either; this repository's `.sce/config.json` uses the new URL.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect the generated `config/schema/sce-config.schema.json` from `nix run .#pkl-generate -- "$(mktemp -d)"` for the new `$id` and both accepted `$schema` values; run `sce config validate` against fixtures declaring each URL.

## Open questions

- The Vercel project, the `config.sce.crocoder.dev` domain attachment, and the three repository secrets are manual one-time steps this plan cannot perform. T01's workflow will fail until they exist. Confirm you are doing that setup, or the plan should be reordered so T02 does not publish a URL nothing serves.
- Should the old `https://sce.crocoder.dev/config.json` declaration be accepted indefinitely, or deprecated on a schedule? This plan accepts both with no deprecation, which means the schema permanently documents two URLs. The alternative — a hard cutover — is less work but breaks `sce config validate` for every repository whose config was bootstrapped before this change, until each one is edited.
