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
authored plan. `check` stays strictly structural, so a scaffold with only those
two placeholders fixed can pass it. Completeness is therefore an authoring gate
in the workflow, not a checker rule. The `sce-change-to-plan` workflow uses
`init` to create new plans and fills the scaffold. Before it reports
`plan_ready`, it requires that no scaffold `TODO` placeholder remains and that
`check` passes. Revisions of existing plans skip `init` but must meet the same
structured plan contract and pass the same `check`.

Only structured plans are supported. The workflow has no fallback: when `init`
or `check` is unavailable, fails, or returns unexpected output, it returns
`blocked`. It never detects older binaries, never authors a new plan without
`init`, and never substitutes a manual reference pass for `sce plan check`. This
change removes the existing manual checker fallback from step 2.10.

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
  - Boundary: `<name>` is one required positional `String` argument, parsed under the existing Clap conventions with no extra parser settings. Every value Clap accepts, including an empty string and a value given after `--`, then goes through application validation: it must match `[a-z0-9]+(-[a-z0-9]+)*`, be at most 64 bytes, and have no `.md` suffix. This grammar excludes path separators and traversal. Clap rejects hyphen-prefixed values such as `-leading`, unknown options, and extra positionals before validation runs. The repository root comes from `git rev-parse --show-toplevel`, run from an explicit start directory; the command passes the process current directory, so tests never change process-global state. A root that is not valid UTF-8 or that contains a newline is refused before any write, because it cannot be printed as a single shell-quoted line. `context` and `context/plans` must be real directories when they exist; they are created when absent. The content is fully written to a hidden `create_new` temporary file inside `context/plans/`, then `std::fs::hard_link` publishes it to `<name>.md`. A successful hard link is the commit point. The temporary file is then removed, both after a successful publication and on every handled error path that follows its creation. A removal failure after a successful link is a nonfatal stderr warning; a removal failure after a failed write or link is appended to the original error.
  - Guarantee: The invariant is that the destination is published atomically and never exposes partial scaffold content. The destination path either does not exist or holds the complete scaffold. An existing entry at the destination is never replaced or modified. A handled error attempts to remove the temporary file and never leaves a partially written destination. The published destination is never removed. Crash recovery does not guarantee temporary-file cleanup.
  - Publication lifecycle: The state a run can leave depends on where it stops.
    - Before the hard link publishes the destination: a crash or kill can leave an orphaned temporary file, but this run has not created the destination.
    - After the hard link and before the temporary file is removed: a crash or kill can leave both the complete destination file and the temporary file.
    - After the temporary file is removed: only the complete destination remains.
    - After the hard link when removing the temporary file fails: the run succeeds with exit `0`, the D5 stdout, and a stderr warning naming the temporary file, which remains for manual removal.
    - On a handled error: the temporary file is removed, an existing destination is never overwritten, and no partially written destination is left. When that removal also fails, the original error and class are returned with the cleanup failure appended, and the temporary file can remain.
    - On a filesystem or publication failure: directories that the run newly created may remain, empty unless other content already existed.
  - Limits: This is not a filesystem-wide transaction, and it promises no durability across power loss. `context/` and `context/plans/` created by a run stay in place when a later step fails. Temporary-file cleanup after a process crash or kill is not promised, so an orphaned temporary file can remain beside no destination or beside a complete one.
  - Rationale: `link` fails on any existing entry, including a dangling or conflicting symlink, so a plan is never overwritten. The destination appears only through the hard link to a fully written temporary file, so no observer and no crash ever sees a half-written plan that later blocks `init`. Empty directories and orphaned temporary files are harmless to correctness, so tolerating them avoids rollback and crash-recovery logic.
  - Rejected: `rename` — it silently replaces an existing destination. Writing directly with `create_new` — an interrupted write leaves a partial plan at the final path. Falling back to that non-atomic write when hard links are unsupported — it reintroduces partial plans. Rolling back created directories on failure — it adds removal logic that can race with concurrent writers in order to remove harmless empty directories. Accepting a path argument — that would invite traversal and repository escape. Setting `allow_hyphen_values` or similar so that `-leading` reaches name validation — it departs from the CLI's Clap conventions only to change which error class a name that is invalid anyway receives.
  - Trade-offs: This requires a filesystem that supports hard links inside `context/plans/`. On one that does not, `init` fails with a runtime error and has no non-atomic fallback. Symlinked `context` or `context/plans` directories are refused, even in a repository that deliberately symlinks `context/` elsewhere, because following the link would allow writes outside the repository root. This is resolved and implemented in T02; it is no longer an open question. A failed run can leave newly created empty directories. A crashed run can leave an orphaned temporary file, either alone or beside a complete destination; when the destination exists, rerunning `init` with the same name reports the existing destination, and the orphaned file must be removed by hand.
  - Review: required
  - Review question: Are hard-link-only publication and its filesystem limitations acceptable?
- D5: `init` reports results through the existing `CliError` classes and prints exactly three lines on stdout.
  - Boundary: On success, the exit code is `0` and stdout is exactly three lines: `Created plan: context/plans/<name>.md`, then `Next: complete the plan and run`, then `sce plan check <quoted-path>`. `<quoted-path>` is the absolute path `<repository-root>/context/plans/<name>.md`, always wrapped in POSIX single quotes, with each `'` in it written as `'\''`. When the hard link succeeds but temporary-file removal fails, the exit code and stdout are unchanged, and stderr carries one `Warning: Created plan context/plans/<name>.md, but failed to remove temporary file '<path>': ...` line that asks for manual removal and states that the plan does not need to be initialized again. An invalid name that reaches application validation, or an existing destination, is `CliError::validation` (exit `3`). Running outside a Git repository, a failed or missing `git`, an unprintable root, or a filesystem failure is `CliError::runtime` (exit `4`). Clap errors keep the existing `classify_clap_error` mapping unchanged: unknown options, hyphen-prefixed values, and extra arguments are parse errors (exit `2`), and the missing `<NAME>` argument is `MissingRequiredArgument`, which that mapping already reports as `SCE-ERR-VALIDATION` (exit `3`), as `sce plan check` does today.
  - Side effects by failure class: Clap errors and application name-validation errors happen before any filesystem access, so they create no directory or file and modify no existing file. A missing repository, a failed or missing `git`, and an unprintable root are detected before any write, so they also leave the filesystem unchanged. An existing destination is never overwritten or modified, and no temporary file remains. A handled filesystem or publication failure never publishes a partial destination file and removes its temporary file, but directories that the run created may remain, as D4 states. These side effects cover handled errors only; after a process crash or kill, the D4 publication lifecycle applies, and an orphaned temporary file can remain.
  - Rationale: Automation and the workflow can branch on the class without parsing text. The `Created plan:` path is always repository-relative, whichever directory `init` ran from, so the workflow can match it. `sce plan check` keeps resolving relative paths against the current directory, so the printed command uses the absolute path and works unchanged from any directory. Always quoting keeps the output deterministic and safe for roots containing spaces, apostrophes, or other shell metacharacters.
  - Rejected: A dedicated new exit code or error class — the existing taxonomy already covers these failures. Printing a repository-relative `check` path — it fails with a missing-file runtime error when `init` runs from a nested directory. Making `check` resolve paths against the repository root — it would change existing `check` semantics. Quoting only when needed — it adds a second output shape for no gain.
  - Trade-offs: A destination that already exists, an invalid name, and a missing argument share exit `3`, so callers that need to tell them apart must read stderr. The third line depends on the checkout location, so tests compute the expected line from the canonical repository root rather than comparing against a fixed string.
- D6: The workflow supports structured plans only. It creates every new plan with `sce plan init`, checks every written plan with `sce plan check`, and has no fallback for either. It gates `plan_ready` on a fully authored plan and a passing `check`.
  - Boundary: In `changeToPlanPlanAuthoring`, step 2.10 handles each plan target as follows:
    - New plan: after the clarification gate and target resolution, run `sce plan init {plan_name}`. Initialization succeeds only when all three hold: exit `0`, stdout contains `Created plan: context/plans/{plan_name}.md`, and that file exists. Every other result returns `blocked` with the exit code and stderr. This includes a missing `sce` executable, a binary without `init`, a nonzero exit, an exit `0` without the expected line, and an expected line with no created file. The agent then fills the generated scaffold with the plan content, applying the authoring rules in `references/plan-template.md`. The template is the authoring reference for filling the scaffold, never a substitute for `init`.
    - Existing plan: do not run `init`. Apply the `Updating an existing plan` rules, which preserve completed tasks and their evidence. The revised plan must satisfy the same structured plan contract.
    - Completeness gate, for both paths: the plan contains no scaffold `TODO` placeholder, and optional sections without real entries (`Design decisions`, `Assumptions`) are removed. The change summary, acceptance criteria, design decisions, verification scenarios, task definitions, and verify instructions are authored for this change. While any placeholder remains, the workflow keeps authoring or returns `blocked`, and never reports `plan_ready`.
    - Structural check, for both paths: run `sce plan check {plan-path}`. It passes only on exit `0` with `structured plan: reference integrity holds` on stdout. On exit `3` with structural diagnostics, repair each reported defect in the plan and rerun it, or return `blocked` with the remaining diagnostics when they cannot be resolved. Every other result returns `blocked`, including a missing `sce` executable, a binary without `check`, any other exit code, and an exit `0` without the success marker.
    `plan_ready` requires both a passing completeness gate and a passing `sce plan check`. The step 2.9 reference-consistency pass stays as pre-write authoring guidance only; it never substitutes for a successful `sce plan check`. The existing step 2.10 text that treats any other checker outcome as "unavailable" and applies a manual `Reference rules` pass instead is removed.
  - Rationale: Structured plans are the only supported format, so any fallback could report an unchecked or hand-shaped plan as ready. Requiring an exact exit code, output line, and created file for `init`, and an exact success marker for `check`, distinguishes real success from a missing, outdated, wrapped, or broken binary without identifying versions. `check` is structural by design (S7), so completeness must be enforced where the plan is authored.
  - Rejected: Detecting older `sce` binaries from their `plan` help output and falling back to manual template authoring — it adds version-sniffing complexity for a format that is no longer supported, and it can hide a broken install. Applying the manual `Reference rules` pass when `check` is unavailable — it is not equivalent to the checker and would let unchecked plans reach `plan_ready`. Running `init` during 2.1 target resolution — it would write a file before the clarification gate. Detecting TODO placeholders in the Rust checker, or adding a validator mode — it would turn `check` into semantic approval, which is out of scope.
  - Trade-offs: An environment without a current `sce` that provides both `plan init` and `plan check` cannot produce or revise plans. The workflow returns `blocked` with the error instead. The workflow depends on the exact stdout wording of D5's first line and of `check`'s success marker, so changing either is a breaking change for the workflow. The completeness gate is an instruction to the authoring agent, enforced by generation contract checks on the workflow text, not by a CLI check of each plan.
  - Review: required
  - Review question: Is workflow-only completeness enforcement acceptable?

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: `sce plan init example`, run inside a Git repository, creates `context/plans/example.md` holding the embedded scaffold with the title `# Plan: example`, and creates `context/plans/` when it is missing. It prints the three-line D5 success output: the repository-relative `Created plan:` path, and a `sce plan check` command with the shell-quoted absolute plan path. No temporary file remains in `context/plans/`, and no other repository file is created or modified.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D3, D4, D5
  - Scenarios: S1
- [x] AC2: Existing destinations, including symlinks, are never overwritten or modified, and a conflict leaves no temporary file behind. Invalid names that reach application validation exit `3`. Inputs that Clap rejects exit `2`, and a missing `<NAME>` keeps the existing Clap mapping (exit `3`). Argument and name-validation rejections create no directory or file. A missing repository exits `4` and leaves the directory unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D4, D5
  - Scenarios: S3, S4, S5
- [x] AC3: Running `init` from a nested directory of a Git repository, including one whose root path contains a space and an apostrophe, creates `<root>/context/plans/<name>.md`. It prints the same repository-relative `Created plan:` line. Its printed `sce plan check` command, run from that nested directory, checks the created file and fails structural validation with exit `3`, not a missing-file runtime error.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D4, D5
  - Scenarios: S2
- [x] AC4: The untouched scaffold fails `sce plan check` with exactly the incomplete-reference defects. The same scaffold with only its reference placeholders filled passes, although its other `TODO` placeholders remain, which shows that `check` is structural and not semantic approval.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::scaffold`
  - Decisions: D1
  - Scenarios: S6, S7
- [x] AC5: The scaffold is generated from the canonical Pkl sources and pinned to the template skeleton by the generation contract. It is embedded in the binary, it is present in the packaged fallback, and it is never committed to the repository.
  - Validate: `nix run .#pkl-check-generated`
  - Decisions: D1, D2
  - Scenarios: S8, S9
- [x] AC6: The generated `sce-change-to-plan` workflow runs `sce plan init` for new plans only, after its clarification and target-resolution gates, and skips `init` for revisions while preserving completed tasks and evidence. It accepts initialization only on exit `0` with the success line and an existing file. It returns `blocked` when `init` is missing, unsupported, exits nonzero, or returns unexpected output. It never reports `plan_ready` while scaffold `TODO` placeholders remain. For new plans and revisions alike, it reports `plan_ready` only after `sce plan check` exits `0` with `structured plan: reference integrity holds`. It repairs and reruns on structural diagnostics, and returns `blocked` when `check` is unavailable or returns any other result. The workflow contains no legacy-binary detection, no manual initialization fallback, and no manual validation fallback. Generation contract checks enforce each of these rules and reject negative fixtures that break them.
  - Validate: `nix run .#pkl-check-generated`
  - Decisions: D6
  - Scenarios: S10, S11, S13, S14, S15
- [x] AC7: `sce --help`, `sce plan --help`, and the generated Bash, Zsh, and Fish completion scripts include `plan init`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_init`
  - Decisions: D3
  - Scenarios: S12
- [x] AC8: A successful hard link is the commit point. When removing the temporary file then fails, `init` exits `0` with the D5 stdout, leaves the complete destination in place, and writes a stderr warning that names the temporary file. When writing or publishing fails, the original error and exit class are kept, the temporary file is removed, and a failed removal is reported as additional context. An existing destination is never overwritten.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init`
  - Decisions: D4, D5
  - Scenarios: S16

### Verification scenarios

- S1: Fresh initialization at the repository root
  - Given: A temporary `git init` repository with no `context/` directory, the current directory at its root.
  - When: `sce plan init example` runs.
  - Then: Exit `0`. Stdout is exactly `Created plan: context/plans/example.md`, `Next: complete the plan and run`, and `sce plan check '<root>/context/plans/example.md'`, where `<root>` is the canonicalized repository root. That file equals the embedded scaffold with its single `{plan-name}` token replaced by `example`. `context/plans/` holds only `example.md`, so no temporary file remains, and `git status --porcelain` lists only `context/`.
  - Check: `plan::init_creates_scaffold_at_repository_root`, created by T02.
- S2: Nested invocation resolves the repository root, and its printed check command works there
  - Given: A temporary Git repository whose root directory name contains a space and an apostrophe (for example `repo with 'quote'`), with the start directory at `<root>/a/b`.
  - When: `sce plan init nested-plan` runs. Then its third stdout line is executed by `sh -c` with `<root>/a/b` as the working directory and `sce` defined as a shell function that prints its decoded arguments NUL-separated. The decoded arguments are then dispatched to the in-process `plan` command.
  - Then: `<root>/context/plans/nested-plan.md` exists, nothing is created under `<root>/a/b`, and the first stdout line is exactly `Created plan: context/plans/nested-plan.md`. The shell decodes the third line into exactly `plan`, `check`, and the absolute path of the created file. Dispatching those arguments exits `3` (`SCE-ERR-VALIDATION`) with exactly the S6 diagnostics, and stderr has no `Failed to read plan file` runtime error.
  - Check: `plan::init_from_nested_directory_targets_repository_root`, created by T02.
- S3: Existing entries are never overwritten
  - Given: A repository where `context/plans/taken.md` exists as a regular file with known bytes, and `context/plans/linked.md` is a dangling symlink.
  - When: `sce plan init taken` and `sce plan init linked` run.
  - Then: Both exit `3` (`SCE-ERR-VALIDATION`). `taken.md` keeps its bytes, `linked.md` is still the same symlink with the same target, and `context/plans/` holds only those two entries, so no temporary file remains. Each conflict is detected when the hard link fails after the temporary file is written, so this case exercises the same cleanup path that a filesystem or publication failure takes. The scenario asserts no directory rollback.
  - Check: `plan::init_never_overwrites_existing_entry`, created by T02.
- S4: Unsafe names and malformed invocations are rejected before any write
  - Given: A repository without `context/`.
  - When: Three groups of arguments are each parsed and run through the runtime command path. Application validation: `sce plan init` followed by each of `../escape`, `a/b`, `Upper`, `trailing-`, `double--hyphen`, `plan.md`, an empty string, a 65-byte name, and `-- -leading`. Clap parse: `sce plan init -leading`, `sce plan init --force example`, and `sce plan init one two`. Missing argument: `sce plan init` with no name.
  - Then: Each application-validation case exits `3` with `SCE-ERR-VALIDATION` and a plan-name error. Each Clap parse case exits `2` with `SCE-ERR-PARSE`. The missing-argument case exits `3` through the existing `MissingRequiredArgument` mapping and names `<NAME>`. In every case, the repository's file listing is unchanged afterwards: `context/` does not exist, and no directory or file has been created.
  - Check: `plan::init_rejects_unsafe_names_without_writing`, created by T02.
- S5: Outside a Git repository
  - Given: A temporary directory that is not inside any Git repository.
  - When: `sce plan init example` runs there.
  - Then: Exit `4` (`SCE-ERR-RUNTIME`). The repository lookup fails before any write, so the directory's file listing is unchanged and no `context/` directory exists.
  - Check: `plan::init_outside_repository_is_runtime_failure`, created by T02.
- S6: The untouched scaffold is not a ready plan
  - Given: The embedded scaffold with its title token replaced.
  - When: It is extracted and validated by the plan service.
  - Then: The diagnostics are exactly ``Scenarios:` on `AC1` is not a comma-separated ID list``, ``AC1` is not covered by any task's `Covers:``, and ``Covers:` on `T01` is not a comma-separated ID list``. The scaffold defines no `D` or `S` entry.
  - Check: `plan::scaffold_untouched_fails_check`, created by T01.
- S7: Structural validation is not semantic approval
  - Given: A scaffold created by `sce plan init authored` in which only `Scenarios: TODO` is replaced by `Scenarios: none (inspection)` and `Covers: TODO` by `Covers: AC1`; every other `TODO` placeholder remains.
  - When: `sce plan check` runs on the created file's absolute path.
  - Then: Exit `0`, with stdout `structured plan: reference integrity holds`, and the file still contains `TODO` placeholders. This passing result is structural only; S13 covers the workflow gate that keeps such a plan from being reported `plan_ready`.
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
- S10: New-plan authoring always uses `init`, with no fallback
  - Given: The generated `sce-change-to-plan/references/plan-authoring.md`.
  - When: Its write step for a new plan is inspected.
  - Then: It runs `sce plan init {plan_name}` after the clarification gate. It accepts initialization only with exit `0`, the `Created plan: context/plans/{plan_name}.md` line, and an existing file. It returns `blocked`, with the exit code and stderr, when `sce` is not found, when `init` is unsupported, on any nonzero exit, on an exit `0` without the expected line, and when the line is printed but no file exists. It fills the generated scaffold and never writes a new plan from the template without `init`. It then applies the completeness gate and runs `sce plan check {plan-path}` before `plan_ready`.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion created by T03.
- S11: Revisions skip `init`
  - Given: The generated plan-authoring reference.
  - When: Its existing-plan revision path is inspected.
  - Then: It never runs `sce plan init`. It keeps the `Updating an existing plan` rules, which preserve completed tasks and evidence. It applies the same completeness gate and requires the same passing `sce plan check` before `plan_ready`. No generated `sce-next-task` or `sce-validate` document mentions `sce plan init`.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion created by T03.
- S12: Help and completions expose `plan init`
  - Given: The built CLI schema.
  - When: Top-level help, `plan` help, and the Bash, Zsh, and Fish completion scripts are rendered.
  - Then: Each mentions `init`, and `sce plan init --help` shows the `<NAME>` argument.
  - Check: `plan::plan_init_appears_in_help` and `plan::completion_scripts_include_plan_init`, created by T02.
- S13: `plan_ready` is impossible while scaffold placeholders remain
  - Given: The generated `sce-change-to-plan/references/plan-authoring.md` for each target, plus a negative fixture of the plan-authoring reference without the completeness gate, and one that places the gate after `plan_ready` is reported.
  - When: The generation contract checks run.
  - Then: The generated text requires that no scaffold `TODO` placeholder remains and that unused optional sections are removed. It requires authored acceptance criteria, design decisions, scenarios, tasks, and verify instructions. It places this gate before `plan_ready`, keeps authoring or returns `blocked` while a placeholder remains, and states that a passing `sce plan check` alone is not enough. Both negative fixtures fail the assertion.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion and its negative fixtures created by T03.
- S14: Initialization failures block without a manual fallback
  - Given: The generated plan-authoring reference, plus two negative fixtures. One writes a new plan from `references/plan-template.md` when `init` is unavailable or fails. The other accepts an exit `0` without the `Created plan:` line as success.
  - When: The generation contract checks run.
  - Then: The generated text returns `blocked` for every `init` result other than the D6 success. It contains no legacy-binary detection, such as matching `plan` help or `Usage: plan <COMMAND>`, and no instruction to author a new plan manually when `init` does not succeed. Both negative fixtures fail the assertion.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion and its negative fixtures created by T03.
- S15: `sce plan check` is never bypassed
  - Given: The generated plan-authoring reference, plus two negative fixtures. One keeps the current step 2.10 manual `Reference rules` fallback for an unavailable checker. The other reports `plan_ready` on a `check` exit `0` without `structured plan: reference integrity holds`.
  - When: The generation contract checks run.
  - Then: The generated text requires a `check` exit `0` with that success marker before `plan_ready`, for new plans and revisions alike. It repairs the plan and reruns `check` on structural diagnostics. It returns `blocked` when `check` is unavailable or returns any other result. It states that the step 2.9 reference-consistency pass never substitutes for `sce plan check`, and it contains neither `the checker is unavailable;` nor any manual validation fallback. Both negative fixtures fail the assertion.
  - Check: `nix run .#pkl-check-generated`, with the `plan-authoring-plan-init` contract assertion and its negative fixtures created by T03.
- S16: Publication is the commit point, and cleanup failures never hide or invent an outcome
  - Given: Temporary Git repositories, with `init::create_with` called using deterministic injected link and remove operations.
  - When: (a) A real hard link succeeds and the injected removal fails. (b) The injected link fails and the real removal succeeds. (c) The destination already exists as a regular file with known bytes, the real hard link fails with `AlreadyExists`, and the injected removal fails.
  - Then: (a) Exit `0`, the exact D5 stdout, the exact cleanup warning on stderr naming the temporary file, the complete scaffold at the destination, and the temporary file still present. (b) Exit `4` with the original `Failed to publish` error, no `Cleanup also failed` line, and an empty `context/plans/`. (c) Exit `3` with `Plan already exists: context/plans/taken.md` followed by the `Cleanup also failed` line naming the temporary file, no warning, and `taken.md` keeping its bytes.
  - Check: `plan::init_cleanup_failure_after_publication_warns_and_succeeds`, `plan::init_publication_failure_removes_temporary_and_keeps_error`, and `plan::init_publication_and_cleanup_failure_keeps_original_error_and_destination`, created by T04.

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
- `context/sce/shared-context-plan-workflow.md`: `init` in new-plan authoring, revisions skipping `init`, the mandatory `sce plan check` gate, and the removal of the manual initialization and validation fallbacks.
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
  - Semantic plan approval, TODO detection, a separate semantic validator, or any change to `sce plan check` rules, wording, or path resolution. Plan completeness is enforced only by the workflow's authoring gate.
  - Legacy or unstructured plan formats, detection of older `sce` binaries, and any compatibility behavior for them.
  - Manual plan initialization when `init` does not succeed, and manual validation in place of `sce plan check`.
  - Directory rollback after a failed `init`, temporary-file cleanup after a crash or kill, `fsync` or power-loss durability, and filesystem-wide transactional guarantees.
  - Changes to Clap parser settings or to `classify_clap_error`.
  - `/next-task` and `/validate` workflow content, including the checker fallback that `sce-validate` currently carries.
  - Committing generated target trees or the scaffold.
  - Validating real plans under `context/plans/` in Nix or generation checks.
- **Constraints:**
  - No new traits, `dyn`, boxed futures, async runtime, database, or crate dependency.
  - No interactive prompts.
  - `init` runs offline and invokes only `git`.
  - `init` publishes only through the temporary-file plus hard-link path, never with a non-atomic write.
  - The workflow never reports `plan_ready` unless `sce plan check` actually ran and succeeded.
  - Output is deterministic.
  - All Cargo commands run through `scripts/run-cli-cargo.sh` under `nix develop`.
- **Non-goal:**
  - `init` does not author plan content, accept arbitrary paths, take `--force` or overwrite flags, or support template variants.
  - The workflow does not support environments whose `sce` lacks `plan init` or `plan check`; it returns `blocked` there.

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

- [x] T02: `Add the sce plan init command with safe repository-root creation` (status:done)
  - Task ID: T02
  - Covers: AC1, AC2, AC3, AC4, AC7
  - Scope: In:
    - `PlanSubcommand::Init { name }` and its help text in `cli_schema.rs`.
    - The `PlanRequest` enum and its dispatch in `command.rs` and `command_runtime.rs`.
    - `cli/src/services/plan/init.rs`, covering name validation of every value Clap accepts, root resolution from an explicit start directory, the unprintable-root refusal, directory checks, and temporary-file plus hard-link publication. The temporary file is removed after publication and on every handled error path that follows its creation. Directories created by the run are not rolled back, there is no non-atomic fallback, and no crash-recovery cleanup or `fsync` is added.
    - The D5 output and error contract, including the always-single-quoted absolute path in the printed `sce plan check` line.
    - The S1–S5, S7, and S12 tests in `cli/src/services/plan/mod.rs`, plus updates to the existing `plan::command_*` tests. S2 uses a root containing a space and an apostrophe and executes the printed line through `sh`. S4 covers the application-validation, Clap-parse, and missing-argument groups separately.

    Out: Pkl sources, workflow prose, changes to `check` behavior or path resolution, changes to `classify_clap_error`, and Clap parser settings such as `allow_hyphen_values`.
  - Dependencies: T01
  - Done when: `sce plan init <name>` behaves as D4 and D5 state from the repository root and from nested directories. Its printed `check` command works from any directory. Clap-rejected inputs keep their existing exit classes. Argument, name, and repository failures create no file or directory. Existing destinations are never modified. No handled failure publishes a partial plan or leaves a temporary file. All listed tests pass. Help and completions show `init`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` — passed (25 tests, including the eight new S1–S5, S7, and S12 tests); `nix develop -c sh -c 'cd cli && cargo fmt'` — applied. Also: `clippy --all-targets --all-features -- -D warnings` via `run-cli-cargo.sh` — clean; full CLI test suite via `run-cli-cargo.sh test` — 635 passed.
  - Completed: 2026-10-24
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/plan/command.rs`, `cli/src/services/plan/init.rs`, `cli/src/services/plan/mod.rs`, `context/plans/sce-plan-init.md`
  - Result: `PlanSubcommand::Init { name }` maps to `PlanRequest::Init(PlanInitRequest)` beside `PlanRequest::Check(PlanCheckRequest)`. `PlanCommand::execute` delegates to `execute_with(current_dir)`, which validates the name through `init::PlanName::parse` before reading the current directory, so tests pass an explicit start directory without touching process state. `init::create` resolves the root with `git rev-parse --show-toplevel` from the start directory, refuses non-UTF-8 or multi-line roots, requires real (non-symlink) `context` and `context/plans` directories or creates them, writes `.<name>.md.<pid>.tmp` with `create_new`, publishes with `hard_link`, and removes the temporary file after publication and on every handled error after its creation. `AlreadyExists` on the link is `CliError::validation` (`Plan already exists: context/plans/<name>.md`); git, root, and filesystem failures are `CliError::runtime`. Stdout is the D5 three lines with the absolute path in POSIX single quotes. The T01 `dead_code` expectations on `SCAFFOLD` and `SCAFFOLD_PLAN_NAME_TOKEN` were removed. Deviation: `cli/src/command_surface.rs` gained a `sce plan init <name>` line in the top-level `Plan Usage:` help section, and `PLAN_CLAP_ABOUT` / `PLAN_TOP_LEVEL_PURPOSE` now mention creation, because S12 requires top-level help to mention `init`. The symlinked-directory open question follows D4 (refused, exit `4`).
  - Context impact: `context/cli/plan-service.md` (`init.rs`, `PlanRequest`, `execute_with`, the init command contract, and the new tests), `context/cli/cli-command-surface.md` and `context/sce/cli-shell-completion-contract.md` (the `sce plan init <name>` surface and updated plan help text), `context/architecture.md` and `context/overview.md` (plan service no longer only read-only), `context/glossary.md` (`plan scaffold`).
  - Context synchronization: synced

- [x] T03: `Use sce plan init in the change-to-plan workflow and document it` (status:done)
  - Task ID: T03
  - Covers: AC6
  - Scope: In:
    - `changeToPlanPlanAuthoring` steps 2.1 and 2.10 in `config/pkl/base/workflow-change-to-plan.pkl`. Add the `init`-for-new-plans path with the D6 success condition, `blocked` for every other `init` result, and the skip for revisions. New plans fill the generated scaffold; there is no manual template authoring when `init` does not succeed.
    - Removal of the existing step 2.10 manual checker fallback: the "Trust the checker only when it actually ran" paragraph's "Any other outcome means the checker is unavailable" branch, the manual `Reference rules` pass applied in its place, and the open-question note about an unavailable checker. Replace it with the D6 `check` handling: pass only on exit `0` with the success marker, repair and rerun on structural diagnostics, and `blocked` on anything else. Step 2.9 stays as pre-write guidance and states that it never substitutes for `sce plan check`.
    - The completeness gate in step 2.10, for new plans and revisions. Before `plan_ready`, every scaffold `TODO` placeholder is replaced or its optional section removed. Acceptance criteria, design decisions, scenarios, task definitions, and verify instructions are authored for the change. A passing `sce plan check` is required but not sufficient.
    - The `plan-authoring-plan-init` assertion in `generation-contract-check.pkl`. It requires the D6 tokens and forbids legacy-binary detection, manual initialization, and manual validation fallback wording, so the workflow can never bypass `sce plan check`. It has negative fixtures for S13 (missing or late gate), S14 (manual template fallback; exit `0` without the success line accepted), and S15 (manual `Reference rules` fallback kept; `plan_ready` without the `check` success marker).
    - Update of the existing `assertPlanAuthoringStructuredReferences` tokens and failure message, which currently require the fallback text (the `Any other outcome means the checker is unavailable;` token and the manual `Reference rules` pass token), plus any other contract substring updates needed by the new wording.
    - `context/sce/shared-context-plan-workflow.md`.

    Out: `/next-task` and `/validate` content, including the `sce-validate` checker fallback; the plan template schema; CLI code; and any TODO detection, new mode, or behavior change in `sce plan check`.
  - Dependencies: T02
  - Done when: All four targets' generated `plan-authoring.md` carry the D6 behavior, including the completeness gate and the mandatory `check` gate. None of them contains legacy-binary detection, a manual initialization fallback, or a manual validation fallback. The contract assertion passes on generated output and fails on each of its negative fixtures. No `sce-next-task` or `sce-validate` document mentions `sce plan init`.
  - Verify: `nix run .#pkl-check-generated` — passed (146 files; the new `plan-authoring-plan-init` contract check evaluates to its pass message, which requires all four targets' `plan-authoring.md` to satisfy it and all six negative fixtures to be rejected; the updated `plan-authoring-structured-references` check and its existing negative fixture still pass).
  - Completed: 2026-10-24
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/check-generated.sh`, `config/pkl/renderers/fixtures/plan-authoring-structured-references-check.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `context/sce/shared-context-plan-workflow.md`, `context/plans/sce-plan-init.md`
  - Result: Step 2.1 no longer writes, and says step 2.10 initializes a new plan after the clarification gate. Step 2.9 is pre-write guidance that never substitutes for `sce plan check`. Step 2.10 now has `New plan` (`sce plan init {plan_name}` with the exact three-part D6 success condition, `blocked` otherwise, then fill the scaffold), `Existing plan` (no `init`, `Updating an existing plan` rules), `Completeness gate`, and `Structural check` (exit `0` plus success marker, repair and rerun on exit `3`, `blocked` otherwise) subsections, and ends with the `plan_ready` permission. The manual `Reference rules` fallback and its open-question note were removed. `assertPlanAuthoringPlanInit` requires the D6 tokens, forbids `manual`, `checker is unavailable`, and legacy-help wording, orders the clarification gate before `sce plan init` and the gate and check marker before the `plan_ready` permission, and keeps `sce plan init` out of `sce-next-task` and `sce-validate` documents. Its six negative fixtures are derived in-contract from each real generated document: gate removed, gate moved after `plan_ready`, manual template fallback appended, init success reduced to exit `0`, the old checker fallback appended, and check success reduced to exit `0`. `requiredPlanAuthoringStructuredTokens` dropped the fallback tokens, and its failure message (mirrored in the fixture and `check-generated.sh`) dropped "with the checker trust rule and manual Reference rules fallback". Deviation: the negative fixtures live in `generation-contract-check.pkl`, following `assertCompactExecutionContractFixtures`, rather than as separate fixture files.
  - Context impact: `context/sce/shared-context-plan-workflow.md` (updated in this task: step 2.10 init, completeness, and check gates, and the change-to-plan versus validate checker split); `context/sce/config-generation.md` (the `plan-authoring-plan-init` contract check).
  - Context synchronization: synced

- [x] T04: `Treat hard-link publication as the init commit point and resolve the symlink question` (status:done)
  - Task ID: T04
  - Covers: AC8
  - Scope: In:
    - `publish` in `cli/src/services/plan/init.rs`: a successful hard link is the commit point; a later cleanup failure becomes a stderr warning with exit `0`, and a cleanup failure after a failed write or link is appended to the original error with its class kept.
    - `init::create_with` with generic link and remove closures for deterministic failure injection, and stderr threaded through `PlanCommand::execute_with_stderr` from `RuntimeCommand::execute_with_stderr`, as `sync` already does.
    - The three S16 tests in `cli/src/services/plan/mod.rs`, and `run_plan_args` capturing command-written stderr.
    - D4 and D5 wording, the resolved symlinked-directory decision, and `context/cli/plan-service.md`.

    Out: changes to `sce plan check`, the workflow, Pkl sources, directory rollback, crash recovery, and `fsync`.
  - Dependencies: T02
  - Done when: A cleanup failure after a successful hard link exits `0` with the D5 stdout and an actionable stderr warning. Write and publication failures keep their original error and class, and report a failed cleanup as extra context. Existing destinations are never overwritten. The open question is resolved in D4.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` — passed (28 tests, including the three new S16 tests); `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings` — clean; `nix develop -c sh -c 'cd cli && cargo fmt'` — applied; `nix flake check` — passed (`cli-tests`, `cli-clippy`, `cli-fmt`); `sce plan check context/plans/sce-plan-init.md` — passed.
  - Completed: 2026-10-24
  - Files changed: `cli/src/services/command_registry.rs`, `cli/src/services/plan/command.rs`, `cli/src/services/plan/init.rs`, `cli/src/services/plan/mod.rs`, `context/cli/plan-service.md`, `context/plans/sce-plan-init.md`
  - Result: `publish` now matches on the publication and cleanup results: both succeed returns no warning; publication succeeds and cleanup fails returns the `Warning: Created plan ...` line, which `create_with` writes to stderr before the unchanged stdout; publication fails returns the original `CliError`; both fail returns `CliError::internal` with the original class and the original message followed by a `Cleanup also failed:` line. `PlanCommand::execute` became `execute_with_stderr`. The symlinked-directory open question is recorded as resolved in D4 (refused, because following the link would allow writes outside the repository root) and Open questions is `None.`.
  - Context impact: `context/cli/plan-service.md` (publication commit point, cleanup warning, failure behavior, `create_with`, `execute_with_stderr`, and the S16 tests).
  - Context synchronization: synced

## Open questions

None.

## Validation Report

**Status:** validated  
**Date:** 2026-10-24

The current status comes from the revalidation below. It covers AC1–AC8 and
S1–S16, including AC8 and S16, which T04 added after the original validation.
The original validation covered only AC1–AC7 and S1–S15. It is kept as a
historical record and is not evidence for AC8 or S16.

### Revalidation after T04 (AC1–AC8, S1–S16)

Run on 2026-10-24 at PR #342 head `69e617c03aa80eebff49c896cbf521672b01ef55`.
The tree was clean apart from edits to `context/plans/`, which Nix and
generation checks do not read.

#### Commands run

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` -> exit 0 (28 passed, 610 filtered out)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init` -> exit 0 (8 passed: the five S1–S5 tests and the three S16 tests)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::scaffold` -> exit 0 (2 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_init` -> exit 0 (1 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::completion_scripts_include_plan_init` -> exit 0 (1 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::plan::<name> -- --exact`, once for each S16 test (`init_cleanup_failure_after_publication_warns_and_succeeds`, `init_publication_failure_removes_temporary_and_keeps_error`, and `init_publication_and_cleanup_failure_keeps_original_error_and_destination`) -> exit 0 each (1 passed)
- Observation run: the three S16 tests ran with `-- --nocapture --test-threads=1` and a temporary `eprintln!` of exit code, stdout, stderr, and `context/plans/` entries. The instrumentation was reverted with `git checkout -- cli/src/services/plan/mod.rs`, and `git status` was clean afterwards. Observed results:
  - (a) Exit `0`. Stdout is the three D5 lines. Stderr is exactly one `Warning: Created plan context/plans/published.md, but failed to remove temporary file '<root>/context/plans/.published.md.<pid>.tmp': injected cleanup failure. Try: remove '…' manually; the plan was created and does not need to be initialized again.` line. Entries are `.published.md.<pid>.tmp` and `published.md`.
  - (b) Exit `4`. Stdout is empty. Stderr is `Error [SCE-ERR-RUNTIME]: Failed to publish '<root>/context/plans/unpublished.md': injected link failure …`, with no `Cleanup also failed` line. Entries are empty.
  - (c) Exit `3`. Stdout is empty. Stderr is `Error [SCE-ERR-VALIDATION]: Plan already exists: context/plans/taken.md. …`, and the next line is `Cleanup also failed: could not remove temporary file '<root>/context/plans/.taken.md.<pid>.tmp': injected cleanup failure. Try: remove it manually.` No `Warning:` appears. Entries are `.taken.md.<pid>.tmp` (the injected removal left it) and `taken.md`, and `taken.md` keeps `known bytes\n`.
- `nix flake check` -> exit 1 (first run, concurrent with other Cargo test runs: `cli-tests` 637 passed, 1 failed — `app::tests::stalled_telemetry_headers_lookup_fails_closed_within_deadline`, "child exited after 8.3714708s", a wall-clock bound outside `services/plan`)
- `nix flake check` -> exit 0 (rerun with no other load: `cli-tests` 638 passed, 0 failed; all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 146 files, inventory sha256 `087ce623…ef16`)
- `nix build .#default --no-link` -> exit 0
- `bash scripts/test-prepare-cli-generated-assets.sh` -> exit 0 (delegation tests passed)
- Inspection: `config/plan-init-scaffold.md` is absent from the worktree, and `git ls-files` lists no `plan-init-scaffold.md`
- `nix run .#sce -- --help` -> `Plan Usage:` lists `sce plan init <name>`; `nix run .#sce -- plan --help` -> lists `init`; `nix run .#sce -- completion --shell {bash,zsh,fish}` -> each script contains `init`
- `nix run .#sce -- plan check context/plans/sce-plan-init.md` -> exit 0 (`structured plan: reference integrity holds`)

#### Success-criteria verification

- [x] AC1: `sce plan init example` creates the scaffold at the repository root with the D5 output -> `plan::init` passed; scenarios: S1
- [x] AC2: Existing entries never overwritten; validation, Clap, and missing-repository failure classes with no writes -> `plan::init` passed; scenarios: S3, S4, S5
- [x] AC3: Nested invocation targets the root, and its printed check command works there -> `plan::init` passed; scenarios: S2
- [x] AC4: Untouched scaffold fails `check` with exactly the reference defects; reference-filled scaffold passes -> `plan::scaffold` passed (2 tests); scenarios: S6, S7
- [x] AC5: Scaffold generated, contract-pinned, embedded, in the fallback, not committed -> `nix run .#pkl-check-generated`, `nix build .#default`, and the fallback test passed; no committed scaffold; scenarios: S8, S9
- [x] AC6: Workflow uses `init` for new plans only, blocks on any other result, gates `plan_ready` on completeness and `check`, has no fallbacks -> `nix run .#pkl-check-generated` passed, including the `plan-authoring-plan-init` contract and its negative fixtures; scenarios: S10, S11, S13, S14, S15
- [x] AC7: Help and completions include `plan init` -> `plan::plan_init` and `plan::completion_scripts_include_plan_init` passed; CLI help and completion output as listed; scenarios: S12
- [x] AC8: Hard-link publication is the commit point; a cleanup failure after it is a warning with exit `0`; write and publication failures keep their error and class and append a failed cleanup; existing destinations are never overwritten -> the three S16 tests passed in `plan::init`, by exact name, and inside `nix flake check`; the observation run matched each S16 case; scenarios: S16

#### Scenario verification

- S1 (AC1): test `services::plan::init_creates_scaffold_at_repository_root` -> ok; status: Verified
- S2 (AC3): test `services::plan::init_from_nested_directory_targets_repository_root` -> ok; status: Verified
- S3 (AC2): test `services::plan::init_never_overwrites_existing_entry` -> ok; status: Verified
- S4 (AC2): test `services::plan::init_rejects_unsafe_names_without_writing` -> ok; status: Verified
- S5 (AC2): test `services::plan::init_outside_repository_is_runtime_failure` -> ok; status: Verified
- S6 (AC4): test `services::plan::scaffold_untouched_fails_check` -> ok; status: Verified
- S7 (AC4): test `services::plan::scaffold_authored_passes_check` -> ok; status: Verified
- S8 (AC5): command `nix run .#pkl-check-generated` (`plan-init-scaffold` contract) -> passed, 146 files; no committed scaffold; status: Verified
- S9 (AC5): commands `bash scripts/test-prepare-cli-generated-assets.sh` and `nix build .#default --no-link` -> both exit 0; status: Verified
- S10 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract) -> passed; status: Verified
- S11 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract) -> passed; status: Verified
- S12 (AC7): tests `services::plan::plan_init_appears_in_help` and `services::plan::completion_scripts_include_plan_init` -> both ok; status: Verified
- S13 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified
- S14 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified
- S15 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified
- S16 (AC8):
  - (a) test `services::plan::init_cleanup_failure_after_publication_warns_and_succeeds` -> ok. It asserts exit `0`, exact D5 stdout, the exact warning naming the temporary file, the complete scaffold at the destination, and the temporary file still present.
  - (b) test `services::plan::init_publication_failure_removes_temporary_and_keeps_error` -> ok. It asserts exit `4`, empty stdout, `SCE-ERR-RUNTIME`, `Failed to publish`, the injected cause, no `Cleanup also failed`, and an empty `context/plans/`.
  - (c) test `services::plan::init_publication_and_cleanup_failure_keeps_original_error_and_destination` -> ok. It asserts exit `3`, empty stdout, `SCE-ERR-VALIDATION`, `Plan already exists: context/plans/taken.md`, the `Cleanup also failed` line naming the temporary file, no `Warning:`, and unchanged `taken.md` bytes.
  - The observation run confirmed all three cases.
  - status: Verified

#### Failed checks and follow-ups

- None. The first `nix flake check` failure is recorded under residual risks.

#### Residual risks

- `app::tests::stalled_telemetry_headers_lookup_fails_closed_within_deadline` failed under concurrent load and passed on the quiet rerun. It asserts a wall-clock bound on a child process and is flaky under load.
- S16 (c) says the `Cleanup also failed` line follows the `Plan already exists` error. The test asserts that both lines are present, but not their order. The order is fixed by the `"{error}\nCleanup also failed: …"` format in `cli/src/services/plan/init.rs` and was confirmed only in the observation run. A future reordering would not fail the test.
- `nix flake check` does not build the release package, so a non-test-only compile failure is caught only by `nix build .#default`.

### Original validation (historical, AC1–AC7 and S1–S15)

Recorded on 2026-10-24, before T04. It did not cover AC8 or S16. Its
`plan::init` count of five tests predates the three S16 tests.

#### Commands run

- `nix run .#sce -- plan check context/plans/sce-plan-init.md` -> exit 0 (`structured plan: reference integrity holds`)
- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 146 files, inventory sha256 49dfdb6b…0392)
- `nix build .#default --no-link` -> exit 0 (packaged CLI built)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::init` -> exit 0 (5 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::scaffold` -> exit 0 (2 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_init` -> exit 0 (1 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::completion_scripts_include_plan_init` -> exit 0 (1 passed)
- `bash scripts/test-prepare-cli-generated-assets.sh` -> exit 0 (delegation tests passed)

#### Success-criteria verification

- [x] AC1: `sce plan init example` creates the scaffold at the repository root with the D5 output -> `plan::init` filter passed (5 tests); scenarios: S1
- [x] AC2: Existing entries never overwritten; validation, Clap, and missing-repository failure classes with no writes -> `plan::init` filter passed; scenarios: S3, S4, S5
- [x] AC3: Nested invocation targets the root and its printed check command works there -> `plan::init` filter passed; scenarios: S2
- [x] AC4: Untouched scaffold fails `check` with exactly the reference defects; reference-filled scaffold passes -> `plan::scaffold` filter passed (2 tests); scenarios: S6, S7
- [x] AC5: Scaffold generated, contract-pinned, embedded, in the fallback, not committed -> `nix run .#pkl-check-generated` passed, `nix build .#default` passed, fallback test passed, no `config/plan-init-scaffold.md` in the worktree or `git ls-files`; scenarios: S8, S9
- [x] AC6: Workflow uses `init` for new plans only, blocks on any other result, gates `plan_ready` on completeness and `check`, has no fallbacks -> `nix run .#pkl-check-generated` passed, including the `plan-authoring-plan-init` contract and its negative fixtures; scenarios: S10, S11, S13, S14, S15
- [x] AC7: Help and completions include `plan init` -> `plan::plan_init` filter passed, and `cli-tests` passed inside `nix flake check`; scenarios: S12

#### Scenario verification

- S1 (AC1): test `services::plan::init_creates_scaffold_at_repository_root` -> ok; status: Verified
- S2 (AC3): test `services::plan::init_from_nested_directory_targets_repository_root` -> ok; status: Verified
- S3 (AC2): test `services::plan::init_never_overwrites_existing_entry` -> ok; status: Verified
- S4 (AC2): test `services::plan::init_rejects_unsafe_names_without_writing` -> ok; status: Verified
- S5 (AC2): test `services::plan::init_outside_repository_is_runtime_failure` -> ok; status: Verified
- S6 (AC4): test `services::plan::scaffold_untouched_fails_check` -> ok; status: Verified
- S7 (AC4): test `services::plan::scaffold_authored_passes_check` -> ok; status: Verified
- S8 (AC5): command `nix run .#pkl-check-generated` (`plan-init-scaffold` contract) -> passed, 146 files; no repository-local `config/plan-init-scaffold.md`; status: Verified
- S9 (AC5): command `bash scripts/test-prepare-cli-generated-assets.sh` and `nix build .#default --no-link` -> both exit 0; status: Verified
- S10 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract) -> passed; status: Verified
- S11 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract) -> passed; status: Verified
- S12 (AC7): test `services::plan::plan_init_appears_in_help` and `services::plan::completion_scripts_include_plan_init` -> both ok; `cli-tests` passed in `nix flake check`; status: Verified
- S13 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified
- S14 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified
- S15 (AC6): command `nix run .#pkl-check-generated` (`plan-authoring-plan-init` contract and negative fixtures) -> passed; status: Verified

#### Failed checks and follow-ups

- None at validation time.
- Post-validation review follow-up (PR #342), recorded as T04 without changing the results above: temporary-file cleanup after a successful hard link no longer fails `init` (AC8, S16), and the resolved symlinked-directory question was removed from Open questions and recorded in D4.

#### Residual risks

- `nix flake check` does not build the release package, so a non-test-only compile failure is caught only by `nix build .#default`.
- The worktree carries an out-of-scope test change in `cli/src/services/hooks/claude_mutation_scope/state.rs`; it is not part of this plan.
