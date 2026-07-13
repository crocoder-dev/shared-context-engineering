# Plan: pi-harness-integration

## Change summary

Add the Pi coding-agent harness as a first-class SCE integration target alongside OpenCode and Claude. Pi consumes project-local configuration from `.pi/`: slash commands are Markdown prompt templates in `.pi/prompts/`, and skills are Agent Skills-format `SKILL.md` packages in `.pi/skills/`. Pi has no native sub-agent format, so SCE agents (shared-context-plan, shared-context-code) are rendered as agent-role prompt templates (`.pi/prompts/agent-*.md`) that instruct Pi to act in that role within the current session.

The change follows the existing target pattern end to end: a new Pkl renderer produces a canonical `config/.pi/` tree, the asset-prep script syncs it into `cli/assets/generated/config/pi`, `build.rs` embeds it, `sce setup --pi` installs it and persists `"pi"` into `integrations.target`, and `sce doctor` detects and health-checks the Pi installation.

Decisions resolved with the user (2026-07-13):
- Generate a dedicated `.pi/skills/` tree from Pkl (no `.pi/settings.json` redirection to `.claude/skills`).
- Render SCE agents as agent-role prompt templates in `.pi/prompts/`.
- CLI: add `--pi` and a new `--all` flag; `--both` keeps its current opencode+claude meaning for backwards compatibility.
- Do not generate or append `AGENTS.md`; skills and prompts carry the workflow.

## Success criteria

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` deterministically emits `config/.pi/prompts/*.md` (commands + agent-role prompts) and `config/.pi/skills/*/SKILL.md`, and a clean re-run produces no diff.
- `nix develop -c ./config/pkl/check-generated.sh` covers the new Pi outputs and passes.
- `sce setup --pi` installs the Pi assets into the target repo's `.pi/` directory and persists `"pi"` in `.sce/config.json` `integrations.target`.
- `sce setup --all` installs opencode, claude, and pi targets; `--both` behavior is unchanged.
- `sce doctor` reports Pi integration health (prompts, skills) when `"pi"` is configured or a `.pi/` directory is detected, and its remediation text mentions `sce setup --pi`.
- `.sce/config.json` schema validation accepts `"pi"` and still rejects unknown target IDs.
- Full workspace checks pass (`cargo test`, pkl parity via `nix run .#pkl-check-generated` or `nix flake check`).

## Constraints and non-goals

- Manual profile only: no Pi equivalent of the OpenCode automated profile in this change.
- No hooks for Pi (Pi extensions are out of scope per the source write-up; the Claude hook mechanism has no Pi analogue here).
- No `.pi/SYSTEM.md`, `.pi/APPEND_SYSTEM.md`, `.pi/settings.json`, or root `AGENTS.md` generation.
- No Pi packaging (`pi install` npm/Git package) support.
- Do not change existing OpenCode/Claude generated outputs or `--both` semantics.
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

- [ ] T02: `Sync and embed Pi assets into the CLI binary` (status:todo)
  - Task ID: T02
  - Goal: Make the generated Pi tree available as embedded CLI assets.
  - Boundaries (in/out of scope): In — extend `scripts/prepare-cli-generated-assets.sh` to copy `config/.pi/` into `cli/assets/generated/config/pi/`, add a `PI_EMBEDDED_ASSETS` target entry in `cli/build.rs` (alongside the opencode/claude entries at `cli/build.rs:11-24`), and commit the synced `cli/assets/generated/config/pi/` tree. Out — setup command wiring, doctor.
  - Done when: `./scripts/prepare-cli-generated-assets.sh` produces `cli/assets/generated/config/pi/` matching `config/.pi/`, and `cargo build -p` the CLI crate succeeds with the new embedded asset set compiled in.
  - Verification notes (commands or checks): `./scripts/prepare-cli-generated-assets.sh && diff -r config/.pi cli/assets/generated/config/pi`; `cargo build` in `cli/`; `cargo test` for any asset-embedding tests.

- [ ] T03: `Add "pi" integration target ID to config types and schema` (status:todo)
  - Task ID: T03
  - Goal: Teach the config layer about the `pi` target so `.sce/config.json` can persist and validate it.
  - Boundaries (in/out of scope): In — add `Pi` variant to `IntegrationTargetId` (`cli/src/services/config/types.rs:269-289`) with `parse()`/string round-trip as `"pi"`, and update `integrations.target` schema validation (`cli/src/services/config/schema.rs:586-624`) including its error message listing valid IDs. Out — setup flags, doctor detection.
  - Done when: `"pi"` parses and serializes; unknown IDs still rejected with the updated valid-values message; unit tests cover the new variant.
  - Verification notes (commands or checks): `cargo test` scoped to config services (e.g. `cargo test -p <cli-crate> config`); check schema error text lists `opencode`, `claude`, `pi`.

- [ ] T04: `Add sce setup --pi and --all flags with Pi asset installation` (status:todo)
  - Task ID: T04
  - Goal: `sce setup --pi` installs embedded Pi assets to the repo's `.pi/` directory and persists `"pi"` into `integrations.target`; `sce setup --all` expands to opencode + claude + pi.
  - Boundaries (in/out of scope): In — `SetupTarget::Pi` and `SetupTarget::All` variants (`cli/src/services/setup/mod.rs:24-29`), `--pi`/`--all` clap flags with mutual-exclusion validation (`cli/src/services/setup/command.rs`), `integration_target_id_str()` mapping (`setup/mod.rs:412-420`), `install_embedded_setup_assets()` deploy path for the pi asset root, and `persist_integration_targets()` handling. `--both` semantics unchanged (opencode+claude). Out — hooks for Pi (no Pi hook assets), doctor.
  - Done when: In a scratch repo, `sce setup --pi --non-interactive` creates `.pi/prompts/` and `.pi/skills/` matching embedded assets and `.sce/config.json` contains `"pi"`; `sce setup --all --non-interactive` installs all three trees; flag-conflict validation rejects combining `--pi` with `--both`/`--all` etc.; setup unit/integration tests updated.
  - Verification notes (commands or checks): `cargo test` for setup services; manual smoke: `cargo run -- setup --pi --non-interactive --repo <tmpdir>` then inspect `.pi/` and `.sce/config.json`.

- [ ] T05: `Add Pi integration health checks to sce doctor` (status:todo)
  - Task ID: T05
  - Goal: `sce doctor` detects and inspects the Pi integration.
  - Boundaries (in/out of scope): In — include `Pi` in `resolve_doctor_integration_targets()` with `.pi/` directory fallback detection (`cli/src/services/doctor/inspect.rs:419-451`), add `collect_pi_integration_groups()` covering prompts and skills groups, iterate it in `inspect_repository_integrations()` (`inspect.rs:453-502`), and extend `NoIntegrationsInstalled` remediation to mention `sce setup --pi`. Out — new problem categories beyond the existing group-health model.
  - Done when: Doctor reports healthy Pi groups after `sce setup --pi`, reports missing/drifted files when `.pi/` content is deleted or altered, and falls back to `.pi/` directory detection when `.sce/config.json` lacks targets; doctor tests updated.
  - Verification notes (commands or checks): `cargo test` for doctor services; manual smoke: run `sce doctor` in the scratch repo from T04 before and after deleting a `.pi/skills/*/SKILL.md`.

- [ ] T06: `Document the Pi target in architecture and setup docs` (status:todo)
  - Task ID: T06
  - Goal: Update repository documentation to describe the third target tree and new flags.
  - Boundaries (in/out of scope): In — `context/architecture.md` (target-tree overview at lines 3-9), `config/pkl/README.md` ownership/profile/command sections to include the Pi profile and `config/.pi/**` outputs, and README/setup docs mentioning `--pi`/`--all`. Out — code changes, Pi user tutorials.
  - Done when: Docs accurately describe Pi generation ownership, setup flags, and doctor coverage; no stale references implying only two targets remain in touched docs.
  - Verification notes (commands or checks): `grep -rn "opencode and claude\|two.*target" context/ config/pkl/README.md` returns no stale claims; proofread rendered sections.

- [ ] T07: `Validation and cleanup` (status:todo)
  - Task ID: T07
  - Goal: Run the full verification suite, confirm end-to-end Pi flow, and sync context.
  - Boundaries (in/out of scope): In — full checks and context sync; fixing regressions surfaced by checks. Out — new features.
  - Done when: `cargo test` (workspace) passes; `nix develop -c ./config/pkl/check-generated.sh` and `nix run .#pkl-check-generated` pass; a clean regeneration + asset-prep re-run yields no git diff; end-to-end smoke (`setup --pi` → `doctor`) succeeds in a scratch repo; `context/` plan checkboxes and any touched context docs reflect final state.
  - Verification notes (commands or checks): `cargo test`; `nix develop -c pkl eval -m . config/pkl/generate.pkl && ./scripts/prepare-cli-generated-assets.sh && git status --short`; `nix run .#pkl-check-generated`; scratch-repo smoke test; review `context/plans/pi-harness-integration.md` statuses.

## Open questions

- None blocking. If Pi's actual installed version diverges from the write-up (e.g. prompt frontmatter keys), adjust the renderer in T01 against `pi --help` / real Pi docs before finalizing frontmatter.
