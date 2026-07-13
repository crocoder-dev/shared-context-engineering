# Plan: pi-harness-integration

## Change summary

Add the Pi coding-agent harness as a first-class SCE integration target alongside OpenCode and Claude. Pi consumes project-local configuration from `.pi/`: slash commands are Markdown prompt templates in `.pi/prompts/`, and skills are Agent Skills-format `SKILL.md` packages in `.pi/skills/`. Pi has no native sub-agent format, so SCE agents (shared-context-plan, shared-context-code) are rendered as agent-role prompt templates (`.pi/prompts/agent-*.md`) that instruct Pi to act in that role within the current session.

The change follows the existing target pattern end to end: a new Pkl renderer produces a canonical `config/.pi/` tree, the asset-prep script syncs it into `cli/assets/generated/config/pi`, `build.rs` embeds it, `sce setup --pi` installs it and persists `"pi"` into `integrations.target`, and `sce doctor` detects and health-checks the Pi installation.

Decisions resolved with the user (2026-07-13):
- Generate a dedicated `.pi/skills/` tree from Pkl (no `.pi/settings.json` redirection to `.claude/skills`).
- Render SCE agents as agent-role prompt templates in `.pi/prompts/`.
- CLI: add `--pi` and a new `--all` flag.
- Updated during T04 (2026-07-13, user decision): remove `--both` entirely (flag, `SetupTarget::Both`, and interactive option); `--all` (opencode+claude+pi) replaces it. Interactive prompt offers OpenCode, Claude, Pi, All.
- Do not generate or append `AGENTS.md`; skills and prompts carry the workflow.

## Success criteria

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` deterministically emits `config/.pi/prompts/*.md` (commands + agent-role prompts) and `config/.pi/skills/*/SKILL.md`, and a clean re-run produces no diff.
- `nix develop -c ./config/pkl/check-generated.sh` covers the new Pi outputs and passes.
- `sce setup --pi` installs the Pi assets into the target repo's `.pi/` directory and persists `"pi"` in `.sce/config.json` `integrations.target`.
- `sce setup --all` installs opencode, claude, and pi targets; `--both` is removed and rejected as an unknown flag.
- `sce doctor` reports Pi integration health (prompts, skills) when `"pi"` is configured or a `.pi/` directory is detected, and its remediation text mentions `sce setup --pi`.
- `.sce/config.json` schema validation accepts `"pi"` and still rejects unknown target IDs.
- Full workspace checks pass (`cargo test`, pkl parity via `nix run .#pkl-check-generated` or `nix flake check`).

## Constraints and non-goals

- Manual profile only: no Pi equivalent of the OpenCode automated profile in this change.
- No hooks for Pi (Pi extensions are out of scope per the source write-up; the Claude hook mechanism has no Pi analogue here).
- No `.pi/SYSTEM.md`, `.pi/APPEND_SYSTEM.md`, `.pi/settings.json`, or root `AGENTS.md` generation.
- No Pi packaging (`pi install` npm/Git package) support.
- Do not change existing OpenCode/Claude generated outputs. (`--both` removal was explicitly approved during T04.)
- Do not hand-edit generated artifacts; all content changes go through Pkl sources.

## Assumptions

- Pi auto-discovers `.pi/prompts/` and `.pi/skills/` in a trusted repo without extra registration (per the provided write-up).
- Pi prompt-template frontmatter uses `description` and optional `argument-hint`; arguments use `$ARGUMENTS`/`$1` syntax, which maps cleanly from existing Claude command templates.

## Task stack

- [x] T01: `Add Pi Pkl renderer and generate config/.pi tree` (status:done)
  - Task ID: T01
  - Completed: 2026-07-13
  - Files changed: config/pkl/renderers/pi-content.pkl (new), config/pkl/renderers/pi-metadata.pkl (new), config/pkl/generate.pkl, config/pkl/check-generated.sh, config/pkl/renderers/metadata-coverage-check.pkl, config/.pi/** (generated: 5 command prompts, 2 agent-role prompts, 8 SKILL.md)
  - Evidence: `pkl eval -m . config/pkl/generate.pkl` emitted config/.pi tree; re-run diff-clean; `nix develop -c ./config/pkl/check-generated.sh` passed covering config/.pi/prompts + config/.pi/skills; metadata-coverage-check.pkl evaluates with pi coverage blocks; spot-checked frontmatter (`description` + `argument-hint`) and `$ARGUMENTS` usage.
  - Goal: Render the canonical Pi target tree from the shared Pkl sources: `config/.pi/skills/{slug}/SKILL.md` for each SCE skill, `config/.pi/prompts/{slug}.md` for each SCE command, and `config/.pi/prompts/agent-{slug}.md` agent-role prompt templates for shared-context-plan and shared-context-code.
  - Boundaries (in/out of scope): In — new `config/pkl/renderers/pi-content.pkl` and `pi-metadata.pkl` (mirroring the claude renderer pair), wiring into `config/pkl/generate.pkl`, extending the `paths` array in `config/pkl/check-generated.sh`, and updating `metadata-coverage-check.pkl` if it enumerates targets. Agent-role prompts adapt agent bodies to Pi semantics: act-as-role instructions, `$ARGUMENTS` argument passing, reference to loading the matching skill. Out — CLI/Rust changes, asset sync, automated profile.
  - Done when: `nix develop -c pkl eval -m . config/pkl/generate.pkl` writes the `config/.pi/` tree; re-running produces no diff; `nix develop -c ./config/pkl/check-generated.sh` passes and covers Pi paths; generated prompt frontmatter contains valid `description` (and `argument-hint` where the source command takes arguments).
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `git status --short config/.pi`; `nix develop -c ./config/pkl/check-generated.sh`; manual read of one command prompt, one agent-role prompt, and one SKILL.md for correct Pi frontmatter and argument syntax.

- [x] T02: `Sync and embed Pi assets into the CLI binary` (status:done)
  - Task ID: T02
  - Completed: 2026-07-13
  - Files changed: scripts/prepare-cli-generated-assets.sh, cli/build.rs, flake.nix (Nix-build asset staging for config/.pi), cli/assets/generated/config/pi/** (synced: 7 prompts, 8 SKILL.md), cli/assets/generated/config/schema/sce-config.schema.json (pre-existing mirror drift from 641c4e7 picked up by re-sync)
  - Evidence: `bash scripts/prepare-cli-generated-assets.sh && diff -r config/.pi cli/assets/generated/config/pi` clean; `cargo build` succeeded (3m21s) after adding `#[allow(dead_code)]` emission for the unused-until-T04 `PI_EMBEDDED_ASSETS`; `nix flake check` passed (exit 0)
  - Goal: Make the generated Pi tree available as embedded CLI assets.
  - Boundaries (in/out of scope): In — extend `scripts/prepare-cli-generated-assets.sh` to copy `config/.pi/` into `cli/assets/generated/config/pi/`, add a `PI_EMBEDDED_ASSETS` target entry in `cli/build.rs` (alongside the opencode/claude entries at `cli/build.rs:11-24`), and commit the synced `cli/assets/generated/config/pi/` tree. Out — setup command wiring, doctor.
  - Done when: `./scripts/prepare-cli-generated-assets.sh` produces `cli/assets/generated/config/pi/` matching `config/.pi/`, and `cargo build -p` the CLI crate succeeds with the new embedded asset set compiled in.
  - Verification notes (commands or checks): `./scripts/prepare-cli-generated-assets.sh && diff -r config/.pi cli/assets/generated/config/pi`; `cargo build` in `cli/`; `cargo test` for any asset-embedding tests.

- [x] T03: `Add "pi" integration target ID to config types and schema` (status:done)
  - Task ID: T03
  - Completed: 2026-07-13
  - Files changed: cli/src/services/config/types.rs (Pi variant + parse + unit tests), cli/src/services/doctor/inspect.rs (compile-only no-op Pi match arm; full doctor behavior in T05), config/pkl/base/sce-config-schema.pkl (target enum + "pi"), config/schema/sce-config.schema.json (regenerated), cli/assets/generated/config/schema/sce-config.schema.json (re-synced mirror)
  - Evidence: `nix flake check` passed (tests, clippy, fmt, pkl-parity); Pkl regen + asset-prep re-run diff-clean beyond intended schema enum change; mirror `diff` clean; new unit tests cover `"pi"` parse and unknown-ID rejection message listing `opencode, claude, pi`
  - Notes: Approved scope additions during review — Pkl-owned JSON schema enum update (runtime validates against the embedded schema) and the minimal doctor Pi arm required for exhaustive-match compilation
  - Goal: Teach the config layer about the `pi` target so `.sce/config.json` can persist and validate it.
  - Boundaries (in/out of scope): In — add `Pi` variant to `IntegrationTargetId` (`cli/src/services/config/types.rs:269-289`) with `parse()`/string round-trip as `"pi"`, and update `integrations.target` schema validation (`cli/src/services/config/schema.rs:586-624`) including its error message listing valid IDs. Out — setup flags, doctor detection.
  - Done when: `"pi"` parses and serializes; unknown IDs still rejected with the updated valid-values message; unit tests cover the new variant.
  - Verification notes (commands or checks): `cargo test` scoped to config services (e.g. `cargo test -p <cli-crate> config`); check schema error text lists `opencode`, `claude`, `pi`.

- [x] T04: `Add sce setup --pi and --all flags with Pi asset installation` (status:done)
  - Task ID: T04
  - Completed: 2026-07-13
  - Files changed: cli/src/cli_schema.rs (flags: --pi/--all added, --both removed), cli/src/services/parse/command_runtime.rs, cli/src/services/setup/mod.rs (SetupTarget::{Pi,All}, Both removed, interactive prompt gains Pi + All, generalized EmbeddedAssetSelectionIter, new unit tests), cli/src/services/setup/command.rs (untouched; request flow unchanged), cli/src/services/default_paths.rs (repo_dir::PI + pi_target_dir), cli/src/command_surface.rs (setup usage line), cli/src/services/doctor/inspect.rs (remediation text --both → --all), cli/build.rs (PI_EMBEDDED_ASSETS no longer dead_code)
  - Evidence: `nix flake check` passed (exit 0; tests, clippy, fmt, pkl parity); scratch-repo smoke via nix-built binary: `setup --pi --non-interactive` installed 15 files to `.pi/` and persisted `"pi"`; `setup --all --non-interactive` installed .opencode (19) + .claude (17) + .pi (15) and persisted all three IDs; `--pi --all` rejected as conflicting; `--both` rejected as unknown option
  - Notes: Scope change approved mid-task by user — `--both` removed entirely (flag, enum variant, interactive option); `--all` is the only multi-target selector; interactive menu now OpenCode/Claude/Pi/All
  - Goal: `sce setup --pi` installs embedded Pi assets to the repo's `.pi/` directory and persists `"pi"` into `integrations.target`; `sce setup --all` expands to opencode + claude + pi.
  - Boundaries (in/out of scope): In — `SetupTarget::Pi` and `SetupTarget::All` variants (`cli/src/services/setup/mod.rs:24-29`), `--pi`/`--all` clap flags with mutual-exclusion validation (`cli/src/services/setup/command.rs`), `integration_target_id_str()` mapping (`setup/mod.rs:412-420`), `install_embedded_setup_assets()` deploy path for the pi asset root, and `persist_integration_targets()` handling. `--both` was removed and replaced by `--all` (opencode+claude+pi). Out — hooks for Pi (no Pi hook assets), doctor.
  - Done when: In a scratch repo, `sce setup --pi --non-interactive` creates `.pi/prompts/` and `.pi/skills/` matching embedded assets and `.sce/config.json` contains `"pi"`; `sce setup --all --non-interactive` installs all three trees; flag-conflict validation rejects combining `--pi` with `--both`/`--all` etc.; setup unit/integration tests updated.
  - Verification notes (commands or checks): `cargo test` for setup services; manual smoke: `cargo run -- setup --pi --non-interactive --repo <tmpdir>` then inspect `.pi/` and `.sce/config.json`.

- [x] T05: `Add Pi integration health checks to sce doctor` (status:done)
  - Task ID: T05
  - Completed: 2026-07-13
  - Files changed: cli/src/services/doctor/inspect.rs (Pi detection/group inspection/problems), cli/src/services/doctor/types.rs (Pi labels/problem kinds), cli/src/services/doctor/mod.rs and cli/src/services/lifecycle.rs (problem-kind plumbing), cli/src/services/default_paths.rs (Pi repo path/asset constants)
  - Evidence: targeted `cargo test doctor::inspect::tests` was blocked by the repo bash policy preferring `nix flake check`; `nix flake check` passed (cli-tests, clippy, fmt, pkl parity, JS checks, workflow lint, Flatpak parity/static checks); `nix run .#pkl-check-generated` passed with generated outputs up to date.
  - Notes: Pi doctor coverage uses the existing integration group-health model with `Pi prompts` and `Pi skills` groups; `.pi/` directory fallback detection works when no integration target is configured; no Pi hooks or new problem categories were added. A temporary `cli/tests/doctor_pi.rs` integration test file was removed after user feedback.
  - Goal: `sce doctor` detects and inspects the Pi integration.
  - Boundaries (in/out of scope): In — include `Pi` in `resolve_doctor_integration_targets()` with `.pi/` directory fallback detection (`cli/src/services/doctor/inspect.rs:419-451`), add `collect_pi_integration_groups()` covering prompts and skills groups, iterate it in `inspect_repository_integrations()` (`inspect.rs:453-502`), and extend `NoIntegrationsInstalled` remediation to mention `sce setup --pi`. Out — new problem categories beyond the existing group-health model.
  - Done when: Doctor reports healthy Pi groups after `sce setup --pi`, reports missing/drifted files when `.pi/` content is deleted or altered, and falls back to `.pi/` directory detection when `.sce/config.json` lacks targets; doctor tests updated.
  - Verification notes (commands or checks): `cargo test` for doctor services; manual smoke: run `sce doctor` in the scratch repo from T04 before and after deleting a `.pi/skills/*/SKILL.md`.

- [x] T06: `Document the Pi target in architecture and setup docs` (status:done)
  - Task ID: T06
  - Completed: 2026-07-13
  - Files changed: README.md, config/pkl/README.md, context/architecture.md, context/plans/pi-harness-integration.md
  - Evidence: targeted proofread of touched sections; exact stale-claim scan passed for pre-T06 README/Pkl README wording (`generated configs that make OpenCode and Claude Code`, `OpenCode and Claude Code are first-class`, `OpenCode and/or Claude config`, `This regenerates both manual and automated OpenCode profiles plus Claude outputs`); no code/generated-output changes.
  - Notes: Important-change context sync applies because README and architecture/Pkl generation docs now present Pi as a first-class target; repaired the stale T04 plan note that still described `--both` as unchanged.
  - Goal: Update repository documentation to describe the third target tree and new flags.
  - Boundaries (in/out of scope): In — `context/architecture.md` (target-tree overview at lines 3-9), `config/pkl/README.md` ownership/profile/command sections to include the Pi profile and `config/.pi/**` outputs, and README/setup docs mentioning `--pi`/`--all`. Out — code changes, Pi user tutorials.
  - Done when: Docs accurately describe Pi generation ownership, setup flags, and doctor coverage; no stale references implying only two targets remain in touched docs.
  - Verification notes (commands or checks): `grep -rn "opencode and claude\|two.*target" context/ config/pkl/README.md` returns no stale claims; proofread rendered sections.

- [x] T07: `Validation and cleanup` (status:done)
  - Task ID: T07
  - Completed: 2026-07-13
  - Files changed: context/plans/pi-harness-integration.md
  - Evidence: direct `cargo test` was blocked by the repo bash policy (`use-nix-flake-check-over-cargo-test`); `nix flake check` passed, including `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, JS checks, workflow lint, native portability audit, and Flatpak parity/static checks; `nix develop -c ./config/pkl/check-generated.sh` passed; `nix run .#pkl-check-generated` passed; clean regeneration plus `bash scripts/prepare-cli-generated-assets.sh` left no additional git diff beyond pre-existing staged T06 docs/plan updates; scratch smoke with isolated XDG roots ran `sce setup --pi --non-interactive`, persisted `"pi"` in `.sce/config.json`, installed Pi prompts/skills, and `sce doctor` reported Pi prompts/skills PASS (full doctor PASS after installing required hooks in the scratch repo).
  - Notes: No code or generated-output changes were needed. The plan is now fully complete; context sync should verify current-state docs remain aligned.
  - Goal: Run the full verification suite, confirm end-to-end Pi flow, and sync context.
  - Boundaries (in/out of scope): In — full checks and context sync; fixing regressions surfaced by checks. Out — new features.
  - Done when: `cargo test` (workspace) passes; `nix develop -c ./config/pkl/check-generated.sh` and `nix run .#pkl-check-generated` pass; a clean regeneration + asset-prep re-run yields no git diff; end-to-end smoke (`setup --pi` → `doctor`) succeeds in a scratch repo; `context/` plan checkboxes and any touched context docs reflect final state.
  - Verification notes (commands or checks): `cargo test`; `nix develop -c pkl eval -m . config/pkl/generate.pkl && ./scripts/prepare-cli-generated-assets.sh && git status --short`; `nix run .#pkl-check-generated`; scratch-repo smoke test; review `context/plans/pi-harness-integration.md` statuses.

## Open questions

- None blocking. If Pi's actual installed version diverges from the write-up (e.g. prompt frontmatter keys), adjust the renderer in T01 against `pi --help` / real Pi docs before finalizing frontmatter.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test'` -> blocked by repo bash policy `use-nix-flake-check-over-cargo-test`; replaced with canonical repo validation below.
- `nix develop -c ./config/pkl/check-generated.sh` -> exit 0; generated outputs up to date.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs up to date.
- `nix flake check` -> exit 0; all checks passed, including CLI tests, clippy, fmt, Pkl parity, JS checks, workflow lint, native portability audit, and Flatpak parity/static checks.
- `nix develop -c sh -c 'pkl eval -m . config/pkl/generate.pkl && bash scripts/prepare-cli-generated-assets.sh' && git status --short` -> exit 0; no additional diff beyond pre-existing staged T06 docs/plan updates.
- Scratch smoke with isolated `XDG_STATE_HOME`/`XDG_CONFIG_HOME`: `sce setup --pi --non-interactive` -> exit 0; installed 15 Pi files and persisted `"pi"` in `.sce/config.json`.
- Scratch smoke follow-up: `sce doctor` after installing required hooks in the same scratch repo -> exit 0; Pi prompts and Pi skills reported `[PASS]`; summary reported 0 blocking problems and 0 warnings.
- Removed temporary scratch repo/state/config directories under `/tmp/opencode/`.

### Success-criteria verification

- [x] `config/.pi/prompts/*.md` and `config/.pi/skills/*/SKILL.md` are generated deterministically: verified by `check-generated.sh`, `pkl-check-generated`, and regeneration + asset-prep diff check.
- [x] Pkl stale-output detection covers Pi outputs: `nix develop -c ./config/pkl/check-generated.sh` passed.
- [x] `sce setup --pi` installs Pi assets and persists `"pi"`: verified in scratch repo with isolated SCE state/config roots.
- [x] `sce setup --all` and `--both` behavior were covered earlier in T04 evidence; final `nix flake check` kept that test surface passing.
- [x] `sce doctor` reports Pi prompts/skills health: verified in scratch repo; Pi groups reported `[PASS]`.
- [x] `.sce/config.json` schema accepts `"pi"` and rejects unknown target IDs: covered by prior T03 tests and final `nix flake check`.
- [x] Workspace checks pass: `nix flake check` passed; direct `cargo test` was policy-blocked in favor of that canonical repo check.

### Failed checks and follow-ups

- Direct `cargo test` was intentionally blocked by repo bash policy; no implementation failure. Canonical replacement `nix flake check` passed.
- Initial scratch `sce setup --pi` attempt used the developer's default auth DB with a mismatched temporary encryption key and failed to open the encrypted auth DB. Re-run with isolated XDG state/config roots succeeded; no code change required.
- A first regeneration command attempted to execute `./scripts/prepare-cli-generated-assets.sh` directly and hit `Permission denied`; re-run through `bash scripts/prepare-cli-generated-assets.sh` succeeded.

### Residual risks

- None identified for the implemented Pi harness integration. Pi frontmatter assumptions remain as stated in Open questions and should be revisited only if real Pi behavior diverges from the source write-up.
