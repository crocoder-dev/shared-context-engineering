# Plan: generate-cli-assets-in-cargo-out-dir

## Change summary

Remove the committed generated target trees at `config/.opencode`, `config/.claude`, and `config/.pi`. Preserve the completed move to ephemeral assets, then move Pkl evaluation out of `cli/build.rs`: a pre-Cargo producer will generate the canonical payload in a separate temporary directory, identify that directory through an explicit handoff, and `build.rs` will validate and copy it into Cargo's invocation-specific `OUT_DIR`. Rust will continue to include generated source and static payloads only from `OUT_DIR`.

This follow-up keeps Pkl out of Cargo build-script sandboxes while retaining deterministic invalidation and the existing packaging fallback. Generated target files remain absent from the repository; published crates remain self-contained through a packaging-only payload generated from Pkl in a temporary clean workspace before `cargo package`/`cargo publish`; downstream builds copy that packaged payload into their own `OUT_DIR` and do not require Pkl.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: `config/.opencode`, `config/.claude`, and `config/.pi` are absent from the repository, and generated workflow, skill, entry-point, plugin, settings, extension, and config-schema outputs exist only in temporary generation/build/package locations.
  - Validate: `test ! -e config/.opencode && test ! -e config/.claude && test ! -e config/.pi`; run the generation check and inspect `git status --short` to confirm it does not recreate them.
- [x] AC2: A supported repository CLI build generates the canonical Pkl payload before Cargo in a separate temporary location; `cli/build.rs` invokes no Pkl subprocess, validates and copies the handed-off payload plus required non-Pkl inputs into Cargo `OUT_DIR`, and writes no generated Rust or static files into `cli/src/`, `cli/assets/`, or `config/`.
  - Validate: run the documented repository Cargo build entrypoint after `cargo clean`; inspect `cli/target/debug/build/shared-context-engineering-*/out/`, assert that `cli/build.rs` has no Pkl process invocation, and run `git status --short cli/src cli/assets config`.
- [x] AC3: The built CLI embeds the deterministic OpenCode, Claude, Pi, hook, policy, schema, and migration payloads expected by setup and runtime consumers.
  - Validate: `nix develop -c sh -c 'cd cli && cargo test setup && cargo test config && cargo test agent_trace && cargo test auth_db'` and the ephemeral Pkl inventory check.
- [x] AC4: A staged crates.io package embeds a packaging-only Pkl-generated payload, and the unpacked crate builds without Pkl, the repository-level Pkl sources, or parent-directory `config/` paths.
  - Validate: prepare a temporary clean repository copy, run the package-preparation and `cargo package` flow, inspect `cargo package --list`, unpack the `.crate`, remove Pkl from `PATH`, and build it with Cargo.
- [x] AC5: Nix package/check builds generate Pkl payloads before entering Cargo build-script execution and hand them to `build.rs` for copying into `OUT_DIR`; Flatpak source builds retain their explicitly prepared ephemeral fallback; neither path relies on committed generated target trees.
  - Validate: `nix flake check`; on Linux, `nix run .#sce-flatpak -- validate`; inspect Nix derivations to confirm Pkl runs in the pre-Cargo producer rather than `cli/build.rs`, and inspect Nix, Flatpak, and release definitions for references to removed `config/.opencode`, `config/.claude`, or `config/.pi` trees.
- [x] AC6: Documented non-Nix repository workflows can pre-generate and hand off the payload for `cargo build`, targeted tests, Clippy, and `cargo install --path cli`, with freshness represented in Cargo invalidation inputs and actionable failure when the handoff is absent or invalid.
  - Validate: run the documented pre-generation entrypoint with representative `cargo build`, targeted-test, Clippy, and `cargo install --path cli` commands; mutate one canonical Pkl input and confirm the payload is regenerated before Cargo; invoke repository Cargo without a valid handoff and assert the documented diagnostic.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Add a decision record superseding the checked-in generated-target ownership portions of `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` while preserving its workflow matrix and canonical Pkl ownership decisions.
- Update `context/decisions/2026-07-27-ephemeral-pkl-build-generation.md` to supersede direct build-script Pkl evaluation with pre-Cargo generation and a validated build-script copy boundary motivated by sandboxing.
- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` to describe pre-Cargo ephemeral Pkl generation, removed generated target trees, `OUT_DIR`-only Rust embedding, and the crates.io fallback payload.
- Update `context/sce/generated-opencode-plugin-registration.md`, `context/sce/pi-extension-runtime.md`, `context/sce/shared-turso-db.md`, `context/cli/config-precedence-contract.md`, and `context/sce/cli-cargo-distribution-contract.md` where they currently identify checked-in generated paths or source-tree build artifacts.

## Constraints and non-goals

- **In scope:** canonical Pkl generation output routing; removal of `config/.opencode`, `config/.claude`, and `config/.pi`; pre-Cargo ephemeral generation and handoff; generation/inventory checks; `cli/build.rs`; Rust compile-time includes for setup assets, policy data, schemas, and migration manifests; Cargo package staging; Nix/Crane inputs; non-Nix repository Cargo workflows; crates.io publishing; Flatpak preparation; focused regression coverage and contributor documentation.
- **Out of scope:** `cargo install --git` support; changing workflow/skill/entry-point content; changing the OpenCode/Claude/Pi target matrix; changing runtime setup destinations; changing database migration SQL; or removing canonical Pkl and `config/lib` authoring sources.
- **Constraints:** Repository/Nix builds generate canonical Pkl outputs before Cargo in a separate ephemeral directory and `build.rs` copies validated inputs into Cargo `OUT_DIR`; `build.rs` must not invoke Pkl; all Rust compile-time generated source and static-data includes resolve from `OUT_DIR`; no generated target tree is committed; generation stays deterministic and exact-inventory checked; published crates build without Pkl or repository-external paths; normal verification remains Nix-managed.
- **Non-goal:** Bundling a Pkl executable/evaluator into the published Rust crate or preserving the removed generated directories as compatibility paths.

## Assumptions

- The previously approved crates.io exception remains in force: publication may create a temporary Pkl-generated payload inside the staged crate, but that payload is never committed and downstream `build.rs` copies it into the consumer's `OUT_DIR`.
- `nix run .#pkl-check-generated` may retain its command name for compatibility while changing from committed-output parity comparison to ephemeral generation, exact-inventory validation, determinism checking, and rejection of the removed target directories.
- Non-Pkl inputs required at compile time, such as hook templates, the Agent Trace schema, and SQL migrations, remain canonical source files and are copied by `build.rs` into `OUT_DIR`.
- The pre-Cargo producer may choose its own temporary output directory; an explicit environment-variable handoff is the smallest established Cargo-compatible mechanism for telling `build.rs` what to validate and copy.
- Direct `cargo install --git` is intentionally unsupported because a git checkout has no guaranteed pre-generation step; supported source installs use `cargo install --path cli` after the repository pre-generation entrypoint.

## Task stack

- [x] T01: `Generate and embed CLI build artifacts from OUT_DIR` (status:complete)
  - Task ID: T01
  - Goal: Refactor `cli/build.rs` and Rust consumers so repository builds run Pkl into `OUT_DIR`, copy remaining static inputs there, generate setup-asset and migration Rust manifests there, and resolve every production compile-time payload through `OUT_DIR`.
  - Boundaries (in/out of scope): In — build-script source/tool discovery, deterministic Pkl invocation, rerun directives, staging of hooks/schemas/migrations, generated Rust manifests, setup/policy/config/Agent Trace include call sites, removal of `cli/src/generated_migrations.rs`, and focused embedding tests. Out — deleting committed generated target trees or changing publication/release orchestration.
  - Dependencies: none
  - Done when: A Nix-dev-shell Cargo build generates the complete target payload under its build-script `OUT_DIR`, leaves source directories unchanged, all production static includes resolve through `OUT_DIR`, and targeted setup/config/policy/Agent Trace/migration tests pass.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo clean && cargo test setup && cargo test config && cargo test agent_trace && cargo test auth_db'`; inspect `cli/target/debug/build/shared-context-engineering-*/out/`; `git status --short cli/src cli/assets config`.
  - Implementation evidence: `cli/build.rs` now evaluates `config/pkl/generate.pkl` into `OUT_DIR/pkl-generated`, stages hooks, Agent Trace schema, and migrations under `OUT_DIR/static`, and generates both Rust manifests in `OUT_DIR`; production setup, policy, config-schema, Agent Trace, and migration includes now resolve from `OUT_DIR`, and the source-tree migration manifest was removed.
  - Verification evidence: The documented clean targeted test command passed (setup 7, config 22, agent_trace 44, auth_db 2); the post-change focused setup suite passed 8 tests including generated-target/static-hook inventory coverage; `cargo clippy --all-targets -- -D warnings` passed; inspection confirmed generated target/static/manifest files under `cli/target/debug/build/shared-context-engineering-*/out/` and no build-created changes under `cli/assets` or `config`.

- [x] T02: `Embed a self-contained generated payload in published crates` (status:complete)
  - Task ID: T02
  - Goal: Make crates.io packaging generate a complete fallback payload from canonical Pkl sources in a temporary clean workspace and embed only that payload in the staged crate for Pkl-free downstream builds.
  - Boundaries (in/out of scope): In — package-preparation script, dedicated ignored package-staging path, `cli/Cargo.toml` include inventory, build-script fallback selection and diagnostics, crates.io publish workflow, clean-copy package verification, and deterministic fallback inventory checks. Out — requiring downstream Cargo users to install Pkl or retaining `cli/assets/generated/` as a normal repository-build input.
  - Dependencies: T01
  - Done when: `cargo package` contains exactly the required generated/static fallback files, an unpacked package builds with no Pkl and no parent Pkl/config tree, missing or incomplete fallback data fails actionably, and repository builds continue to prefer direct Pkl generation into `OUT_DIR`.
  - Verification notes (commands or checks): prepare a temporary clean repository copy; run the updated package-preparation script and `nix develop -c cargo package --manifest-path <copy>/cli/Cargo.toml --locked --allow-dirty`; inspect `cargo package --list`; unpack and build the `.crate` with Cargo/Rust available but Pkl absent.
  - Implementation evidence: The package-preparation script now evaluates canonical Pkl twice into an ignored `cli/package-fallback/`, rejects nondeterminism, stages hooks, schemas, and migrations, and writes an exact SHA-256 inventory; `cli/build.rs` prefers repository Pkl sources but validates and copies that fallback into `OUT_DIR` for packaged crates, while `cli/Cargo.toml` and the crates.io workflow include and prepare only the packaging payload.
  - Verification evidence: A clean temporary repository copy prepared and packaged 296 files successfully; package-list inspection found 81 fallback entries and no legacy `assets/`, `migrations/`, or parent `config/` inputs; the unpacked crate built successfully with Pkl removed from `PATH`; deleting a fallback file produced the expected actionable inventory-mismatch failure; repository `cargo test setup` passed 8 tests and `cargo clippy --all-targets -- -D warnings` passed.

- [x] T03: `Remove committed generated targets and align build channels` (status:complete)
  - Task ID: T03
  - Goal: Delete the generated OpenCode, Claude, and Pi config trees; replace committed-output parity with ephemeral deterministic inventory validation; and align Nix, Flatpak, and release builds with direct Pkl generation or packaging-only fallback preparation.
  - Boundaries (in/out of scope): In — removal of `config/.opencode`, `config/.claude`, and `config/.pi`; `config/pkl/check-generated.sh`; `pkl-parity`; source filters and build inputs in `flake.nix`; removal of `cli/assets/generated/` preparation; Flatpak manifest source and static validation; generated Flatpak manifest regeneration; release/package helper call sites; obsolete path constants/tests; and contributor documentation. Out — changing canonical workflow content, target inventories, install destinations, release artifact formats, or supported channels.
  - Dependencies: T02
  - Done when: No generated target directory or general-purpose CLI generated mirror remains in the repository, the generation check proves repeatable exact output in temporary directories and rejects legacy paths, Crane builds run the Pkl-to-`OUT_DIR` path, Flatpak has an explicit ephemeral fallback path when Pkl is unavailable, and all targeted Nix/Flatpak checks pass.
  - Verification notes (commands or checks): `test ! -e config/.opencode && test ! -e config/.claude && test ! -e config/.pi`; run `nix run .#pkl-check-generated` twice and compare its temporary inventories; `nix flake check`; on Linux, `nix run .#regenerate-flatpak-manifest` followed by `nix run .#sce-flatpak -- validate`; search active build definitions for removed generated paths.
  - Implementation evidence: Removed the committed OpenCode, Claude, Pi, generated SCE schema, and legacy `cli/assets/generated` outputs; replaced parity snapshots with two-pass ephemeral SHA-256 inventory validation; routed Crane source builds through canonical Pkl inputs and `OUT_DIR`; removed obsolete source-tree asset path helpers; and changed Flatpak manifests/helpers to stage a temporary packaging fallback before entering the Pkl-free Flatpak build sandbox.
  - Verification evidence: Two consecutive `nix run .#pkl-check-generated` runs reported the same 73-file inventory digest (`605e7d406bd56047f76b8eecff3f53e4226e208133166225c34d868de032faae`); legacy-path and diff checks passed; `nix run .#regenerate-flatpak-manifest` and `nix run .#sce-flatpak -- validate` passed; a release Flatpak source package contained the fallback inventory and representative generated payload; and `nix flake check` passed all checks.

- [x] T04: `Copy pre-generated Pkl payloads into OUT_DIR` (status:complete)
  - Task ID: T04
  - Goal: Replace repository-mode Pkl subprocess execution in `cli/build.rs` with a validated handoff from an externally generated payload directory while retaining `OUT_DIR` ownership and the packaged-fallback path.
  - Boundaries (in/out of scope): In — the generated-input environment contract, build-script validation/copying, Cargo rerun signals for the handed-off inventory and canonical inputs, actionable missing/stale/invalid-input diagnostics, and focused build-script tests. Out — implementing the producer in Nix or repository commands, changing Rust include paths, and changing the packaging fallback format.
  - Dependencies: T03
  - Done when: Repository-mode `build.rs` invokes no Pkl process, accepts only a complete deterministic generated-input handoff, copies it into `OUT_DIR/pkl-generated`, preserves static staging and packaged fallback behavior, reruns when the handoff or canonical inputs change, and focused tests cover valid, missing, incomplete, and stale handoffs.
  - Verification notes (commands or checks): run the focused build-script/setup tests through `nix develop`; inspect `cli/build.rs` for absence of `Command::new("pkl")`; exercise valid, missing, incomplete, and stale generated-input directories and compare the copied `OUT_DIR` inventory.
  - Implementation evidence: `cli/build.rs` now requires `SCE_CLI_GENERATED_INPUT_DIR` for repository builds, validates exact sorted SHA-256 payload and canonical-input inventories, copies the validated `pkl-generated` tree into Cargo `OUT_DIR`, and emits rerun signals for the handoff environment, both inventories, and every canonical generator input; packaged fallback and static staging paths remain unchanged. Focused build-script tests are exposed through `cli/tests/build_script.rs` and cover missing environment/directory, valid copy parity, incomplete payload, and stale canonical inputs.
  - Verification evidence: A temporary canonical Pkl handoff was generated outside Cargo with `SHA256SUMS` and `INPUTS.SHA256SUMS`; `nix develop` runs of `cargo test --test build_script` passed 5 tests, `cargo test setup` passed 8 tests, `cargo clippy --test build_script -- -D warnings` passed, and `cargo fmt --check` passed. Source inspection found no Pkl `Command` invocation in `cli/build.rs`, while the valid-copy test asserted identical source and `OUT_DIR/pkl-generated` inventories.

- [x] T05: `Add a repository pre-Cargo generation entrypoint` (status:complete)
  - Task ID: T05
  - Goal: Provide one documented repository command boundary that runs canonical Pkl generation in a temporary directory and invokes a requested Cargo workflow with the generated-input handoff.
  - Boundaries (in/out of scope): In — a repository-owned helper or equivalent app, temporary-directory lifecycle, deterministic generation/inventory preparation, environment handoff, support for build, targeted tests, Clippy, and `cargo install --path cli`, failure cleanup, and contributor documentation. Out — `cargo install --git`, package-fallback preparation, and Nix/Crane derivation wiring.
  - Dependencies: T04
  - Done when: Contributors outside Nix build sandboxes can use one documented entrypoint for the supported Cargo workflows, each invocation generates before Cargo and passes a validated payload directory, canonical input changes cannot silently reuse stale output, and failures leave no repository-generated tree.
  - Verification notes (commands or checks): run the helper with representative `cargo build`, targeted `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo install --path cli` invocations; mutate a temporary canonical input copy to prove regeneration; interrupt or fail Cargo and verify temporary cleanup and a clean `git status --short`.
  - Implementation evidence: Added `scripts/run-cli-cargo.sh` as the repository Cargo boundary; each invocation evaluates canonical Pkl twice into a fresh temporary directory, rejects nondeterministic output, writes exact payload and canonical-input SHA-256 inventories, passes the directory only through `SCE_CLI_GENERATED_INPUT_DIR`, forwards Cargo arguments unchanged, preserves failures, and removes the handoff on normal exit or signals. Added a fake-tool behavioral harness covering argument forwarding, fresh regeneration after a canonical-input mutation, inventory validity, Cargo failure propagation, and cleanup; updated contributor and local-checkout documentation to use the wrapper and identify direct `cargo install --git` as unsupported.
  - Verification evidence: `bash -n scripts/run-cli-cargo.sh scripts/test-run-cli-cargo.sh` passed; `bash scripts/test-run-cli-cargo.sh` passed, including the temporary canonical-input mutation and exit-42 cleanup case; wrapper-driven Nix-dev-shell `cargo build`, targeted `cargo test setup` (8 passed), and `cargo clippy --all-targets -- -D warnings` all passed; wrapper-driven `cargo install --path cli --locked` installed an executable into a temporary root; `git diff --check`, removed-generated-path assertions, and `git status --short cli/src cli/assets config scripts AGENTS.md cli/README.md` confirmed no generated source tree was left behind.

- [x] T06: `Pre-generate Pkl payloads for Crane builds` (status:complete)
  - Task ID: T06
  - Goal: Move Pkl evaluation for native, release, test, and Clippy derivations into a pre-Cargo Nix boundary and pass the resulting directory to `cli/build.rs` without exposing Pkl inside Cargo build-script execution.
  - Boundaries (in/out of scope): In — `flake.nix` source/input partitioning, reusable pre-generated payload derivation or hook, handoff environment, native/release/check derivations, cache/invalidation behavior, and focused Nix assertions. Out — changing dependency-only Cargo artifacts, published-crate fallback generation, Flatpak's existing Pkl-free fallback, and release artifact formats.
  - Dependencies: T04
  - Done when: Every Crane derivation that compiles the CLI receives the same canonical pre-generated payload contract, Pkl is absent from the Cargo build-script environment, canonical Pkl changes invalidate generation and dependent builds, dependency artifacts remain shared where applicable, and native, release, test, Clippy, and format topology remains intact.
  - Verification notes (commands or checks): inspect derivation inputs/environment; run the narrow Crane package/test/Clippy checks and `nix flake check`; change a canonical Pkl input in a temporary worktree and inspect rebuild boundaries; confirm Flatpak and packaged-crate fallback checks still use their existing Pkl-free path.
  - Implementation evidence: `flake.nix` now builds one canonical, deterministic `sce-cli-generated-input` derivation from the isolated Pkl/plugin fileset, writes the payload and canonical-input SHA-256 inventories, and passes its store path through `SCE_CLI_GENERATED_INPUT_DIR` to native, native-release, musl-release, test, and Clippy Cargo derivations. Pkl was removed from `commonCargoArgs`; a pre-Cargo assertion rejects any compiling derivation whose `PATH` still exposes Pkl. Dependency-only artifacts and the format derivation remain outside the generated-input dependency, and a focused `cli-generated-input` check validates target presence plus both inventories.
  - Verification evidence: `nix build --no-link .#checks.x86_64-linux.cli-generated-input .#sce .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy` passed; `nix build --no-link .#sce-release` passed; derivation inspection showed the same generated-input store path on native, release, test, and Clippy derivations, no Pkl native build input, and no handoff on `cli-fmt`. A temporary canonical Pkl comment changed the generated-input, native package, test, and Clippy derivation paths while host and musl dependency-artifact paths plus `cli-fmt` stayed unchanged; the probe was then removed. `nix flake check` passed all checks, `nix run .#sce-flatpak -- validate` passed using its deterministic packaging-only fallback, and diff inspection confirmed no Flatpak or packaged-crate fallback definitions changed.

## Open questions

None. The user explicitly chose pre-Cargo generation into a separate directory with `build.rs` copying into `OUT_DIR`, excluded `cargo install --git`, and identified sandboxing as the reason to move Pkl out of the build script.

## Validation Report

**Status:** validated
**Date:** 2026-07-27

### Commands run

- `test ! -e config/.opencode && test ! -e config/.claude && test ! -e config/.pi` -> exit 0 (all removed generated target trees are absent)
- `bash -n scripts/run-cli-cargo.sh scripts/test-run-cli-cargo.sh && bash scripts/test-run-cli-cargo.sh` -> exit 0 (wrapper syntax, argument forwarding, canonical-input mutation/regeneration, inventory, failure propagation, and cleanup checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (73-file deterministic ephemeral inventory passed with digest `605e7d406bd56047f76b8eecff3f53e4226e208133166225c34d868de032faae`)
- `nix flake check` -> exit 0 (all compatible-system flake checks passed)
- `nix run .#sce-flatpak -- validate` -> exit 0 (deterministic packaging fallback, Flatpak validation, and local manifest check passed)
- `nix develop -c ./scripts/run-cli-cargo.sh clean --manifest-path cli/Cargo.toml && nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` -> exit 0 (clean repository build generated the handoff before Cargo and built successfully)
- `git status --short cli/src cli/assets config` -> exit 0 (no build-created source, asset, or config changes)
- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_db && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml --test build_script'` -> exit 0 (setup 8, config 22, agent_trace 44, auth_db 2, and build-script handoff 5 tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` -> exit 0 (Clippy passed with warnings denied)
- `nix develop -c ./scripts/run-cli-cargo.sh install --path cli --locked --root "$install_root"` plus executable assertion and cleanup -> exit 0 (`sce` installed to a temporary root and was executable)
- Initial temporary package-list harness using `rg` -> exit 2 (`rg` was unavailable outside the Nix command; no product check failed, and the harness was replaced with shell-only inspection)
- Initial shell-only package-list inspection -> exit 1 (the inspection incorrectly rejected expected `package-fallback/pkl-generated/config/**` entries; the corrected inspection rejects only repository-level `config/` paths)
- Corrected clean-copy package preparation, `cargo package --list`, `cargo package`, archive inspection, Pkl removal from `PATH`, and unpacked-crate `cargo build --locked` flow -> exit 0 (296 files packaged, exactly 81 fallback entries inspected, and the unpacked crate built without Pkl or parent `config/` inputs)
- `nix develop -c env -u SCE_CLI_GENERATED_INPUT_DIR cargo check --manifest-path cli/Cargo.toml` -> exit 101 as expected (build failed actionably, requiring `SCE_CLI_GENERATED_INPUT_DIR` with `pkl-generated/`, `SHA256SUMS`, and `INPUTS.SHA256SUMS`)
- Initial `nix derivation show ... | jq` derivation inspection -> exit 5 (inspection used the wrong JSON root; no derivation failed)
- Corrected `nix derivation show .#sce .#sce-release .#checks.x86_64-linux.cli-tests .#checks.x86_64-linux.cli-clippy | jq -e ...` -> exit 0 (all four compiling derivations share one generated-input handoff, omit Pkl from native build inputs, and enforce Pkl absence before Cargo)
- `git diff --check` -> exit 0 (no whitespace errors)

### Scaffolding removed

- `/tmp/sce-package-validation.*` — temporary clean repository copy, package list, crate archive, and unpacked build tree used only for validation
- `/tmp/sce-validation-install.*` — temporary Cargo installation root used only for validation
- `/tmp/sce-cli-generated-input.*` — wrapper-generated handoff directories; cleanup assertions confirmed none remained

### Success-criteria verification

- [x] AC1: `config/.opencode`, `config/.claude`, and `config/.pi` are absent from the repository, and generated outputs exist only in temporary generation/build/package locations -> absence assertions and post-generation `git status` passed; the ephemeral inventory check did not recreate removed trees.
- [x] AC2: Supported repository builds generate before Cargo, while `build.rs` only validates/copies into `OUT_DIR` and leaves source trees unchanged -> the clean wrapper build passed; `OUT_DIR` contains `pkl-generated/`, `static/`, `setup_embedded_assets.rs`, and `generated_migrations.rs`; source inspection found no Pkl process invocation; focused status was clean.
- [x] AC3: The CLI embeds deterministic OpenCode, Claude, Pi, hook, policy, schema, and migration payloads -> the 73-file deterministic inventory and all authored setup/config/Agent Trace/auth DB suites passed; `OUT_DIR` inspection confirmed all three generated targets, schema, hooks, migrations, and generated manifests.
- [x] AC4: The staged crates.io package carries a packaging-only payload and builds without Pkl or repository parent inputs -> clean-copy packaging produced 296 files with exactly 81 fallback entries, no repository-level `assets/generated`, `migrations`, or `config` paths, and the unpacked crate built with Pkl removed from `PATH`.
- [x] AC5: Nix generates and hands off before Cargo, and Flatpak uses its explicit fallback without committed targets -> both full Nix and Flatpak validation passed; derivation inspection confirmed one pre-Cargo handoff and Pkl exclusion; Nix/Flatpak/release inspection found no reliance on committed generated trees.
- [x] AC6: Documented wrapper workflows support build, tests, Clippy, and local install with freshness and actionable invalid-handoff behavior -> all representative wrapper commands passed; the behavioral harness proved regeneration after canonical-input mutation and cleanup; build-script tests covered valid, missing, incomplete, and stale handoffs; direct Cargo without the handoff emitted the documented diagnostic.

### Failed checks and follow-ups

- None.

### Residual risks

- `nix flake check` reported that `aarch64-darwin`, `aarch64-linux`, and `x86_64-darwin` are incompatible with this host and were not checked locally.
- Flatpak validation skipped optional `flatpak-builder-lint` because it is not installed; required Flatpak validation still passed.
