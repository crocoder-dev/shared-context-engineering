# Decision: Make workflow optionality an install-time concern driven by a catalog flag and a per-repository persisted selection

Date: 2026-07-31
Status: Accepted
Plan: `context/plans/optional-workflow-installation.md`
Task: `T01, T02, T03, T04, T05, T06`

## Context

Every catalog workflow was installed into every selected target
unconditionally. `/brownfield` made that uniformity uncomfortable: it is a
cold-start reconstruction tool most repositories run once or never, yet every
`sce setup` shipped its command and its `sce-brownfield` skill package into
`.opencode/`, `.claude/`, and `.pi/`, and `sce doctor` then required those files
to stay present and unmodified forever.

Three constraints shaped the answer. Pkl is the single canonical source for
workflow identity and for the config JSON Schema, so a selection surface must
not introduce a second place where workflows are named. The generation contract
asserts an exact artifact-path inventory, so anything that conditioned
generation would make that inventory depend on runtime state. And the repository
had accumulated a clear cost signal for per-workflow branching in Rust: adding
the sixth workflow needed no Rust change at all, and a selection feature that
required a match arm per workflow would give that back.

There was also a correctness hazard specific to a persisted selection. Setup is
routinely rerun, and its install policy is remove-and-replace. If an omitted
flag meant "select nothing", a plain `sce setup --claude --non-interactive`
would silently delete a workflow the repository had deliberately opted into.

## Decision

Express workflow optionality as a single `optional` flag on the typed workflow
catalog record, carried out of Pkl through one generated manifest into one
build-generated Rust catalog, and resolve it per repository as a persisted
selection that install and health-check paths share — never as a condition on
generation.

Concretely: `WorkflowRecord.optional` defaults to `false` and is `true` only for
`brownfield`; `config/pkl/base/optional-workflow-manifest.pkl` projects the
optional records into the generated `config/optional-workflows.json`;
`cli/build.rs` turns that manifest into the `OPTIONAL_WORKFLOWS` static; and a
repository's opt-in lives in repo-local `integrations.optional_workflows`,
whose accepted values the config schema derives from the same catalog records.

## Rationale

Keeping optionality out of generation is what makes the rest safe. All six
workflows are still authored, composed, and generated for all three targets, so
the payload, the exact-path contract, and the metadata-coverage contract are
untouched by a repository's choice — the generated output remains a pure
function of the authored source. Optionality only ever subtracts from what an
install writes to disk.

Routing identity through one asserted manifest keeps Pkl canonical without
making the CLI parse Pkl. The generation contract asserts the manifest's
content against the catalog rather than merely permitting the path, so a
manifest that drifts from the catalog fails the build instead of silently
teaching the CLI something false. Downstream of that, every consumer — accepted
`--workflow` values, prompt rows, config-key validation, doctor's expected
inventory — reads the generated catalog, so marking another workflow optional is
one Pkl line plus a schema regeneration, with no new Rust branch.

Treating an absent selection as "reuse what is persisted" rather than "select
nothing" is the only reading under which a rerun is safe. The flag, when
supplied, is the exact selection for that run, which keeps a single explicit
mental model: state the selection or inherit it, never partially merge.

Having installation and doctor share one filter — and deleting the unfiltered
enumeration path entirely — removes the drift class where a run installs one set
of files and a later health check demands another. There is no remaining code
path that can enumerate embedded assets without consulting the selection.

## Alternatives considered

- **Condition generation on the flag** — Rejected; the generated artifact
  inventory would stop being a function of the authored source, and the exact
  path count that guards unintended inventory changes would become unstable.
- **Enumerate optional workflows in Rust** — Rejected; it reintroduces the
  per-workflow branching the catalog exists to eliminate, and creates a second
  place where a workflow's identity can drift from Pkl.
- **Treat an absent `--workflow` as an empty selection** — Rejected; under
  remove-and-replace, a routine rerun would uninstall an earlier opt-in with no
  warning.
- **A general per-workflow enable/disable system covering all six workflows** —
  Rejected as out of scope; the five core workflows are the product's baseline,
  and making them individually removable would turn a narrow install choice into
  an open configuration surface.
- **Report a deselected workflow's leftover files as stray** — Rejected;
  whole-target remove-and-replace already removes them, so the report would name
  a state the install path cannot produce.

## Compatibility and risks

- A default `sce setup` now installs five core workflows and no `/brownfield`
  assets, reversing the previous unconditional install. Repositories that want
  `/brownfield` must opt in once with `sce setup --workflow brownfield` or the
  interactive prompt; the choice then persists.
- `integrations.optional_workflows` becomes a public repo-local config key that
  downstream repositories will persist. Removing or renaming it later would
  invalidate their config files, which is the main reason this decision is
  costly to reverse.
- The generated artifact inventory grows by one path (70 → 71) for the manifest.
  No existing generated file changes.
- `cli/build.rs` gains a hard dependency on the staged
  `config/optional-workflows.json`: a payload missing it, or carrying an
  unexpected `schemaVersion`, fails the build rather than degrading.
- Schema-validation error text repository-wide now carries a JSON-pointer prefix
  so a rejected value names its key. No existing assertion depended on the prior
  unprefixed text.
- The generated OpenCode Code agent still lists `"sce-brownfield": allow`
  regardless of installation. The permission is inert when the skill is absent,
  and filtering it would push a runtime concern back into generation.
- This repository's own `.sce/config.json` records no selection, so its own
  `sce doctor` no longer inspects its committed `/brownfield` mirrors. Recording
  `["brownfield"]` there would restore that coverage.

## Guardrails

- The `optional` flag is an install-time statement only. It must never condition
  generation, composition, routing, permissions, or the artifact-path contract.
- Optional-workflow identity leaves Pkl through exactly one artifact,
  `config/optional-workflows.json`, whose content the generation contract
  asserts against the catalog.
- Accepted ids, error text, prompt rows, config-schema enum values, and asset
  membership are all derived from the catalog. Adding an optional workflow must
  not require a new Rust match arm.
- Asset membership is derived from each workflow's `commandSlug`/`skillSlug`
  plus the target's own command and skill directory names — never an enumerated
  file list.
- An absent selection means "reuse the persisted value", never "select nothing".
- Installation and doctor consult the selection through the same filter; no
  unfiltered embedded-asset enumeration path may be reintroduced.
- Unknown ids are rejected during request resolution, before any file or config
  write.
- Only workflows explicitly marked `optional` participate. The five core
  workflows have no enable/disable surface.

## Consequences

- The workflow catalog record grows one field, and `brownfield` is the only
  workflow that sets it.
- The CLI gains a build-time codegen seam (`OPTIONAL_WORKFLOWS`) alongside the
  existing setup-asset and migration manifests.
- `sce setup` gains a repeatable `--workflow <slug>` flag and an interactive
  multi-select shown after target selection, seeded from the persisted
  selection; `--workflow` is rejected alongside `--bootstrap-context` and on a
  hooks-only run, neither of which installs target assets.
- `sce doctor`'s required inventory is now scoped twice: by resolved target and
  by persisted optional-workflow selection. An unselected optional workflow
  contributes no row and no missing-file problem.
- Interactive setup resolves the repository root before prompting, so a non-git
  directory now fails before the target prompt rather than after it.

## Follow-up

- None.

## References

- Plan: [`optional-workflow-installation`](../plans/optional-workflow-installation.md)
- Task: `T01, T02, T03, T04, T05, T06`
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Patterns`](../patterns.md)
- Current-state context: [`CLI command surface`](../cli/cli-command-surface.md)
- Current-state context: [`CLI config precedence contract`](../cli/config-precedence-contract.md)
- Current-state context: [`Setup repo-local config bootstrap`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`Brownfield workflow`](../sce/brownfield-workflow.md)
- Current-state context: [`Doctor human text contract`](../sce/doctor-human-text-contract.md)
- Evidence: [`workflow-catalog.pkl`](../../config/pkl/base/workflow-catalog.pkl)
- Evidence: [`optional-workflow-manifest.pkl`](../../config/pkl/base/optional-workflow-manifest.pkl)
- Evidence: [`generation-contract-check.pkl`](../../config/pkl/renderers/generation-contract-check.pkl)
- Evidence: [`sce-config-schema.pkl`](../../config/pkl/base/sce-config-schema.pkl)
- Evidence: [`cli/build.rs`](../../cli/build.rs)
- Related decision: [`Register /brownfield as the sixth cross-target SCE workflow`](2026-07-31-brownfield-sixth-cross-target-workflow.md)
