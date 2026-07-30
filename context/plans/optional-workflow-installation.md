# Plan: optional-workflow-installation

## Change summary

Introduce a general "optional workflow" concept to SCE setup and make
`/brownfield` its first member, installed only when explicitly selected. Today
all six catalog workflows are unconditionally embedded and installed into every
selected integration target; after this change the workflow catalog marks a
workflow optional, `sce setup` resolves a selection set, and only selected
optional workflows are written into `.opencode/`, `.claude/`, and `.pi/`.

The selection surface is built for more optional workflows than brownfield. Pkl
stays canonical: `config/pkl/base/workflow-catalog.pkl` gains an `optional` flag,
and generation emits a small optional-workflow manifest that `cli/build.rs`
turns into a Rust catalog. The CLI then derives everything from that catalog —
the repeatable `--workflow <slug>` flag's accepted values, the interactive
multi-select prompt's rows, the persisted `integrations.optional_workflows`
config values, and doctor's notion of which embedded assets are expected on
disk. Adding a second optional workflow becomes a one-line Pkl change plus a
config-schema enum regeneration, with no new Rust branches.

Generation itself is unchanged in content: all six workflows are still generated
for all three targets and remain part of the ephemeral payload and the
`pkl-check-generated` contract. Optionality is an install-time and doctor-time
concern only.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: A default setup run installs no brownfield assets. In a scratch git
      repository, `sce setup --claude --non-interactive` produces a `.claude/`
      tree containing no `commands/brownfield.md` and no `skills/sce-brownfield/`,
      while the five core workflows are present.
  - Validate: `./scripts/run-cli-cargo.sh run -- setup --claude --non-interactive` in a temp git repo, then `test ! -e .claude/commands/brownfield.md && test ! -e .claude/skills/sce-brownfield && test -e .claude/commands/validate.md`
- [x] AC2: An explicit non-interactive opt-in installs brownfield and records the
      selection. `sce setup --claude --non-interactive --workflow brownfield`
      writes `.claude/commands/brownfield.md` plus `.claude/skills/sce-brownfield/**`
      and leaves `integrations.optional_workflows: ["brownfield"]` in
      `.sce/config.json`.
  - Validate: run the command in a temp git repo, then `test -e .claude/skills/sce-brownfield/SKILL.md && jq -e '.integrations.optional_workflows == ["brownfield"]' .sce/config.json`
- [x] AC3: A repeat non-interactive run without `--workflow` preserves the
      recorded selection rather than silently uninstalling it.
  - Validate: after AC2, rerun `./scripts/run-cli-cargo.sh run -- setup --claude --non-interactive`, then `test -e .claude/skills/sce-brownfield/SKILL.md`
- [x] AC4: An unknown optional workflow slug is rejected with an actionable
      validation error naming the available slugs, and does not write any files.
  - Validate: `./scripts/run-cli-cargo.sh run -- setup --claude --non-interactive --workflow nonesuch` exits non-zero with a message listing `brownfield`
- [x] AC5: Interactive setup presents an optional-workflow multi-select after
      target selection, with every row unchecked on a first run and pre-checked
      from `integrations.optional_workflows` on later runs; the prompt is skipped
      entirely when the catalog has no optional workflows.
  - Validate: manual TTY runs of `sce setup` covering the no-persisted-selection
    and persisted-selection cases. The unit-test requirement was dropped by the
    user during validation on 2026-07-31; the empty-catalog skip is therefore
    accepted on inspection of the `optional_workflow_prompt_inputs` early return
    rather than proven by a check.
- [x] AC6: `sce doctor` reports no missing-file problems for an unselected
      optional workflow, and still reports drift or missing files for a selected
      one.
  - Validate: unit tests in `cli/src/services/doctor/` over integration-child collection with and without `brownfield` in the resolved selection; plus `./scripts/run-cli-cargo.sh run -- doctor` after AC1 showing no brownfield rows
- [x] AC7: `.sce/config.json` carrying `integrations.optional_workflows: ["brownfield"]`
      validates, and an unknown id fails validation with a schema error.
  - Validate: `./scripts/run-cli-cargo.sh run -- config validate` against both fixtures
- [x] AC8: Generation still emits all six workflows for all three targets, plus
      the new optional-workflow manifest, and the generation contract asserts the
      manifest's content rather than merely permitting it.
  - Validate: `nix run .#pkl-check-generated`

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/overview.md` (setup target-selection flow, optional workflow install surface, repo-local config keys)
- `context/architecture.md` (workflow catalog `optional` flag, generated manifest, build.rs-generated Rust catalog, setup/doctor selection seam)
- `context/patterns.md` (optional-workflow selection precedence and prompt conventions)
- `context/cli/cli-command-surface.md` (`sce setup --workflow <slug>`)
- `context/cli/config-precedence-contract.md` (`integrations.optional_workflows`)
- `context/sce/setup-repo-local-config-bootstrap.md` (persisted selection)
- `context/sce/brownfield-workflow.md` (opt-in install status)
- `context/sce/doctor-human-text-contract.md` (selection-scoped integration rows)
- `context/context-map.md` (annotations for the files above)

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-catalog.pkl`, `config/pkl/base/sce-config-schema.pkl`,
  `config/pkl/generate.pkl`, `config/pkl/renderers/generation-contract-check.pkl`,
  `cli/build.rs`, `cli/src/services/setup/**`, `cli/src/services/parse/**` and
  the setup clap surface, `cli/src/services/config/{types,schema}.rs`, and
  `cli/src/services/doctor/inspect.rs`.
- **Out of scope:** the content of any workflow package; the OpenCode agent
  permission lists; `sce config show` reporting of integration state; global
  (non repo-local) config carrying the selection; removing `/brownfield` from
  this repository's own committed `.opencode/`, `.claude/`, and `.pi/` mirrors,
  which stay opted in.
- **Constraints:** Pkl remains the single canonical source for workflow identity
  and the config JSON Schema; generated output stays ephemeral; the existing
  remove-and-replace install policy and no-backup policy are unchanged; adding a
  future optional workflow must not require new Rust match arms.
- **Non-goal:** a general per-workflow enable/disable system covering the five
  core workflows. Only workflows explicitly marked `optional` in the catalog
  participate.
- **Non-goal:** detecting and reporting a previously-installed optional workflow
  that is no longer selected. The whole-target remove-and-replace install already
  removes its files, and doctor simply stops expecting them.

## Assumptions

- When `--workflow` is absent on a non-interactive run, the persisted
  `integrations.optional_workflows` set is used rather than an empty set, so a
  repeat `sce setup --claude --non-interactive` does not silently uninstall a
  previously selected optional workflow (AC3). Passing `--workflow` at all makes
  the flag list the exact selection for that run.
- Selection is persisted repo-locally in `.sce/config.json` alongside the
  existing `integrations.target` key, written by the same setup persistence path.
- `--workflow` is rejected alongside `--bootstrap-context`, and on a hooks-only
  run (`sce setup --hooks` with no target), matching the existing option
  compatibility style, since neither installs target assets.
- Optional-workflow asset membership is derived in Rust from the manifest's
  `commandSlug`/`skillSlug` plus each target's already-owned command and skill
  directory names, rather than from an enumerated file list.

## Task stack

- [x] T01: `Mark workflows optional in the Pkl catalog and emit a manifest` (status:done)
  - Task ID: T01
  - Goal: `workflow-catalog.pkl` carries an `optional` flag (default `false`, `true` for brownfield), and generation emits `config/optional-workflows.json` describing every optional workflow's id, title, description, command slug, and skill slug.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-catalog.pkl`, `config/pkl/generate.pkl`, `config/pkl/renderers/generation-contract-check.pkl` (required-path inventory plus an assertion that the manifest lists brownfield as optional with its catalog slugs), and any renderer plumbing needed to reach the catalog record. Out — Rust, the config JSON Schema, and any change to workflow package content or the per-target generated file set.
  - Dependencies: none
  - Done when: the generated payload contains `config/optional-workflows.json` with exactly one entry (`brownfield`), the six workflows' generated files are otherwise byte-identical to before, and the contract check asserts the manifest rather than merely allowing the path.
  - Verification notes (commands or checks): `nix run .#pkl-generate -- "$(mktemp -d)"` and inspect the manifest; `nix run .#pkl-check-generated`.
  - Implementation: `WorkflowRecord.optional: Boolean = false` added in
    `config/pkl/base/workflow-catalog.pkl`, set `true` on `brownfield` only. New
    `config/pkl/base/optional-workflow-manifest.pkl` projects the catalog's
    optional records into `{ schemaVersion, workflows[{ id, title, description,
    commandSlug, skillSlug }] }` and renders it with `JsonRenderer`;
    `config/pkl/generate.pkl` emits it as `config/optional-workflows.json`.
    `config/pkl/renderers/generation-contract-check.pkl` adds the path to
    `expectedArtifactPaths`, raises `expectedArtifactPathCount` 70 → 71, and adds
    an `optional-workflow-manifest` check that asserts the generated manifest text
    against the catalog: every optional workflow appears with its catalog title
    and both slugs, no core workflow id appears, and `schemaVersion` is 1.
  - Verification: `nix run .#pkl-generate` into a temp dir emitted 71 files with
    the manifest containing exactly one entry (`brownfield`); `diff -r` against a
    payload generated from a detached `HEAD` worktree showed the manifest as the
    only difference, so the six workflows' files are byte-identical.
    `nix run .#pkl-check-generated` passed (71 files, inventory sha256
    `1b0c9a36…3676740`). Negative probe: renaming the manifest's `commandSlug`
    field made `pkl eval config/pkl/renderers/generation-contract-check.pkl` fail
    with `generated optional-workflow manifest does not match the catalog's
    optional workflows`, proving the check asserts rather than permits.
  - Deviations: none. `config/pkl/check-generated.sh`'s `required_paths` list was
    left unchanged; the manifest's presence and content are enforced by the Pkl
    contract check that the script already runs.

- [x] T02: `Embed the optional-workflow catalog into the CLI` (status:done)
  - Task ID: T02
  - Goal: `cli/build.rs` reads the generated manifest and emits a Rust catalog into `OUT_DIR`, and setup exposes it plus a selection-aware embedded-asset iterator.
  - Boundaries (in/out of scope): In — `cli/build.rs` manifest read and codegen, an `OptionalWorkflow` type and `OPTIONAL_WORKFLOWS` catalog in `cli/src/services/setup/`, per-target asset-membership resolution, and a selection-filtered variant of `iter_embedded_assets_for_setup_target`. Out — the install path, the prompt, CLI flags, config, and doctor; existing call sites keep using the unfiltered iterator in this task.
  - Dependencies: T01
  - Done when: the CLI compiles with a non-empty embedded optional-workflow catalog, and unit tests prove that filtering with an empty selection excludes exactly the brownfield command and skill assets for each of OpenCode, Claude, and Pi, and that filtering with `brownfield` selected yields the same asset set as today.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml optional_workflow`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml`.
  - Implementation: `cli/build.rs` gained `generate_optional_workflow_catalog`,
    which reads the staged `pkl-generated/config/optional-workflows.json`,
    rejects a manifest whose `schemaVersion` is not 1 or whose entries lack a
    non-empty string field, and emits `optional_workflows.rs` into `OUT_DIR` as
    `pub static OPTIONAL_WORKFLOWS: &[OptionalWorkflow]`. The manifest path was
    added to `validate_staged_artifacts`' required-path list, and `serde_json`
    was added to `[build-dependencies]`. In `cli/src/services/setup/mod.rs`: the
    `OptionalWorkflow` type plus an `include!` of the generated catalog; a
    `WorkflowAssetLayout` per concrete target built from the existing
    `default_paths` command/skill directory constants; a
    `asset_belongs_to_optional_workflow` predicate matching
    `{command_dir}/{command_slug}.md` and the `{skills_dir}/{skill_slug}/`
    prefix; and `iter_embedded_assets_for_setup_target_with_selection`, which
    filters each concrete target's slice by the unselected optional workflows.
    The unfiltered `iter_embedded_assets_for_setup_target` is unchanged, now
    sharing a new `embedded_assets_for_concrete_target` helper, so every existing
    call site behaves as before.
  - Verification: five unit tests were written and run against this
    implementation —
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    optional_workflow` passed (5 tests): the catalog is non-empty and carries
    brownfield's catalog slugs; an empty selection excludes exactly
    `command/brownfield.md`, `commands/brownfield.md`, `prompts/brownfield.md`
    and the `skills/sce-brownfield/` subtree for OpenCode, Claude, and Pi with no
    unrelated asset dropped; a full selection yields byte-identical asset vectors
    to the unfiltered iterator for all four targets; `All` filtering equals the
    sum of its concrete targets; membership does not match core workflow assets
    or a foreign target's command directory. Those tests were then removed at the
    user's explicit request (see Deviations), so that evidence is not reproducible
    from the current tree. Post-removal, `./scripts/run-cli-cargo.sh test
    --manifest-path cli/Cargo.toml` passed (180 tests),
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets` was clean, and `./scripts/run-cli-cargo.sh fmt
    --manifest-path cli/Cargo.toml -- --check` was clean.
  - Deviations: **the done check requiring unit tests is not satisfied by the
    committed tree.** The five tests that proved the filtering behavior were
    deleted on explicit user instruction after they passed, so no standing
    regression guard covers optional-workflow asset filtering. T04–T06 will
    exercise the seam through their own tests; if a durable guard is wanted, it
    must be re-added deliberately.
    `serde_json` was added to `[build-dependencies]` to parse the
    manifest. It is already a runtime dependency, so `Cargo.lock` is unchanged
    and no new crate enters the vendored set. `OptionalWorkflow` and the filtered
    iterator carry `#[allow(dead_code)]` until T04–T06 consume them, matching the
    existing `get_required_hook_asset` precedent in the same module.

- [x] T03: `Add integrations.optional_workflows to the config contract` (status:done)
  - Task ID: T03
  - Goal: `.sce/config.json` accepts `integrations.optional_workflows` as a unique string array constrained to the catalog's optional workflow ids, and the CLI parses it into typed config.
  - Boundaries (in/out of scope): In — `config/pkl/base/sce-config-schema.pkl` (property plus enum derived from the catalog), `cli/src/services/config/types.rs` `IntegrationsConfig`, and `cli/src/services/config/schema.rs` parsing/mapping. Out — setup writing the key, doctor reading it, and `sce config show` output.
  - Dependencies: T01
  - Done when: a config file with `integrations.optional_workflows: ["brownfield"]` passes `sce config validate`, an unknown id fails with a schema error naming the key, and the parsed value is reachable from the typed config layer.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::`; `./scripts/run-cli-cargo.sh run -- config validate --config <fixture>` for both fixtures; `nix run .#pkl-check-generated`.
  - Implementation: `config/pkl/base/sce-config-schema.pkl` imports
    `workflow-catalog.pkl`, derives `optionalWorkflowIds` by filtering the catalog
    for `optional`, and adds `integrations.optional_workflows` as a described
    `uniqueItems` string array whose items enumerate those ids, so the enum
    follows the catalog rather than a hand-written list. In Rust,
    `cli/src/services/config/types.rs` gained
    `parse_optional_workflow_id(raw, source)`, which validates an id against the
    build-generated `OPTIONAL_WORKFLOWS` catalog and reports the available slugs
    (no per-workflow match arms), plus
    `IntegrationsConfig.optional_workflows: Vec<String>`.
    `cli/src/services/config/schema.rs` added
    `ParsedIntegrationsConfigDocument.optional_workflows`, extended the
    `integrations` allowed-key list and its description to `target,
    optional_workflows`, and reworked `map_integrations_config` so either key
    alone yields a parsed `IntegrationsConfig` (the other defaulting to empty)
    while both absent still yields `None`.
    `validate_config_value_against_schema` now prefixes each schema error with
    its JSON-pointer instance path, so a bad value names the offending key.
  - Verification: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
    passed (180 tests, unchanged from the pre-task baseline);
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets` and `fmt -- --check` were clean. Fixture runs of
    `./scripts/run-cli-cargo.sh run --manifest-path cli/Cargo.toml -- config
    validate --config <fixture>`:
    `{"integrations":{"target":["claude"],"optional_workflows":["brownfield"]}}`
    reported `valid` (exit 0); `{"integrations":{"optional_workflows":["nonesuch"]}}`
    exited 4 with `/integrations/optional_workflows/0: "nonesuch" is not one of
    "brownfield"`; a duplicated id exited 4 with `/integrations/optional_workflows:
    ... has non-unique elements`. `nix run .#pkl-check-generated` passed (71 files,
    inventory sha256 `321a27e5…7291364f`; the hash moved from T01's because the
    generated config schema now carries the new property).
  - Deviations: unit tests covering the new parsing were written and passed
    during implementation, then removed at the user's explicit request, matching
    T02's precedent. The three done checks are evidenced by the fixture CLI runs
    above rather than by a standing regression guard, so nothing in the committed
    tree guards `integrations.optional_workflows` parsing. Schema-validation error
    text repository-wide now carries a JSON-pointer prefix; this was required by
    the done check that an unknown id fail with an error naming the key, and no
    existing assertion or snapshot depended on the previous unprefixed text.

- [x] T04: `Resolve, install, and persist optional workflows non-interactively` (status:done)
  - Task ID: T04
  - Goal: `sce setup --workflow <slug>` (repeatable) resolves a selection, target asset installation is filtered by it, and the resolved selection is persisted to `integrations.optional_workflows`.
  - Boundaries (in/out of scope): In — the setup clap surface and `SetupCliOptions`/`SetupRequest`, slug validation against the embedded catalog with an actionable unknown-slug error, the flag-over-persisted-config precedence rule, filtered installation in `run_setup_for_mode`, and extending the existing persistence path to write the selection. Out — the interactive prompt (T05) and doctor (T06).
  - Dependencies: T02, T03
  - Done when: AC1, AC2, AC3, and AC4 hold via the non-interactive path; `--workflow` with `--bootstrap-context` or with a hooks-only run is rejected with an actionable message; and existing setup tests still pass.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; manual runs of AC1–AC4 in a temp git repo.
  - Implementation: `cli/src/cli_schema.rs` gained a repeatable
    `--workflow <SLUG>` on `setup`, threaded through
    `cli/src/services/parse/command_runtime.rs` into
    `SetupCliOptions.workflows`. `resolve_setup_request` validates the slugs
    against the build-generated `OPTIONAL_WORKFLOWS` (dedup, order-preserving)
    via a new `validate_optional_workflow_slugs`, whose error lists every
    available slug with no per-workflow match arm, and rejects `--workflow`
    alongside `--bootstrap-context` and on a hooks-only run. The result lands in
    `SetupRequest.optional_workflows: Option<Vec<String>>`, where `None` means
    the flag was absent. `run_setup_for_mode` now takes that selection: `Some`
    is the exact selection for the run, `None` reuses the persisted
    `integrations.optional_workflows` read through the existing
    `parse_file_config` path (new `persisted_optional_workflows` helper).
    Installation switched to
    `iter_embedded_assets_for_setup_target_with_selection`, plumbed through
    `install_embedded_setup_assets(_with_rename)`, and
    `persist_integration_targets` now writes the resolved selection alongside
    `integrations.target`. `cli/src/command_surface.rs` documents an example.
  - Verification: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
    passed (180 tests), `clippy --all-targets` and `fmt -- --check` clean.
    Manual runs in a scratch git repo (with an `origin` remote, required by the
    unrelated agent-trace identity check) against the built binary:
    AC1 `setup --claude --non-interactive` left no `commands/brownfield.md` and
    no `skills/sce-brownfield/` while `commands/validate.md` was present, and
    recorded `{"optional_workflows":[],"target":["claude"]}`;
    AC2 `--workflow brownfield` installed both the command and
    `skills/sce-brownfield/SKILL.md` and recorded
    `{"optional_workflows":["brownfield"],"target":["claude"]}`;
    AC3 a repeat run without `--workflow` kept both the files and the recorded
    selection; AC4 `--workflow nonesuch` exited 3 with `Unknown optional
    workflow 'nonesuch' for '--workflow'. Available workflows: brownfield.` and,
    in a fresh repo, wrote nothing but `.git/`. `sce config validate` reported
    the resulting config valid. `--bootstrap-context --workflow brownfield` and
    `--hooks --workflow brownfield` were both rejected with actionable messages.
    A `--all` spot check installed brownfield into `.opencode/`, `.claude/`, and
    `.pi/` and preserved it across a repeat run.
  - Deviations: unit tests covering slug validation, the compatibility
    rejections, flag/persisted precedence, and filtered installation were written
    and passed (24 setup tests), then removed at the user's explicit request,
    matching T02 and T03. The done checks are evidenced by the manual scratch-repo
    runs above rather than by a standing regression guard, so nothing in the
    committed tree guards `--workflow` parsing, selection precedence, or filtered
    installation. `cli/src/services/command_registry.rs` needed the new
    `SetupRequest` field in its bare-`setup` construction.

- [x] T05: `Add the interactive optional-workflow multi-select` (status:done)
  - Task ID: T05
  - Goal: interactive `sce setup` prompts for optional workflows after target selection, unchecked by default and pre-checked from the persisted selection, and skips the prompt when the catalog is empty.
  - Boundaries (in/out of scope): In — the `prompt` module's `inquire::MultiSelect` prompt, its title/row labels using the shared styling policy, catalog-driven row construction, cancellation and `NotTTY` handling matching the existing target prompt, and threading the result into the resolved request. Out — flag parsing, persistence mechanics, and doctor.
  - Dependencies: T04
  - Done when: AC5 holds; cancelling the optional-workflow prompt returns the existing cancelled outcome with no files changed; and a non-TTY interactive run still fails with the existing actionable guidance.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::prompt`; one manual TTY run of `sce setup` selecting nothing and then selecting brownfield.
  - Implementation: `SetupDispatch::Proceed` became a struct variant carrying
    `optional_workflows: Option<Vec<String>>` (dropping `Copy`), and
    `SetupTargetPrompter` gained `prompt_optional_workflows(defaults)` returning
    `None` for a cancelled prompt. `resolve_setup_dispatch` now takes the
    pre-check defaults and, in interactive mode only, runs the workflow prompt
    after the target prompt, mapping either cancellation to
    `SetupDispatch::Cancelled`. The `prompt` module gained
    `prompt_optional_workflows`, an `inquire::MultiSelect` over
    `optional_workflow_prompt_inputs`, which returns `None` when the catalog is
    empty (skipping the prompt) and otherwise catalog-ordered rows plus the
    pre-checked indices for the supplied ids, ignoring ids absent from the
    catalog. Rows are labelled `{title} — {description}` through
    `prompt_value_with_color_policy`, the title through `prompt_label`, and
    `NotTTY` bails with the existing guidance extended with `--workflow <slug>`.
    `cli/src/services/setup/command.rs` now resolves the repository root before
    dispatch so the prompt can be seeded from
    `setup::persisted_optional_workflows` (made `pub`), falling back to a
    supplied `--workflow` selection, and passes the prompted selection to
    `run_setup_for_mode`, where it takes precedence over the request's flag
    selection. `OptionalWorkflow` no longer needs `#[allow(dead_code)]`.
  - Verification: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
    passed (180 tests, the pre-task baseline), `clippy --all-targets` and
    `fmt -- --check` clean. Manual TTY runs against the built binary in scratch
    git repos, driven through `script(1)` with recorded keystrokes: selecting
    Claude then confirming an empty selection rendered `Select optional
    workflows` with `[ ] SCE Brownfield — …` after the target prompt, installed
    19 files with no `commands/brownfield.md` or `skills/sce-brownfield/`, and
    recorded `{"optional_workflows":[],"target":["claude"]}`; checking the row
    with space installed both and recorded
    `{"optional_workflows":["brownfield"],"target":["claude"]}`; a repeat
    interactive run in that repo rendered the row as `[x] SCE Brownfield` and
    preserved both the files and the recorded selection. Pressing Esc at the
    optional-workflow prompt printed `Setup cancelled. No files were changed.`
    and left the repository containing only `.git/`. A non-TTY `sce setup`
    exited 4 with the unchanged `Interactive setup requires a TTY. …` guidance.
  - Deviations: six unit tests over row/default construction (empty catalog,
    no persisted selection, persisted selection, unknown persisted id, the real
    catalog, and the row label) were written and passed under
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    setup::prompt`, then removed at the user's explicit request, matching
    T02–T04. **AC5's named unit tests are therefore not present in the committed
    tree**; the three AC5 cases are evidenced by the TTY runs above except the
    empty-catalog skip, which the real catalog cannot exercise and which now
    rests only on the `optional_workflow_prompt_inputs` early return. An
    interactive run invoked with `--workflow <slug>` seeds the prompt from those
    slugs rather than the persisted selection; the prompt answer stays
    authoritative for the run. Resolving the repository root before the prompt
    means a non-git directory now errors before the target prompt instead of
    after it; no files are written on either path.

- [x] T06: `Scope doctor integration checks to the selected optional workflows` (status:done)
  - Task ID: T06
  - Goal: `sce doctor` expects an optional workflow's assets only when the repo-local config records it as selected.
  - Boundaries (in/out of scope): In — `cli/src/services/doctor/inspect.rs` resolution of the selected set from `.sce/config.json` and filtering of OpenCode/Claude/Pi integration children before health inspection. Out — the doctor text layout, new problem kinds, `--fix` behavior, and reporting stray files for deselected workflows.
  - Dependencies: T04
  - Done when: AC6 holds — no missing-file problem or integration row is produced for brownfield when it is unselected, and drift/missing detection for brownfield is unchanged when it is selected.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`; `./scripts/run-cli-cargo.sh run -- doctor` in the AC1 and AC2 repos.
  - Implementation: `inspect_repository_integrations` in
    `cli/src/services/doctor/inspect.rs` resolves the selected optional workflows
    once per run through the existing `setup::persisted_optional_workflows`
    (repo-local `.sce/config.json`; an absent, unreadable, or key-less config
    means nothing is selected) and threads that selection into
    `collect_opencode_integration_groups`, `collect_claude_integration_groups`,
    and `collect_pi_integration_groups`, which now build their embedded-asset
    lists with `iter_embedded_assets_for_setup_target_with_selection`. Grouping,
    sorting, and every `inspect_*_integration_health` problem push are unchanged:
    an unselected workflow simply contributes no children, so it can produce
    neither a row nor a missing-file problem. Doctor was the last production
    consumer of the unfiltered iterator, so `iter_embedded_assets_for_setup_target`
    and `EmbeddedAssetSelectionIter` were deleted from
    `cli/src/services/setup/mod.rs` (dead code fails the repo's clippy gate) and
    the two setup tests that used them now call the filtered iterator with the
    full catalog selection.
  - Verification: three unit tests added in `cli/src/services/doctor/inspect.rs`
    cover integration-child collection with and without `brownfield` selected —
    an empty selection drops exactly the brownfield command and skill children
    for OpenCode, Claude, and Pi while keeping core-workflow children; a
    `["brownfield"]` selection restores them; and, against a repository root with
    no assets on disk, an empty selection produces no brownfield problem while
    core missing-file problems still fire, and a `["brownfield"]` selection
    reports `commands/brownfield.md` as `Missing`.
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
    passed (3 tests); the full suite passed (183 tests, 180 baseline plus these
    three); `clippy --all-targets` and `fmt -- --check` clean. Manual runs
    against the built binary in scratch git repos: in the AC1 repo
    (`setup --claude --non-interactive`) `sce doctor` printed zero brownfield
    rows and all integration groups `[PASS]`, its 3 blocking problems being the
    unrelated missing git hooks; in the AC2 repo (`--workflow brownfield`)
    doctor listed the brownfield command and skill rows, and after appending a
    byte to `commands/brownfield.md` and deleting
    `skills/sce-brownfield/SKILL.md` reported them as `content mismatch` and
    `[MISS]`, so drift and missing detection are unchanged for a selected
    workflow.
  - Deviations: this repository's own `.sce/config.json` records no
    `integrations.optional_workflows`, so `sce doctor` here no longer inspects
    the committed `/brownfield` mirrors in `.claude/`, `.opencode/`, and `.pi/`
    (doctor reports 0 problems). Recording `["brownfield"]` in the repo's config
    would restore that coverage; it was left alone as outside this task's
    boundary. Deleting the now-unused unfiltered asset iterator touched
    `cli/src/services/setup/mod.rs`, one file beyond the task's declared
    in-scope list, because the repo's clippy gate rejects dead code.

## Open questions

- The generated OpenCode Code agent keeps `"sce-brownfield": allow` in its skill
  permission list regardless of installation, so an uninstalled repository ships
  a permission for a skill that is not present. It is inert, and conditioning the
  permission list on install-time selection would push a runtime concern back
  into generation. Left as-is; say so if you would rather the list be filtered.
- Nothing here changes this repository's own committed `.opencode/`, `.claude/`,
  and `.pi/` mirrors, which continue to carry `/brownfield`. That is consistent
  with the repo dogfooding the workflow, but it does mean the default-off
  behavior is not exercised by the repo's own tree — only by the tests and
  manual scratch-repo runs named in the acceptance criteria.

## Validation Report

**Status:** validated  
**Date:** 2026-07-31

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (71 files, inventory sha256 `321a27e5…7291364f`)
- `nix flake check` -> exit 0 (5 flake checks: pkl-generated-check, cli clippy, cli fmt, cli generated-input-check, cli tests — all passed)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` -> exit 0 (3 tests passed)
- `sce setup --claude --non-interactive` (temp git repo) -> exit 0 (19 files, no brownfield assets)
- `sce setup --claude --non-interactive --workflow brownfield` (temp git repo) -> exit 0 (brownfield command + skill installed)
- `sce setup --claude --non-interactive` (repeat in the same repo) -> exit 0 (selection preserved)
- `sce setup --claude --non-interactive --workflow nonesuch` -> exit 3 (validation error, nothing written)
- `sce doctor` (AC1 repo) -> exit 0 (zero brownfield rows)
- `sce doctor` (AC2 repo) -> exit 0 (brownfield command and skill rows present)
- `sce config validate --config <valid fixture>` -> exit 0 (`valid`)
- `sce config validate --config <unknown-id fixture>` -> exit 4 (`/integrations/optional_workflows/0: "nonesuch" is not one of "brownfield"`)
- `script(1)`-driven TTY `sce setup` (first run) -> exit 0 (prompt rendered `[ ] SCE Brownfield`, nothing installed)
- `script(1)`-driven TTY `sce setup` (repo with persisted selection) -> exit 0 (prompt rendered `[x] SCE Brownfield`; Esc printed `Setup cancelled. No files were changed.`)

Note: AC5's unit-test requirement was dropped by the user during this validation
run; see **Residual risks**.

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: A default setup run installs no brownfield assets -> scratch repo `setup --claude --non-interactive` left no `.claude/commands/brownfield.md` and no `.claude/skills/sce-brownfield`, with `.claude/commands/validate.md` present; config recorded `"optional_workflows": []`.
- [x] AC2: An explicit non-interactive opt-in installs brownfield and records the selection -> `--workflow brownfield` wrote `.claude/commands/brownfield.md` and `.claude/skills/sce-brownfield/SKILL.md`; `.sce/config.json` held `integrations.optional_workflows == ["brownfield"]`.
- [x] AC3: A repeat run without `--workflow` preserves the recorded selection -> repeat `setup --claude --non-interactive` kept `skills/sce-brownfield/SKILL.md` and the recorded selection.
- [x] AC4: An unknown slug is rejected with an actionable error and writes nothing -> exit 3 with `Unknown optional workflow 'nonesuch' for '--workflow'. Available workflows: brownfield.`; the fresh repo contained only `.git`.
- [x] AC5: Interactive setup presents an optional-workflow multi-select -> manual TTY runs passed: the prompt renders after target selection, `[ ] SCE Brownfield` on a first run with nothing installed on an empty confirmation, `[x] SCE Brownfield` when `integrations.optional_workflows` holds it, and Esc prints `Setup cancelled. No files were changed.` with the repository untouched. The criterion's original unit-test requirement was dropped by the user during this validation; the empty-catalog skip is accepted on inspection of the `catalog.is_empty()` early return in `optional_workflow_prompt_inputs` (`cli/src/services/setup/mod.rs:1508`) rather than proven by a check.
- [x] AC6: `sce doctor` scopes integration checks to the selection -> the three unit tests in `cli/src/services/doctor/inspect.rs` passed; `sce doctor` in the AC1 repo printed zero brownfield rows, and in the AC2 repo listed the brownfield command and skill rows.
- [x] AC7: `integrations.optional_workflows` validates, and an unknown id fails with a schema error -> both fixture runs of `sce config validate` behaved as specified, the failure naming the key by JSON pointer.
- [x] AC8: Generation still emits all six workflows for all three targets plus the manifest, asserted rather than permitted -> `nix run .#pkl-check-generated` passed against the exact 71-path inventory, and `config/pkl/renderers/generation-contract-check.pkl:328` `assertOptionalWorkflowManifest` errors unless the manifest carries `schemaVersion: 1`, at least one optional workflow, each optional workflow's catalog title and both slugs, and no core workflow id.

### Failed checks and follow-ups

- None.

### Residual risks

- AC5's original `Validate:` line named unit tests over prompt row/default construction covering the empty-catalog, no-persisted-selection, and persisted-selection cases. Those tests are absent from the tree (written and deleted during T05 at the user's request). On 2026-07-31 the user directed that the unit-test requirement be dropped and validation finished on the manual TTY evidence, so the criterion was amended accordingly. The empty-catalog skip is consequently unverified by any check: it rests on the `catalog.is_empty()` early return at `cli/src/services/setup/mod.rs:1508`, which the real non-empty catalog can never reach.

- T02, T03, T04, and T05 each deleted the tests that proved their behavior, so nothing in the committed tree guards optional-workflow asset filtering, `integrations.optional_workflows` parsing, `--workflow` slug validation, selection precedence, or prompt construction. Only the three T06 doctor tests remain as a standing regression guard on the seam.
- This repository's own `.sce/config.json` records no `integrations.optional_workflows`, so `sce doctor` here no longer inspects the committed `/brownfield` mirrors in `.claude/`, `.opencode/`, and `.pi/`.
- The generated OpenCode Code agent still lists `"sce-brownfield": allow` whether or not the workflow is installed (recorded as an open question, deliberately out of scope).
- Unrelated to this plan: `format_setup_success_message` in `cli/src/services/setup/mod.rs:538` emits doubled quotes and a doubled colon (`- Claude:: installed 19 file(s) to ' '<path>''`). The line is untouched by this plan's diff.
