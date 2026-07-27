# Plan: generate-cli-assets-in-cargo-out-dir

## Change summary

Remove the committed generated target trees at `config/.opencode`, `config/.claude`, and `config/.pi`. Repository builds will evaluate the canonical Pkl model directly into Cargo's ephemeral `OUT_DIR`; `build.rs` will stage the remaining required static inputs there, and Rust will include generated source and static payloads only from `OUT_DIR`.

Generated target files will no longer be repository artifacts or parity snapshots. Published crates remain self-contained through a packaging-only payload generated from Pkl in a temporary clean workspace before `cargo package`/`cargo publish`; downstream builds copy that packaged payload into their own `OUT_DIR` and do not require Pkl.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: `config/.opencode`, `config/.claude`, and `config/.pi` are absent from the repository, and generated workflow, skill, entry-point, plugin, settings, extension, and config-schema outputs exist only in temporary generation/build/package locations.
  - Validate: `test ! -e config/.opencode && test ! -e config/.claude && test ! -e config/.pi`; run the generation check and inspect `git status --short` to confirm it does not recreate them.
- [ ] AC2: A repository CLI build runs the canonical Pkl generator into Cargo `OUT_DIR`, stages required non-Pkl inputs there, and writes no generated Rust or static files into `cli/src/`, `cli/assets/`, or `config/`.
  - Validate: `nix develop -c sh -c 'cd cli && cargo clean && cargo build'`; inspect `cli/target/debug/build/shared-context-engineering-*/out/` and run `git status --short cli/src cli/assets config`.
- [ ] AC3: The built CLI embeds the deterministic OpenCode, Claude, Pi, hook, policy, schema, and migration payloads expected by setup and runtime consumers.
  - Validate: `nix develop -c sh -c 'cd cli && cargo test setup && cargo test config && cargo test agent_trace && cargo test auth_db'` and the ephemeral Pkl inventory check.
- [ ] AC4: A staged crates.io package embeds a packaging-only Pkl-generated payload, and the unpacked crate builds without Pkl, the repository-level Pkl sources, or parent-directory `config/` paths.
  - Validate: prepare a temporary clean repository copy, run the package-preparation and `cargo package` flow, inspect `cargo package --list`, unpack the `.crate`, remove Pkl from `PATH`, and build it with Cargo.
- [ ] AC5: Nix package/check builds and Flatpak source builds consume direct `OUT_DIR` generation or an explicitly prepared ephemeral fallback without relying on committed generated target trees.
  - Validate: `nix flake check`; on Linux, `nix run .#sce-flatpak -- validate`; inspect Nix, Flatpak, and release definitions for references to removed `config/.opencode`, `config/.claude`, or `config/.pi` trees.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Add a decision record superseding the checked-in generated-target ownership portions of `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` while preserving its workflow matrix and canonical Pkl ownership decisions.
- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` to describe ephemeral Pkl generation, removed generated target trees, `OUT_DIR`-only Rust embedding, and the crates.io fallback payload.
- Update `context/sce/generated-opencode-plugin-registration.md`, `context/sce/pi-extension-runtime.md`, `context/sce/shared-turso-db.md`, `context/cli/config-precedence-contract.md`, and `context/sce/cli-cargo-distribution-contract.md` where they currently identify checked-in generated paths or source-tree build artifacts.

## Constraints and non-goals

- **In scope:** canonical Pkl generation output routing; removal of `config/.opencode`, `config/.claude`, and `config/.pi`; ephemeral generation/inventory checks; `cli/build.rs`; Rust compile-time includes for setup assets, policy data, schemas, and migration manifests; Cargo package staging; Nix/Crane inputs; crates.io publishing; Flatpak preparation; focused regression coverage and contributor documentation.
- **Out of scope:** Changing workflow/skill/entry-point content, changing the OpenCode/Claude/Pi target matrix, changing runtime setup destinations, changing database migration SQL, or removing canonical Pkl and `config/lib` authoring sources.
- **Constraints:** Repository/Nix builds generate canonical Pkl outputs directly into Cargo `OUT_DIR`; all Rust compile-time generated source and static-data includes resolve from `OUT_DIR`; no generated target tree is committed; generation stays deterministic and exact-inventory checked; published crates build without Pkl or repository-external paths; normal verification remains Nix-managed.
- **Non-goal:** Bundling a Pkl executable/evaluator into the published Rust crate or preserving the removed generated directories as compatibility paths.

## Assumptions

- The previously approved crates.io exception remains in force: publication may create a temporary Pkl-generated payload inside the staged crate, but that payload is never committed and downstream `build.rs` copies it into the consumer's `OUT_DIR`.
- `nix run .#pkl-check-generated` may retain its command name for compatibility while changing from committed-output parity comparison to ephemeral generation, exact-inventory validation, determinism checking, and rejection of the removed target directories.
- Non-Pkl inputs required at compile time, such as hook templates, the Agent Trace schema, and SQL migrations, remain canonical source files and are copied by `build.rs` into `OUT_DIR`.

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

## Open questions

None. The user explicitly chose ephemeral `OUT_DIR` generation and removal of all three committed generated target trees, while retaining the earlier packaging requirement for a self-contained crates.io artifact.
