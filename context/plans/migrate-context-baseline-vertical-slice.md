# Plan: migrate-context-baseline-vertical-slice

## Change summary

Migrate `sce setup`'s durable-context baseline bootstrap — currently
`services::setup::bootstrap_context_baseline` in `cli/src/services/setup/mod.rs`,
which mixes the canonical directory/file manifest, `RepoPaths`-based path
calculation, direct `std::fs` I/O, and styled success-string rendering into one
function — into the hexagonal skeleton landed by
`context/plans/cli-hexagonal-architecture-skeleton.md`. This is the skeleton's
first real vertical slice: a `ContextBaseline` domain model
(`cli/src/domain/context/`), a narrow `ContextStore` application port and
`EnsureContextBaseline` use case (`cli/src/application/`), a
`FilesystemContextStore` outbound adapter that performs the actual I/O
(`cli/src/adapters/outbound/filesystem/`), and an inbound renderer
(`cli/src/adapters/inbound/cli/setup.rs`). `services::setup::bootstrap_context_baseline`
becomes a thin compatibility facade that constructs the adapter, runs the use
case, and renders the report — so both call sites that already route through
it (`sce setup --bootstrap-context` and every normal successful `sce setup`
run) pick up the new implementation automatically, with no change to the
public `sce setup` command surface, output text, or styling.

This plan does not touch `SetupRequest`, Clap parsing, repository discovery,
prompts, workflow selection, integration installation, lifecycle providers, or
any other part of `setup`. `composition::run` continues to delegate to
`app::run`; this slice proves the layering works end-to-end for one operation
without wiring `setup` itself through `composition.rs`.

## Acceptance criteria

- [x] AC1: The canonical context-baseline directory/file manifest is defined
  once, in `cli/src/domain/context/baseline.rs`, and nowhere in
  `cli/src/services/setup/mod.rs`.
  - Validate: `test -f cli/src/domain/context/baseline.rs`; `grep -n "CONTEXT_OVERVIEW_TEMPLATE\|CONTEXT_MAP_TEMPLATE\|CONTEXT_TMP_GITIGNORE_CONTENT" cli/src/services/setup/mod.rs` matches nothing.
- [x] AC2: `EnsureContextBaseline::execute` depends only on the
  application-owned `ContextStore` port and the domain `ContextBaseline`
  type — no `crate::services`, `crate::adapters`, or infrastructure import.
  - Validate: `grep -n "crate::services\|crate::adapters\|std::fs" cli/src/application/use_cases/ensure_context_baseline.rs` matches nothing; `./scripts/check-cli-architecture.sh` passes.
- [x] AC3: All filesystem access for the migrated baseline path (directory
  creation, existence checks, file writes) lives in
  `cli/src/adapters/outbound/filesystem/context_store.rs` and nowhere in
  `domain` or `application`.
  - Validate: `grep -rn "std::fs\|Path::exists\|fs::create_dir_all\|fs::write" cli/src/domain cli/src/application` matches nothing.
- [x] AC4: An existing baseline file with custom content is left byte-for-byte
  unchanged by a bootstrap run.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml bootstrap_context_baseline_is_additive_and_idempotent`
- [x] AC5: Every canonical baseline directory and file is created when
  missing.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml bootstrap_context_baseline_creates_expected_paths`
- [x] AC6: Running the use case twice against the same repository root
  produces no file-content changes, and the second run's
  `ContextBaselineChanges` reports every baseline path under
  `existing_directories`/`existing_files` rather than the `created_*` fields.
  - Validate: new `FilesystemContextStore` unit test asserting the second
    `ensure_baseline` call's returned `ContextBaselineChanges` on an
    already-bootstrapped tree.
- [x] AC7: Both `sce setup --bootstrap-context` and a normal successful `sce
  setup` run invoke the migrated `EnsureContextBaseline` use case, because
  both already route through the single `bootstrap_context_baseline` call
  site in `cli/src/services/setup/command.rs:60`.
  - Validate: existing tests `resolve_setup_request_accepts_bootstrap_context_alone`,
    `parser_routes_bootstrap_context_to_context_only_request`, and the two
    `bootstrap_context_baseline_*` tests all continue to pass unchanged.
- [x] AC8: `sce setup --bootstrap-context` and normal setup still emit
  `Context baseline ensured.` with unchanged styling.
  - Validate: `grep -n "Context baseline ensured" cli/src/adapters/inbound/cli/setup.rs`; existing test assertions `message.contains("Context baseline ensured.")` continue to pass.

### Full validation

- `nix flake check`
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/check-cli-architecture.sh`
- `./scripts/test-check-cli-architecture.sh`

### Context sync

- `context/architecture.md` (`## CLI internal hexagonal architecture`
  currently states "No slice migration is in scope for this skeleton phase";
  that sentence goes stale once this plan lands and needs a first-slice note)
- `context/sce/setup-repo-local-config-bootstrap.md` (`## Implementation`
  currently describes `bootstrap_context_baseline` as calling
  `ensure_context_directory`/`ensure_context_file` directly against
  `RepoPaths` accessors; needs to describe the new
  domain/application/adapter path)
- `context/glossary.md` (optional new entries for `ContextStore`,
  `EnsureContextBaseline`, and `FilesystemContextStore`, following the
  existing pattern for other adapter/seam entries such as `local Turso
  adapter`)

## Constraints and non-goals

- **In scope:** new `cli/src/domain/context/{mod.rs,baseline.rs}`; new
  `cli/src/application/ports/context_store.rs`; new
  `cli/src/application/use_cases/ensure_context_baseline.rs`; new
  `cli/src/adapters/outbound/filesystem/{mod.rs,context_store.rs}`; new
  `cli/src/adapters/inbound/cli/setup.rs`; the corresponding `mod`
  declarations in `cli/src/domain/mod.rs`, `cli/src/application/ports/mod.rs`,
  `cli/src/application/use_cases/mod.rs`, `cli/src/adapters/outbound/mod.rs`,
  `cli/src/adapters/inbound/cli/mod.rs`; `cli/src/services/setup/mod.rs`
  (reducing `bootstrap_context_baseline` to a compatibility facade and
  removing the now-superseded `CONTEXT_*_TEMPLATE` constants,
  `ensure_context_directory`, `ensure_context_file`, and their tests'
  internal path construction); `cli/src/services/default_paths.rs` (removing
  the `RepoPaths::context_*` accessors once the migrated path makes them
  unused, to avoid dead-code warnings under `cargo build`/`clippy -D
  warnings`); the listed `context/**` files.
- **Out of scope:** `SetupRequest`, Clap setup parsing, repository discovery,
  interactive prompts, optional-workflow selection, integration
  installation, lifecycle providers, config bootstrap, database
  initialization, hooks, `AppContext`, any other part of the `setup` command;
  `composition.rs` (setup is not wired through it in this phase);
  `services::setup::command.rs` beyond the fact that its existing call to
  `bootstrap_context_baseline` is unchanged.
- **Constraints:** single Cargo package, no new crate dependencies; `domain`
  and `application` code must satisfy `scripts/check-cli-architecture.sh`;
  the rendered `"Context baseline ensured."` output stays byte-for-byte
  identical; existing tests that assert on `bootstrap_context_baseline`'s
  return value keep passing without relaxing their assertions.
- **Non-goal:** migrating any other part of `setup`, or any other command, in
  this plan. `application/ports` gains exactly one port
  (`ContextStore`); no speculative `FileSystem`/`Database`/`HttpClient`/
  `Clock`/`Logger` port is added.
- **Non-goal:** exposing the created/existing file lists in the CLI's
  rendered output. `render_context_baseline_report` renders only the fixed
  success string; `ContextBaselineChanges` is proven through tests, not
  through a new output contract.

## Assumptions

- `EnsureContextBaseline::execute` returns `Result<EnsureContextBaselineReport, S::Error>`
  directly rather than introducing a separate `EnsureContextBaselineError`
  wrapper type, since `ContextStore::Error` is already the operation's only
  failure mode and an application-owned wrapper would add a layer with
  nothing to say. `services::setup::bootstrap_context_baseline` converts that
  error to `anyhow::Error` at the compatibility-facade boundary, same as
  today.
- `RepoPaths::context_dir()`, `context_plans_dir()`, `context_handovers_dir()`,
  `context_decisions_dir()`, `context_tmp_dir()`, `context_overview_file()`,
  `context_architecture_file()`, `context_patterns_file()`,
  `context_glossary_file()`, `context_map_file()`, and
  `context_tmp_gitignore_file()` in `cli/src/services/default_paths.rs` have
  no callers outside `services/setup/mod.rs` (confirmed by repository search).
  Once the migrated adapter builds baseline paths by joining
  `repository_root` with `ContextBaseline`'s own relative-path strings
  instead of calling these accessors, they become dead code and are removed
  in the same task, with the two existing tests that currently call them
  switched to plain `repo.join("context")`-style path construction for their
  assertions. This keeps `cargo build`/`clippy --all-targets --all-features
  -D warnings` clean, consistent with T01's "unused-code-clean" bar in the
  hexagonal-skeleton plan.
- `ContextBaseline::sce_default()`'s directory and file lists, and every
  file's `initial_content`, are byte-identical to the current
  `CONTEXT_*_TEMPLATE` constants and `CONTEXT_TMP_GITIGNORE_CONTENT` in
  `cli/src/services/setup/mod.rs`, per `context/sce/setup-repo-local-config-bootstrap.md`'s
  "Baseline paths" list — this plan relocates that content, it does not
  change it.

## Task stack

- [x] T01: `Add the domain ContextBaseline model` (status:done)
  - Task ID: T01
  - Goal: Define `ContextBaseline`, `BaselineFile`, and
    `ContextBaseline::sce_default()` in `cli/src/domain/context/baseline.rs`,
    with relative paths and template content copied verbatim from the
    current `CONTEXT_*_TEMPLATE` constants and directory list in
    `cli/src/services/setup/mod.rs`. Add `cli/src/domain/context/mod.rs` and
    wire `pub(crate) mod context;` into `cli/src/domain/mod.rs`.
  - Boundaries (in/out of scope): In — `cli/src/domain/context/{mod.rs,baseline.rs}`,
    the one-line `mod` addition to `cli/src/domain/mod.rs`. Out — any
    application, adapter, or `services` change; this task does not wire the
    new type into anything yet.
  - Dependencies: none
  - Done when: `ContextBaseline::sce_default()` returns the 5 canonical
    directories (`context`, `context/plans`, `context/handovers`,
    `context/decisions`, `context/tmp`) and 6 canonical files
    (`context/overview.md`, `context/architecture.md`, `context/patterns.md`,
    `context/glossary.md`, `context/context-map.md`, `context/tmp/.gitignore`)
    with content matching the current constants; a domain-local unit test
    asserts the directory/file counts and each relative path;
    `./scripts/check-cli-architecture.sh` passes with the new file present.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml domain::context`; `./scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/domain/context/baseline.rs` (`BaselineFile`,
    `ContextBaseline`, `ContextBaseline::sce_default()`, with the
    `CONTEXT_*_TEMPLATE`/`CONTEXT_TMP_GITIGNORE_CONTENT` content copied
    verbatim from `cli/src/services/setup/mod.rs`) and
    `cli/src/domain/context/mod.rs` (`pub(crate) mod baseline;`, no
    re-export yet per the "does not wire the new type into anything yet"
    boundary — an unused `pub(crate) use` would trip the workspace's
    `warnings = "deny"` lint before T02 exists to consume it). Wired
    `pub(crate) mod context;` into `cli/src/domain/mod.rs`. No other files
    changed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml domain::context` → 2 passed (`sce_default_has_the_canonical_directories_and_files`, `sce_default_file_content_matches_legacy_templates`).
    - `./scripts/check-cli-architecture.sh` → passed.

- [x] T02: `Add the application ContextStore port` (status:done)
  - Task ID: T02
  - Goal: Define `ContextStore` (a trait with an associated `Error` type and
    an `ensure_baseline(&self, repository_root: &Path, baseline:
    &ContextBaseline) -> Result<ContextBaselineChanges, Self::Error>` method)
    and `ContextBaselineChanges` (`created_directories`,
    `existing_directories`, `created_files`, `existing_files`, all
    `Vec<PathBuf>`) in `cli/src/application/ports/context_store.rs`. Wire
    `pub(crate) mod context_store;` into `cli/src/application/ports/mod.rs`.
  - Boundaries (in/out of scope): In — the port file and its `mod`
    declaration only. Out — any concrete implementation (T04), the use case
    (T03).
  - Dependencies: T01
  - Done when: the port compiles against `crate::domain::context::ContextBaseline`
    with no `crate::services` or `crate::adapters` dependency;
    `./scripts/check-cli-architecture.sh` passes.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`; `./scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/application/ports/context_store.rs`
    (`ContextBaselineChanges`, `ContextStore` trait with associated `Error`
    and `ensure_baseline`, importing only `crate::domain::context::baseline::ContextBaseline`)
    and wired `pub(crate) mod context_store;` into
    `cli/src/application/ports/mod.rs`. No other files changed. Both new
    items carry `#[allow(dead_code)]` (consistent with existing repo
    convention, e.g. `cli/src/services/default_paths.rs`) since nothing
    constructs or implements them until T03/T04.
    - `./scripts/check-cli-architecture.sh` → passed.
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` with
      `RUSTFLAGS="--cap-lints warn"` → 0 type errors, confirming the port
      compiles cleanly against `ContextBaseline` with no
      `crate::services`/`crate::adapters` dependency.
    - Plain `./scripts/run-cli-cargo.sh build`/`test` (workspace
      `warnings = "deny"`) still fails, but only on the 7 pre-existing
      `never used`/`never constructed` errors in
      `cli/src/domain/context/baseline.rs` left by T01 (confirmed by
      reverting this task's two changes and re-running: same 7 errors,
      unchanged). No error originates from this task's new file. This is
      expected transitional state for the slice — `ContextBaseline` and
      `ContextStore` stay unconsumed by production code until T03 (use
      case) and T05 (compatibility facade) wire them in, the same pattern
      already called out for `RepoPaths::context_*` in this plan's
      Assumptions.

- [x] T03: `Add the EnsureContextBaseline use case` (status:done)
  - Task ID: T03
  - Goal: Define `EnsureContextBaseline<S: ContextStore>`,
    `EnsureContextBaselineRequest { repository_root: PathBuf }`, and
    `EnsureContextBaselineReport { repository_root: PathBuf, changes:
    ContextBaselineChanges }` in
    `cli/src/application/use_cases/ensure_context_baseline.rs`, with
    `execute` calling `ContextBaseline::sce_default()` and delegating to the
    injected `ContextStore`. Wire `pub(crate) mod ensure_context_baseline;`
    into `cli/src/application/use_cases/mod.rs`.
  - Boundaries (in/out of scope): In — the use-case file and its `mod`
    declaration. Out — any concrete `ContextStore` implementation or
    call-site wiring.
  - Dependencies: T02
  - Done when: `EnsureContextBaseline::execute` compiles generically over any
    `S: ContextStore` with no `crate::services`/`crate::adapters` import;
    a use-case-level unit test with an in-memory fake `ContextStore`
    confirms `execute` calls `ensure_baseline` with the resolved repository
    root and `ContextBaseline::sce_default()`; `./scripts/check-cli-architecture.sh`
    passes.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ensure_context_baseline`; `./scripts/check-cli-architecture.sh`.
  - Evidence: Added `cli/src/application/use_cases/ensure_context_baseline.rs`
    (`EnsureContextBaselineRequest`, `EnsureContextBaselineReport`,
    `EnsureContextBaseline<S: ContextStore>` with `new`/`execute`; `execute`
    calls `ContextBaseline::sce_default()` and delegates to the injected
    `ContextStore::ensure_baseline`, returning
    `Result<EnsureContextBaselineReport, S::Error>` per the plan's
    Assumptions; imports only `crate::application::ports::context_store` and
    `crate::domain::context::baseline`) and a unit test using an in-memory
    `FakeContextStore` that records call arguments and asserts `execute`
    calls `ensure_baseline` with the resolved repository root and
    `ContextBaseline::sce_default()`. Wired
    `pub(crate) mod ensure_context_baseline;` into
    `cli/src/application/use_cases/mod.rs`. All new items carry
    `#[allow(dead_code)]` since nothing constructs `EnsureContextBaseline`
    from production code until T05, consistent with T01/T02. No other files
    changed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ensure_context_baseline` → 1 passed (`execute_calls_ensure_baseline_with_resolved_root_and_default_baseline`).
    - `./scripts/check-cli-architecture.sh` → passed.

- [x] T04: `Add the FilesystemContextStore outbound adapter` (status:done)
  - Task ID: T04
  - Goal: Implement `ContextStore` for a new `FilesystemContextStore` in
    `cli/src/adapters/outbound/filesystem/context_store.rs`: for each
    baseline directory, create it if missing via `fs::create_dir_all` and
    record created-vs-existing; for each baseline file, skip (record
    existing) if the path exists, otherwise create parent directories and
    write the initial content (record created); return a path-specific error
    type on I/O failure. Add `cli/src/adapters/outbound/filesystem/mod.rs`
    and wire `pub(crate) mod filesystem;` into
    `cli/src/adapters/outbound/mod.rs`.
  - Boundaries (in/out of scope): In — the adapter file, its `mod`
    declaration, and its own unit tests. Out — the compatibility facade
    wiring in `services::setup` (T05).
  - Dependencies: T03
  - Done when: adapter unit tests (temp-directory based) prove: a missing
    baseline is fully created; an existing file with custom content is left
    byte-for-byte unchanged on rerun; a second `ensure_baseline` call against
    an already-bootstrapped tree reports every path under
    `existing_directories`/`existing_files` and nothing under
    `created_directories`/`created_files`; a partially-bootstrapped tree
    (some paths present, some missing) creates only the missing ones.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::context_store`.
  - Evidence: Added `cli/src/adapters/outbound/filesystem/context_store.rs`
    (`ContextStoreError { path, source: std::io::Error }` implementing
    `std::error::Error`/`Display`; `FilesystemContextStore`, a unit struct
    implementing `ContextStore` by joining each baseline directory/file's
    relative path against `repository_root`, creating missing directories
    via `fs::create_dir_all`, skipping existing files untouched, and writing
    missing files after creating their parent directory, recording each path
    under the matching `created_*`/`existing_*` field) and
    `cli/src/adapters/outbound/filesystem/mod.rs`
    (`pub(crate) mod context_store;`). Wired
    `pub(crate) mod filesystem;` into `cli/src/adapters/outbound/mod.rs`.
    Four temp-directory-based unit tests cover full creation, byte-for-byte
    idempotency of custom content, second-run existing-path reporting, and
    partial-tree creation of only missing paths (using the repository's
    existing hand-rolled `unique_temp_dir` helper pattern from
    `services::setup::tests`, since the crate has no `tempfile` dev-dependency).
    Also changed `mod ports;` to `pub(crate) mod ports;` in
    `cli/src/application/mod.rs` — required so `crate::adapters` (an outward
    layer per `context/architecture.md`'s "Adapters depend inward on ports
    the application layer owns") can reference
    `crate::application::ports::context_store`; `use_cases` remained
    unaffected since it lives inside `application` and already had access.
    No other files changed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::context_store` → 4 passed (`ensure_baseline_creates_a_missing_baseline_fully`, `ensure_baseline_leaves_existing_custom_content_byte_for_byte_unchanged`, `ensure_baseline_second_run_reports_only_existing_paths`, `ensure_baseline_creates_only_missing_paths_in_a_partially_bootstrapped_tree`).
    - `./scripts/check-cli-architecture.sh` → passed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (full suite) → 190 passed, 0 failed.
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings` → clean.

- [x] T05: `Wire the inbound renderer and compatibility facade, retire the legacy implementation` (status:done)
  - Task ID: T05
  - Goal: Add `render_context_baseline_report(report: &EnsureContextBaselineReport)
    -> String` in `cli/src/adapters/inbound/cli/setup.rs` (returning
    `services::style::success("Context baseline ensured.")`, unchanged
    text/styling), wired via `pub(crate) mod setup;` in
    `cli/src/adapters/inbound/cli/mod.rs`. Reduce
    `services::setup::bootstrap_context_baseline` to a facade that
    constructs a `FilesystemContextStore`, runs
    `EnsureContextBaseline::execute`, maps any error through
    `anyhow::Context` (same error-message shape as today), and renders the
    report. Remove the now-superseded `CONTEXT_*_TEMPLATE` constants,
    `CONTEXT_TMP_GITIGNORE_CONTENT`, `ensure_context_directory`, and
    `ensure_context_file` from `cli/src/services/setup/mod.rs`. Remove the
    now-unused `RepoPaths::context_*` accessors from
    `cli/src/services/default_paths.rs` and update the two
    `bootstrap_context_baseline_*` tests in `services/setup/mod.rs` to
    construct expected paths by joining the repo root directly instead of
    through `RepoPaths`.
  - Boundaries (in/out of scope): In — the files named above. Out — anything
    in `services/setup/command.rs` (its existing call to
    `bootstrap_context_baseline` needs no change), Clap parsing, prompts,
    lifecycle providers.
  - Dependencies: T04
  - Done when: `bootstrap_context_baseline_creates_expected_paths`,
    `bootstrap_context_baseline_is_additive_and_idempotent`,
    `resolve_setup_request_accepts_bootstrap_context_alone`,
    `resolve_setup_request_rejects_bootstrap_context_with_target`,
    `parser_routes_bootstrap_context_to_context_only_request`, and
    `help_documents_bootstrap_context_flag` all continue to pass unchanged;
    `cargo build`/`clippy --all-targets --all-features -D warnings` is clean
    (no dead-code warnings from removed `RepoPaths` accessors or removed
    constants); `./scripts/check-cli-architecture.sh` and
    `./scripts/test-check-cli-architecture.sh` pass.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`; `./scripts/check-cli-architecture.sh`; `./scripts/test-check-cli-architecture.sh`.
  - Evidence: Added `cli/src/adapters/inbound/cli/setup.rs`
    (`render_context_baseline_report`, returning
    `services::style::success("Context baseline ensured.")` unchanged) wired
    via `pub(crate) mod setup;` in `cli/src/adapters/inbound/cli/mod.rs`.
    Rewrote `services::setup::bootstrap_context_baseline` in
    `cli/src/services/setup/mod.rs` to construct a `FilesystemContextStore`,
    run `EnsureContextBaseline::execute`, map any error through
    `anyhow::Context` (same message shape as the legacy
    `ensure_context_directory`/`ensure_context_file` `.with_context` calls),
    and render via `render_context_baseline_report`. Removed the superseded
    `CONTEXT_OVERVIEW_TEMPLATE`, `CONTEXT_ARCHITECTURE_TEMPLATE`,
    `CONTEXT_PATTERNS_TEMPLATE`, `CONTEXT_GLOSSARY_TEMPLATE`,
    `CONTEXT_MAP_TEMPLATE`, `CONTEXT_TMP_GITIGNORE_CONTENT`,
    `ensure_context_directory`, and `ensure_context_file` from
    `cli/src/services/setup/mod.rs`. Removed the now-unused
    `RepoPaths::context_dir`/`context_plans_dir`/`context_decisions_dir`/
    `context_handovers_dir`/`context_tmp_dir`/`context_overview_file`/
    `context_architecture_file`/`context_glossary_file`/`context_patterns_file`/
    `context_map_file`/`context_tmp_gitignore_file` accessors from
    `cli/src/services/default_paths.rs`, along with the `context_dir` module
    and the now-unused `context_file` constants those accessors alone
    consumed (kept `context_file::SKILL_DEFINITION`, which has an unrelated
    caller). Updated `assert_baseline_paths_exist`,
    `bootstrap_context_baseline_creates_expected_paths`, and
    `bootstrap_context_baseline_is_additive_and_idempotent` in
    `services/setup/mod.rs` to construct expected paths via
    `repo.join("context/...")` instead of `RepoPaths` accessors.
    Bumped `mod` visibility to `pub(crate)` on `adapters::inbound`,
    `adapters::outbound`, `adapters::inbound::cli`, and
    `application::use_cases` (in `cli/src/adapters/mod.rs`,
    `cli/src/adapters/inbound/mod.rs`, and `cli/src/application/mod.rs`) so
    the facade in `services::setup` — a sibling of `adapters`/`application`
    under the crate root, not a descendant — can reach
    `EnsureContextBaseline` and `FilesystemContextStore`; this mirrors T04's
    identical fix for `application::ports`. Removed the now-inaccurate
    `#[allow(dead_code)] // consumed starting with the compatibility facade
    (T05)` attributes from `cli/src/application/use_cases/ensure_context_baseline.rs`
    and `cli/src/adapters/outbound/filesystem/context_store.rs`, since those
    items are genuinely consumed by the facade as of this task.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` → 190
      passed, 0 failed, including
      `bootstrap_context_baseline_creates_expected_paths`,
      `bootstrap_context_baseline_is_additive_and_idempotent`,
      `resolve_setup_request_accepts_bootstrap_context_alone`,
      `resolve_setup_request_rejects_bootstrap_context_with_target`,
      `parser_routes_bootstrap_context_to_context_only_request`, and
      `help_documents_bootstrap_context_flag`.
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
      --all-targets --all-features -- -D warnings` → clean.
    - `./scripts/check-cli-architecture.sh` → passed.
    - `./scripts/test-check-cli-architecture.sh` → all 8 assertions passed.
    - `grep -n "CONTEXT_OVERVIEW_TEMPLATE\|CONTEXT_MAP_TEMPLATE\|CONTEXT_TMP_GITIGNORE_CONTENT" cli/src/services/setup/mod.rs` → no matches (AC1).
    - `grep -n "Context baseline ensured" cli/src/adapters/inbound/cli/setup.rs` → present (AC8).

## Open questions

None. The change request specifies the target file layout, type shapes, the
single existing call site both setup paths already share, the exact output
compatibility requirement, and an explicit non-goals list; the one material
implementation choice this plan had to resolve on its own — what happens to
`RepoPaths`'s now-unused `context_*` accessors once the adapter stops calling
them — is recorded under `Assumptions` rather than left open, since leaving
them in place would fail the repository's existing clean-build bar
(`clippy --all-targets --all-features -D warnings`) for no benefit.

Separately, `context/cli/default-path-catalog.md` claims
`cli/src/services/default_paths.rs` "includes a regression test that scans
non-test Rust source under `cli/src/` and fails when new centralized
production path literals appear outside the default-path service." No such
test exists anywhere in the repository today. That drift is unrelated to this
plan's scope (it predates this change and isn't touched by it) and is called
out here only so it isn't mistaken for a constraint this plan must satisfy;
correcting it is a separate, later context-hygiene fix.

## Validation Report

**Status:** validated  
**Date:** 2026-08-04

### Commands run

- `nix flake check` -> exit 0 (all checks passed, including `checks.x86_64-linux.cli-fmt`, `checks.x86_64-linux.cli-architecture`, and `checks.x86_64-linux.cli-tests`; new files under `cli/src` were staged with `git add` for the duration of this run since the flake's Nix source only sees git-tracked content, then unstaged again afterward)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (190 passed, 0 failed)
- `./scripts/check-cli-architecture.sh` -> exit 0 (no forbidden dependencies in domain or application layers)
- `./scripts/test-check-cli-architecture.sh` -> exit 0 (all 8 assertions passed)
- `test -f cli/src/domain/context/baseline.rs; grep -n "CONTEXT_OVERVIEW_TEMPLATE\|CONTEXT_MAP_TEMPLATE\|CONTEXT_TMP_GITIGNORE_CONTENT" cli/src/services/setup/mod.rs` -> exit 1/no match (AC1 satisfied)
- `grep -n "crate::services\|crate::adapters\|std::fs" cli/src/application/use_cases/ensure_context_baseline.rs` -> exit 1/no match (AC2 satisfied)
- `grep -rn "std::fs\|Path::exists\|fs::create_dir_all\|fs::write" cli/src/domain cli/src/application` -> exit 1/no match (AC3 satisfied)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml bootstrap_context_baseline_is_additive_and_idempotent` -> exit 0, 1 passed (AC4)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml bootstrap_context_baseline_creates_expected_paths` -> exit 0, 1 passed (AC5)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml filesystem::context_store` -> exit 0, 4 passed (AC6)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resolve_setup_request_accepts_bootstrap_context_alone` -> exit 0, 1 passed (AC7)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml parser_routes_bootstrap_context_to_context_only_request` -> exit 0, 1 passed (AC7)
- `grep -n "Context baseline ensured" cli/src/adapters/inbound/cli/setup.rs` -> exit 0, match found (AC8)

### Scaffolding removed

None.

### Success-criteria verification

- [x] AC1: canonical manifest defined once in `baseline.rs`, absent from `services/setup/mod.rs` -> grep confirms no `CONTEXT_*_TEMPLATE` matches remain in `services/setup/mod.rs`; `baseline.rs` exists.
- [x] AC2: `EnsureContextBaseline::execute` has no `crate::services`/`crate::adapters`/`std::fs` import -> grep confirms no matches; `check-cli-architecture.sh` passed.
- [x] AC3: filesystem access confined to the outbound adapter -> grep across `cli/src/domain` and `cli/src/application` confirms no matches.
- [x] AC4: idempotent/additive on custom content -> `bootstrap_context_baseline_is_additive_and_idempotent` passed.
- [x] AC5: full creation when missing -> `bootstrap_context_baseline_creates_expected_paths` passed.
- [x] AC6: second run reports existing paths, not created -> `filesystem::context_store` suite (4 tests, including `ensure_baseline_second_run_reports_only_existing_paths`) passed.
- [x] AC7: both `sce setup --bootstrap-context` and normal setup route through the migrated use case -> `resolve_setup_request_accepts_bootstrap_context_alone`, `parser_routes_bootstrap_context_to_context_only_request`, `bootstrap_context_baseline_creates_expected_paths`, and `bootstrap_context_baseline_is_additive_and_idempotent` all passed unchanged.
- [x] AC8: unchanged success message/styling -> grep confirms `"Context baseline ensured."` in `cli/src/adapters/inbound/cli/setup.rs`; existing `message.contains(...)` assertions in the passing test suite confirm it.

### Failed checks and follow-ups

None.

### Residual risks

- None identified; all functional tests, the architecture gate, `nix flake check` (including formatting), and every acceptance criterion's own check passed.
