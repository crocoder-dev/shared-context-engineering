# Plan: sce-plan-init

## Change summary

Add `sce plan init <name>`, a deterministic, offline scaffold generator that
creates `context/plans/<name>.md` at the Git repository root. It extends the
existing read-only `sce plan check` service in `cli/src/services/plan/` (PR #342,
HEAD `7c6dde7c`) rather than adding a parallel service.

The scaffold is a new compact generated artifact, `config/plan-init-scaffold.md`,
rendered by the existing Pkl pipeline from the canonical plan format in
`config/pkl/base/workflow-change-to-plan.pkl`. It travels through the generated
payload handoff into Cargo `OUT_DIR`, and the packaged fallback carries it for
Pkl-free builds. The binary embeds it with `include_str!`, so the binary never
runs Pkl, Nix, an LLM, or the network.

The untouched scaffold fails `sce plan check`. Its `Scenarios: TODO` and
`Covers: TODO` placeholders are not valid ID lists, so it cannot pass for an
authored plan. The `sce-change-to-plan` workflow then uses `init` to create new
plans, fills the scaffold, and runs `check` before it reports `plan_ready`.
Revisions of existing plans skip `init`. Older binaries fall back to manual
template authoring.

## Design decisions

- D1: The scaffold has its own Pkl renderer built from fragments shared with the canonical plan template. A generation contract pins it to the template's skeleton.
  - Boundary: `config/pkl/base/workflow-change-to-plan.pkl` owns the plan format. A new `config/pkl/base/plan-init-scaffold.pkl` renders only the compact scaffold, using section fragments shared with `changeToPlanPlanTemplate`, such as the task context synchronization lifecycle text. `config/pkl/renderers/plan-structure.pkl` exposes its existing skeleton mode as a public function so the contract can check the scaffold.
  - Rationale: The two documents share verbatim text through one source. The contract checks the parts that must agree: the skeleton heading list (`templateSkeletonHeadings`), the required fields for each kind, and the exact `check` defects of the untouched scaffold. The scaffold can leave out the authoring rules and examples without becoming a second template.
  - Rejected: A Rust string template — it would be a second hand-maintained format outside the Pkl contract. Text-transforming the fenced `## Template` block — placeholders span several lines and carry authoring prose, so a transform would be fragile and hard to read. Emitting the full `plan-template.md` — it carries rules and filled-in examples that a plan file must not contain.
  - Trade-offs: Scaffold prose such as TODO wording is authored separately from template prose. Drift between them is caught only at the heading, field, and defect level, not word for word.
- D2: The scaffold ships as a required generated-payload file and is embedded with `include_str!`. It is not a setup install asset.
  - Boundary: `config/pkl/generate.pkl` emits `config/plan-init-scaffold.md` into the payload. `cli/build.rs` `validate_staged_artifacts` requires `pkl-generated/config/plan-init-scaffold.md`. `cli/src/services/plan/` includes it from `OUT_DIR`. `scripts/prepare-cli-generated-assets.sh` already moves the whole `pkl-generated` tree into the packaged fallback.
  - Rationale: The same payload serves repository builds, `nix build .#default`, and the crates.io and Flatpak fallback. `include_str!` makes a missing scaffold a compile-time failure.
  - Rejected: Adding it to `TARGETS` / `EmbeddedAsset` — that catalog feeds `sce setup` installs and would put the scaffold into harness trees. A committed copy under `config/` — this repository forbids committed generated output.
  - Trade-offs: A fallback package prepared before this change no longer builds. That is intended, and the remedy is the existing prepare script.
- D3: `init` and `check` are variants of one native Rust enum inside the existing plan command, dispatched statically.
  - Boundary: `cli_schema::PlanSubcommand` gains `Init { name }`. `services::parse::command_runtime::convert_plan_subcommand` maps each subcommand to a `PlanRequest::{Check, Init}` value. `PlanCommand::execute` matches on it. Init logic lives in a new `cli/src/services/plan/init.rs`, and `RuntimeCommand::Plan` stays the only registry entry.
  - Rationale: This reuses the existing `plan` command name, help, and completion wiring. It also keeps the command synchronous, with no database or repository context.
  - Rejected: A new top-level `sce init`-style command — it would split plan ownership. A trait or `dyn` operation object — the repository constraints rule these out, and two variants do not need them.
  - Trade-offs: `PlanCheckRequest` moves behind an enum, so the existing `plan::command_*` tests change their destructuring.
- D4: The plan name is validated before any filesystem access, and the scaffold is published with a no-clobber hard link from a fully written temporary file.
  - Boundary: Names must match `[a-z0-9]+(-[a-z0-9]+)*`, at most 64 bytes, with no `.md` suffix. The repository root comes from `git rev-parse --show-toplevel`, run from the current directory. `context` and `context/plans` must be real directories when they exist; they are created when absent. The content is written to a `create_new` temporary file inside `context/plans/`, then `std::fs::hard_link` publishes it to `<name>.md`, and the temporary file is removed on every path.
  - Rationale: `link` fails on any existing entry, including a dangling or conflicting symlink, so a plan is never overwritten. A successful exit only ever exposes complete content, and a crash cannot leave a half-written plan that later blocks `init`.
  - Rejected: `rename` — it silently replaces an existing destination. Writing directly with `create_new` — an interrupted write leaves a partial plan at the final path. Accepting a path argument — that would invite traversal and repository escape.
  - Trade-offs: This requires a filesystem that supports hard links inside `context/plans/`. On one that does not, `init` fails with a runtime error and has no non-atomic fallback. Symlinked `context` or `context/plans` directories are refused.
- D5: `init` reports results through the existing `CliError` classes and prints exactly three lines on stdout.
  - Boundary: On success, stdout is `Created plan: context/plans/<name>.md`, `Next: complete the plan and run`, and `sce plan check context/plans/<name>.md`, and the exit code is `0`. An invalid name or an existing destination is `CliError::validation` (exit `3`). Running outside a Git repository, a failed or missing `git`, or a filesystem failure is `CliError::runtime` (exit `4`). Clap usage errors keep the existing parse class (exit `2`).
  - Rationale: Automation and the workflow can branch on the class without parsing text. The printed path is always repository-relative, whichever directory `init` ran from.
  - Rejected: A dedicated new exit code or error class — the existing taxonomy already covers these failures.
  - Trade-offs: A destination that already exists and an invalid name share exit `3`, so callers that need to tell them apart must read stderr.
- D6: The workflow runs `init` only when writing a new plan, and it confirms the file exists rather than trusting the exit code.
  - Boundary: In `changeToPlanPlanAuthoring`, step 2.10 runs `sce plan init {plan_name}` for a new plan, after the clarification gate and target resolution. Initialization counts as successful only when stdout contains `Created plan: context/plans/{plan_name}.md` and that file exists. `init` counts as unavailable when it exits `0` without that line (an older binary printing `plan` help), exits `2`, or the command is not found. Only then does the workflow fall back to writing from `references/plan-template.md`. Exit `3` or `4` returns `blocked` with the reported error and no fallback. Revisions skip `init`.
  - Rationale: An older `sce` (verified with `0.4.0-pre-alpha-v11`) prints `plan` help and exits `0` for `sce plan init foo`, so the exit code alone would report success with no file written.
  - Rejected: Running `init` during 2.1 target resolution — it would write a file before the clarification gate. Falling back on any non-zero exit — it would hide real filesystem failures and collisions.
  - Trade-offs: The workflow now depends on exact stdout wording, so changing D5's first line is a breaking change for the workflow.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `sce plan init example`, run inside a Git repository, creates `context/plans/example.md` holding the embedded scaffold with the title `# Plan: example`, creates `context/plans/` when it is missing, prints the three-line success output, and modifies no other repository file.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D3, D4, D5
  - Scenarios: S1
- [ ] AC2: Existing destinations, including symlinks, are never overwritten, and unsafe names or a missing repository are rejected with the documented class before any file or directory is created.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D4, D5
  - Scenarios: S3, S4, S5
- [ ] AC3: Running `init` from a nested directory of a Git repository creates the same repository-root-relative `context/plans/<name>.md` and prints the same root-relative paths.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D4
  - Scenarios: S2
- [ ] AC4: The untouched scaffold fails `sce plan check` with exactly the incomplete-reference defects, and the same scaffold with its reference placeholders filled correctly passes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::scaffold`
  - Decisions: D1
  - Scenarios: S6, S7
- [ ] AC5: The scaffold is generated from the canonical Pkl sources and pinned to the template skeleton by the generation contract. It is embedded in the binary, it is present in the packaged fallback, and it is never committed to the repository.
  - Validate: `nix run .#pkl-check-generated`
  - Decisions: D1, D2
  - Scenarios: S8, S9
- [ ] AC6: The generated `sce-change-to-plan` workflow runs `sce plan init` for new plans only, after its clarification and target-resolution gates. It verifies initialization by output and file existence, falls back to manual template authoring only when `init` is unavailable, skips `init` for revisions, and runs `sce plan check` before `plan_ready`.
  - Validate: `nix run .#pkl-check-generated`
  - Decisions: D6
  - Scenarios: S10, S11
- [ ] AC7: `sce --help`, `sce plan --help`, and the generated Bash, Zsh, and Fish completion scripts include `plan init`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_init`
  - Decisions: D3
  - Scenarios: S12

### Verification scenarios

- S1: Fresh initialization at the repository root
  - Given: A temporary `git init` repository with no `context/` directory, the current directory at its root.
  - When: `sce plan init example` runs.
  - Then: Exit `0`. Stdout is exactly the three D5 lines for `context/plans/example.md`. That file equals the embedded scaffold with its single `{plan-name}` token replaced by `example`. `git status --porcelain` lists only `context/`.
  - Check: `plan::init_creates_scaffold_at_repository_root`, created by T02.
- S2: Nested invocation resolves the repository root
  - Given: A temporary Git repository with the current directory at `<root>/a/b`.
  - When: `sce plan init nested-plan` runs.
  - Then: `<root>/context/plans/nested-plan.md` exists, nothing is created under `<root>/a/b`, and stdout names `context/plans/nested-plan.md`.
  - Check: `plan::init_from_nested_directory_targets_repository_root`, created by T02.
- S3: Existing entries are never overwritten
  - Given: A repository where `context/plans/taken.md` exists as a regular file with known bytes, and `context/plans/linked.md` is a dangling symlink.
  - When: `sce plan init taken` and `sce plan init linked` run.
  - Then: Both exit `3` (`SCE-ERR-VALIDATION`). `taken.md` keeps its bytes, `linked.md` is still the same symlink, and no temporary file remains in `context/plans/`.
  - Check: `plan::init_never_overwrites_existing_entry`, created by T02.
- S4: Unsafe names are rejected before any write
  - Given: A repository without `context/`.
  - When: `sce plan init` runs with each of `../escape`, `a/b`, `Upper`, `trailing-`, `-leading`, `double--hyphen`, `plan.md`, an empty string, and a 65-byte name.
  - Then: Each exits `3`, and `context/` still does not exist afterwards.
  - Check: `plan::init_rejects_unsafe_names_without_writing`, created by T02.
- S5: Outside a Git repository
  - Given: A temporary directory that is not inside any Git repository.
  - When: `sce plan init example` runs there.
  - Then: Exit `4` (`SCE-ERR-RUNTIME`), and the directory is left unchanged.
  - Check: `plan::init_outside_repository_is_runtime_failure`, created by T02.
- S6: The untouched scaffold is not a ready plan
  - Given: The embedded scaffold with its title token replaced.
  - When: It is extracted and validated by the plan service.
  - Then: The diagnostics are exactly ``Scenarios:` on `AC1` is not a comma-separated ID list``, ``AC1` is not covered by any task's `Covers:``, and ``Covers:` on `T01` is not a comma-separated ID list``. The scaffold defines no `D` or `S` entry.
  - Check: `plan::scaffold_untouched_fails_check`, created by T01.
- S7: A correctly authored scaffold passes
  - Given: A scaffold created by `sce plan init authored` in which `Scenarios: TODO` is replaced by `Scenarios: none (inspection)` and `Covers: TODO` by `Covers: AC1`.
  - When: `sce plan check context/plans/authored.md` runs.
  - Then: Exit `0`, with stdout `structured plan: reference integrity holds`.
  - Check: `plan::scaffold_authored_passes_check`, created by T02.
- S8: The generation contract pins the scaffold
  - Given: The Pkl sources after T01.
  - When: Generation checks run.
  - Then: `output.files` contains `config/plan-init-scaffold.md`, and the artifact count is one higher than before. The scaffold passes skeleton mode, fails `check` with exactly the S6 defects, and contains `{plan-name}` exactly once. No repository-local `config/plan-init-scaffold.md` exists.
  - Check: `nix run .#pkl-check-generated`, with the `plan-init-scaffold` contract assertion created by T01.
- S9: The packaged fallback carries the scaffold
  - Given: A fallback staged by `scripts/prepare-cli-generated-assets.sh`.
  - When: The fallback test runs, and the packaged CLI is built.
  - Then: `pkl-generated/config/plan-init-scaffold.md` is in the fallback inventory, and `nix build .#default` succeeds without a runtime Pkl dependency.
  - Check: `bash scripts/test-prepare-cli-generated-assets.sh`, extended by T01, and `nix build .#default`.
- S10: New-plan authoring uses `init`, with a safe fallback
  - Given: The generated `sce-change-to-plan/references/plan-authoring.md`.
  - When: Its write step for a new plan is inspected.
  - Then: It runs `sce plan init {plan_name}` after the clarification gate. It requires both the `Created plan:` line and an existing file. It falls back to the template only for exit `0` without that line, exit `2`, or a missing command. It returns `blocked` on exit `3` or `4`. It then runs `sce plan check {plan-path}` before `plan_ready`.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion created by T03.
- S11: Revisions skip `init`
  - Given: The generated plan-authoring reference.
  - When: Its existing-plan revision path is inspected.
  - Then: It never runs `sce plan init`. It keeps the `Updating an existing plan` rules, which preserve completed tasks and evidence. No generated `sce-next-task` or `sce-validate` document mentions `sce plan init`.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion created by T03.
- S12: Help and completions expose `plan init`
  - Given: The built CLI schema.
  - When: Top-level help, `plan` help, and the Bash, Zsh, and Fish completion scripts are rendered.
  - Then: Each mentions `init`, and `sce plan init --help` shows the `<NAME>` argument.
  - Check: `plan::plan_init_appears_in_help` and `plan::completion_scripts_include_plan_init`, created by T02.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix build .#default`

### Context sync

- `context/cli/plan-service.md`: `init.rs`, the `PlanRequest` enum, the init command contract, and the new tests.
- `context/sce/plan-structure-contract.md`: the scaffold's relationship to skeleton mode and its expected `check` defects.
- `context/sce/config-generation.md`: the new generated artifact, the public skeleton function, the `plan-init-scaffold` and `plan-authoring-plan-init` contract checks, and the artifact count.
- `context/sce/shared-context-plan-workflow.md`: `init` in new-plan authoring, the older-binary fallback, and revisions skipping `init`.
- `context/cli/cli-command-surface.md` and `context/sce/cli-shell-completion-contract.md`: the `sce plan init <name>` surface.
- `context/glossary.md`: a `plan scaffold` term.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - `cli/src/services/plan/`, `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, and `cli/build.rs`.
  - `config/pkl/base/workflow-change-to-plan.pkl`, a new `config/pkl/base/plan-init-scaffold.pkl`, and `config/pkl/generate.pkl`.
  - `config/pkl/renderers/plan-structure.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/check-generated.sh`, and `scripts/test-prepare-cli-generated-assets.sh`.
  - The durable context files listed under Context sync.
- **Out of scope:**
  - Semantic plan approval, or any change to `sce plan check` rules or wording.
  - `/next-task` and `/validate` workflow content.
  - Committing generated target trees or the scaffold.
  - Validating real plans under `context/plans/` in Nix or generation checks.
- **Constraints:**
  - No new traits, `dyn`, boxed futures, async runtime, database, or crate dependency.
  - No interactive prompts.
  - `init` runs offline and invokes only `git`.
  - Output is deterministic.
  - All Cargo commands run through `scripts/run-cli-cargo.sh` under `nix develop`.
- **Non-goal:**
  - `init` does not author plan content, accept arbitrary paths, take `--force` or overwrite flags, or support template variants.

## Assumptions

- The 64-byte name limit and the `[a-z0-9]+(-[a-z0-9]+)*` grammar are local choices. They are consistent with the existing kebab-case plan names in `context/plans/`.
- Repository root resolution reuses the existing `git rev-parse --show-toplevel` pattern, as in `cli/src/services/bash_policy.rs`, rather than adding a new Git library.
- The scaffold keeps `## Design decisions` and `## Assumptions` as headed sections, each holding one TODO line that tells the author to delete the section when it has no real entries. It defines no `D`, `S`, or placeholder reference IDs, because skeleton mode requires every heading in `templateSkeletonHeadings`.

## Task stack

- [x] T01: `Generate and embed the plan init scaffold from canonical Pkl` (status:done)
  - Task ID: T01
  - Covers: AC4, AC5
  - Scope: In:
    - Shared plan-format fragments in `workflow-change-to-plan.pkl`, with no byte change to the generated `plan-template.md`.
    - The new `plan-init-scaffold.pkl` and the `generate.pkl` entry.
    - A public skeleton-check function in `plan-structure.pkl`.
    - The `plan-init-scaffold` assertion and the artifact count of 146 in `generation-contract-check.pkl`.
    - A required output and a forbidden committed path in `check-generated.sh`.
    - A required staged path in `cli/build.rs`.
    - A crate-visible `include_str!` constant in `cli/src/services/plan/`, with the `plan::scaffold_untouched_fails_check` test.
    - A fallback-presence assertion in `scripts/test-prepare-cli-generated-assets.sh`.

    Out: CLI command wiring and workflow prose.
  - Dependencies: none
  - Done when: The generated payload contains a compact scaffold with no authoring rules or examples. The contract pins its skeleton, its token, and its exact `check` defects. Both payload and fallback builds embed it, and the untouched-scaffold test passes.
  - Verify: `nix run .#pkl-check-generated` — passed (146 files; `plan-init-scaffold` contract assertion evaluates to its pass message); `bash scripts/test-prepare-cli-generated-assets.sh` — passed; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::scaffold_untouched_fails_check -- --exact` — ran 0 tests because the full test name is `services::plan::scaffold_untouched_fails_check`; rerun with that exact name — passed (1 test). Also: `plan::` module tests — 17 passed; `clippy --all-targets --all-features` via `run-cli-cargo.sh` — clean; `cargo fmt` applied; a real fallback staged into a scratch directory lists `pkl-generated/config/plan-init-scaffold.md` and is byte-identical to the payload scaffold; generated output diffed against HEAD differs only by the new scaffold, so `plan-template.md` is byte-identical.
  - Completed: 2026-10-24
  - Files changed: `cli/build.rs`, `cli/src/services/plan/mod.rs`, `config/pkl/base/plan-init-scaffold.pkl`, `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/check-generated.sh`, `config/pkl/generate.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/renderers/plan-structure.pkl`, `scripts/test-prepare-cli-generated-assets.sh`, `context/plans/sce-plan-init.md`
  - Result: `changeToPlanPlanTemplate` now interpolates shared `planAcceptanceCriteriaIntro`, `planFullValidationIntro`, and `planTaskContextSynchronizationLifecycle` fragments. The new `plan-init-scaffold.pkl` renders `config/plan-init-scaffold.md` from them. `plan-structure.pkl` exposes `skeletonDefects`. The `plan-init-scaffold` contract pins skeleton mode, the single `{plan-name}` token, no `D`/`S` entries, no rules or examples, and the exact three S6 defects. `check-generated.sh` requires the output and forbids a committed copy, and `build.rs` requires the staged path. `SCAFFOLD` and `SCAFFOLD_PLAN_NAME_TOKEN` are crate-visible `include_str!`/token constants carrying `#[cfg_attr(not(test), expect(dead_code))]` until T02 uses them. Deviation: the plan's `--exact` filter needs the `services::` prefix; T02's verify filters without `--exact` are unaffected. `nix build .#default` was not run here: it is `/validate` full validation, and flake builds do not see the untracked new `.pkl` until it is tracked.
  - Context impact: `context/sce/config-generation.md` (new generated artifact, public skeleton function, `plan-init-scaffold` contract, artifact count 146), `context/sce/plan-structure-contract.md` (scaffold and skeleton mode, expected `check` defects), `context/cli/plan-service.md` (embedded scaffold constant and test).
  - Context synchronization: synced

- [ ] T02: `Add the sce plan init command with safe repository-root creation` (status:todo)
  - Task ID: T02
  - Covers: AC1, AC2, AC3, AC4, AC7
  - Scope: In:
    - `PlanSubcommand::Init { name }` and its help text in `cli_schema.rs`.
    - The `PlanRequest` enum and its dispatch in `command.rs` and `command_runtime.rs`.
    - `cli/src/services/plan/init.rs`, covering name validation, root resolution, directory checks, and temporary-file plus hard-link publication.
    - The D5 output and error contract.
    - The S1–S5, S7, and S12 tests in `cli/src/services/plan/mod.rs`, plus updates to the existing `plan::command_*` tests.

    Out: Pkl sources, workflow prose, and changes to `check` behavior.
  - Dependencies: T01
  - Done when: `sce plan init <name>` behaves as D4 and D5 state from the repository root and from nested directories. All listed tests pass. Help and completions show `init`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Context synchronization: pending

- [ ] T03: `Use sce plan init in the change-to-plan workflow and document it` (status:todo)
  - Task ID: T03
  - Covers: AC6
  - Scope: In:
    - `changeToPlanPlanAuthoring` steps 2.1 and 2.10 in `workflow-change-to-plan.pkl`, adding the `init`-for-new-plans path, output-and-file verification, the narrow fallback, and the skip for revisions.
    - The `plan-authoring-plan-init` assertion and its negative fixture in `generation-contract-check.pkl`.
    - Any existing contract substring updates needed by the new wording.
    - `context/sce/shared-context-plan-workflow.md`.

    Out: `/next-task` and `/validate` content, the plan template schema, and CLI code.
  - Dependencies: T02
  - Done when: All four targets' generated `plan-authoring.md` carry the D6 behavior. The contract assertion passes on generated output and fails on its negative fixture. No `sce-next-task` or `sce-validate` document mentions `sce plan init`.
  - Verify: `nix run .#pkl-check-generated`.
  - Context synchronization: pending

## Open questions

- Should `init` refuse a symlinked `context/` or `context/plans/` directory, as D4 plans to? A repository that deliberately symlinks `context/` elsewhere would be blocked. Following the link instead would allow writes outside the repository root.
- Hard-link publication fails on filesystems without hard-link support, such as some network or FUSE mounts. Is a runtime error with no fallback acceptable for the MVP, or should `init` fall back to `create_new` with cleanup on write failure?
