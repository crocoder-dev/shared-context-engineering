# Plan: codex-explicit-workflow-invocation

## Change summary

Extends the existing, completed Codex integration (`context/plans/codex-cli-integration.md`) rather than replacing it. Today every generated Codex skill under `.agents/skills/` — including the six catalog-registered SCE workflows — is only a discoverable skill: Codex may implicitly activate one from conversational relevance alone, the same way any ordinary skill would. This plan adds Codex-specific `agents/openai.yaml` metadata to the six catalog workflow skills (`sce-change-to-plan`, `sce-next-task`, `sce-validate`, `sce-commit`, `sce-handover`, `sce-brownfield`) so each disables implicit invocation (`policy.allow_implicit_invocation: false`), making them explicit command-like entrypoints reachable only via `$sce-<slug>` or Codex's `/skills` UI — closer to how Claude Code, Pi, and OpenCode already treat these stateful lifecycle workflows. It also adds a concrete `$sce-<slug>` example to each generated `SKILL.md`'s `## Input` section so the explicit-invocation convention is visible in the instructions themselves, without introducing `$ARGUMENTS` (Codex skill loading has no such substitution). The internal `sce-decision` package is deliberately excluded: it has no user-facing entrypoint on any target and is invoked only by `/next-task`'s own task-synchronization gate, so gating it behind explicit user selection would risk breaking that internal call. No `.codex/prompts`/`.codex/commands` mechanism is introduced, and Codex's hooks, tracing, Bash policy, `apply_patch` evidence, setup, and doctor behavior are unchanged.

## Acceptance criteria

- [ ] AC1: Each of the six catalog-registered workflow skills generates `.agents/skills/{slug}/agents/openai.yaml` containing a `policy: allow_implicit_invocation: false` block and an `interface` block whose `display_name`/`short_description` are derived from the workflow catalog's `title`/`description`.
  - Validate: `nix run .#pkl-generate -- "$(mktemp -d)"` then inspect `.agents/skills/{sce-change-to-plan,sce-next-task,sce-validate,sce-commit,sce-handover,sce-brownfield}/agents/openai.yaml`; `nix run .#pkl-check-generated`.
- [ ] AC2: `sce-decision` continues to generate no `agents/openai.yaml` and keeps its current (implicit-eligible) invocation policy, since it is an internal helper invoked by `/next-task`'s own instructions rather than a user-facing entrypoint.
  - Validate: `nix run .#pkl-check-generated` assertion; direct inspection of `.agents/skills/sce-decision/` shows only `SKILL.md` and `references/adr-template.md`.
- [ ] AC3: Every generated Codex `SKILL.md`'s `## Input` section states a concrete `$sce-{slug}` invocation example and contains no literal `$ARGUMENTS`.
  - Validate: direct inspection of generated Codex skill bodies; `nix run .#pkl-check-generated`.
- [ ] AC4: `sce setup --codex --non-interactive` installs each workflow's `agents/openai.yaml` beside its `SKILL.md`, honors the existing `integrations.optional_workflows` selection for `sce-brownfield` (installed only when selected), never installs one for `sce-decision`, and `sce doctor`'s Codex `Skills` group reports the new files as healthy rather than missing or stale.
  - Validate: in a scratch Git repository, run `sce setup --codex --non-interactive` with and without `--workflow brownfield`, then `sce doctor`, and inspect the installed tree and doctor output.
- [ ] AC5: No `.codex/prompts/`, `.codex/commands/`, or other new custom-slash-command mechanism is introduced; `.codex/hooks.json`, the install-guidance hook script, the `sce hooks codex` dispatcher, Bash policy delegation, and `apply_patch` evidence capture are unchanged.
  - Validate: `git diff` shows no new path under `.codex/`; `nix flake check`.
- [ ] AC6: OpenCode, Claude, and Pi generated output (commands, skills, agents, settings) is unchanged.
  - Validate: `nix run .#pkl-check-generated`; diff each target's generated payload against its pre-change output.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/architecture.md`, `context/context-map.md`, `context/overview.md` — describe the Codex renderer also emitting `agents/openai.yaml` per catalog workflow, and the bumped exact generated-artifact count.
- `context/sce/codex-integration-runtime.md` — document the `$sce-*` explicit-invocation convention, `/skills` discovery, the `allow_implicit_invocation: false` mechanism and its rationale (stateful SCE lifecycle transitions must not auto-trigger), and why `sce-decision` is the one catalog-adjacent package deliberately excluded.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside
  the status. Never infer `synced` from conversation history; write every lifecycle
  transition to the plan file.

## Constraints and non-goals

- **In scope:** `config/pkl/renderers/codex-content.pkl`; a new `config/pkl/renderers/codex-metadata.pkl` (mirroring the existing `opencode-metadata.pkl`/`claude-metadata.pkl` convention); the `## Input` parameterization in `config/pkl/renderers/workflow-composite.pkl` / `config/pkl/base/workflow-content.pkl`; `config/pkl/renderers/metadata-coverage-check.pkl`; `config/pkl/renderers/generation-contract-check.pkl`; `config/pkl/generate.pkl` output wiring; `cli/build.rs` and `cli/src/services/setup/**`/`cli/src/services/doctor/**` only if verification in T02 finds a real asset-staging or reporting gap; the durable context files listed under Context sync.
- **Out of scope:** `cli/src/services/hooks/codex/**` (the `UserPromptSubmit`/`Stop`/`PreToolUse(Bash)`/`PostToolUse(apply_patch)` dispatcher and its evidence pipeline); `.codex/hooks.json`/hook-script generation; `sce-decision`'s own behavior, content, or invocation mechanism; any OpenCode/Claude/Pi command or agent generation change beyond what the shared catalog/renderer plumbing mechanically requires; a Codex slash-command (e.g. `/change-to-plan`) compatibility layer; `cli/migrations/**`; the Agent Trace schema or `apply_patch` normalization/parsing.
- **Constraints:** derive `interface.display_name`/`interface.short_description` from the existing typed `workflow-catalog.pkl` records rather than introducing a second workflow catalog; author the exact upstream-confirmed `agents/openai.yaml` schema (see Assumptions) rather than an invented shape; reuse the existing composite renderer's per-target `argumentsReference` parameterization for Codex's `## Input` divergence rather than forking `workflow-composite.pkl`'s control flow; leave `sce-decision`'s generation path untouched.
- **Non-goal:** a per-repository configurable allowlist of which workflows are explicit-only — the change request treats all six catalog workflows uniformly; Codex UI branding fields (`icon_small`, `icon_large`, `brand_color`) — optional upstream fields with no current asset source in this repository; `dependencies.tools` (MCP tool) declarations — none of these skills depend on an MCP tool.

## Assumptions

- Confirmed the current upstream `agents/openai.yaml` schema via `developers.openai.com/codex/skills` (redirects to `learn.chatgpt.com/docs/build-skills`) on 2026-08-24 rather than guessing: three top-level snake_case sections — `interface` (`display_name`, `short_description`, `icon_small`, `icon_large`, `brand_color`, `default_prompt`, all optional), `policy.allow_implicit_invocation` (boolean, default `true`; `false` excludes the skill from implicit/conversational activation while explicit `$skill-name` invocation and `/skills` discovery remain unaffected), and `dependencies.tools`. This plan authors only `interface.{display_name,short_description,default_prompt}` and `policy.allow_implicit_invocation`.
- `sce-decision` is excluded from this change: per `context/glossary.md`'s `decision skill package` entry and `context/architecture.md`, it has no user-facing command or prompt on any target and is invoked only by `/next-task`'s own task-synchronization gate — the change request's own "used internally as a helper capability" carve-out applies to it directly.
- `sce-brownfield` is treated as a full sixth explicit-only workflow (`display_name`/`short_description`/`default_prompt` plus `allow_implicit_invocation: false`), generated under the same existing optional-workflow install-time selection filter that already governs its other assets — no new filtering logic is needed since that filter already excludes or includes its whole skill subtree.
- `interface.default_prompt` has no existing canonical-catalog counterpart (unlike `display_name`/`short_description`, which reuse the catalog's `title`/`description` verbatim); each workflow's `default_prompt` is authored once, directly in the new Codex-metadata renderer module, as one short imperative sentence (for example `"Turn this change request into an SCE plan."` for `sce-change-to-plan`), since it is genuinely Codex-only UI convenience text with no cross-target equivalent to derive it from.

## Task stack

- [x] T01: `Add typed Codex skill-metadata model and agents/openai.yaml renderer` (status:complete)
  - Task ID: T01
  - Scope: In — new `config/pkl/renderers/codex-metadata.pkl`: a typed record for Codex skill interface metadata, one authored value per catalog workflow (`display_name`/`short_description` derived from `workflow-catalog.pkl`'s `title`/`description`; `default_prompt` authored per workflow per the Assumptions above), and a pure render function producing `agents/openai.yaml` text in the confirmed upstream schema. Out — wiring into `codex-content.pkl`/`generate.pkl`; any `SKILL.md` body change; any check-file update.
  - Dependencies: none
  - Done when: the module evaluates standalone and, for each of the six catalog workflow slugs, produces YAML text containing `interface: display_name / short_description / default_prompt` and `policy: allow_implicit_invocation: false` in the confirmed nesting, with no other top-level keys.
  - Verify: `nix develop -c pkl eval config/pkl/renderers/codex-metadata.pkl` (or an inline eval expression exercising each workflow's rendered output).
  - Completed: 2026-08-24
  - Files changed: `config/pkl/renderers/codex-metadata.pkl` (new)
  - Result: Added `CodexSkillMetadata` (`displayName`, `shortDescription`, `defaultPrompt`), an authored `defaultPrompts` mapping (one short imperative sentence per catalog workflow), `metadataByCommandSlug` built from `workflow-catalog.pkl`'s `title`/`description`, and a pure `render` function producing `agents/openai.yaml` text (`interface: {display_name, short_description, default_prompt}` / `policy: {allow_implicit_invocation: false}`, no other top-level keys) exposed via `renderedByCommandSlug` for all six catalog workflow slugs. Not wired into `codex-content.pkl`/`generate.pkl` (T02) or any check file (T02).
  - Verify (actual): `nix develop -c pkl eval config/pkl/renderers/codex-metadata.pkl` — passed; printed `metadataByCommandSlug`/`renderedByCommandSlug` for `change-to-plan`, `next-task`, `validate`, `commit`, `handover`, `brownfield`, each rendered block containing exactly `interface` (`display_name`/`short_description`/`default_prompt`) and `policy.allow_implicit_invocation: false`. `nix run .#pkl-check-generated` — passed unchanged (`135 files`, same inventory hash as pre-task), confirming the new unwired module has no effect on existing generated output.
  - Context impact: Localized. New standalone Pkl renderer module with no consumers yet (wiring is T02); no existing renderer, generated artifact, or root context file changed. No context synchronization required for this task.
  - Context synchronization: synced

- [ ] T02: `Emit agents/openai.yaml for the six catalog workflows and verify Codex install/doctor pickup` (status:todo)
  - Task ID: T02
  - Scope: In — extend `codex-content.pkl`'s exposed skill-document map to add `{slug}/agents/openai.yaml` for `sce-change-to-plan`, `sce-next-task`, `sce-validate`, `sce-commit`, `sce-handover`, and `sce-brownfield` (not `sce-decision`); wire the addition through `config/pkl/generate.pkl` if the existing flattened-map consumption does not already pick it up; extend `metadata-coverage-check.pkl`'s exact-inventory assertions and `generation-contract-check.pkl` (bump `expectedArtifactPathCount`; assert every one of the six documents contains `policy: allow_implicit_invocation: false` plus catalog-derived `interface` values; assert `sce-decision` has none); verify in a scratch repository that `sce setup --codex --non-interactive` installs the new files (honoring `--workflow brownfield` selection) and that `sce doctor` reports them as healthy, adjusting `cli/build.rs` asset staging or `cli/src/services/doctor/**` only if that verification surfaces a real gap. Out — `## Input` prose changes (T03).
  - Dependencies: T01
  - Done when: `nix run .#pkl-generate -- "$(mktemp -d)"` produces `.agents/skills/{slug}/agents/openai.yaml` for the six workflows and none for `sce-decision`; `nix run .#pkl-check-generated` passes with the updated contract; a scratch-repo `sce setup --codex --non-interactive` installs the new files and `sce doctor` reports no problem for them.
  - Verify: `nix run .#pkl-check-generated`; manual scratch-repo `sce setup --codex --non-interactive` plus `sce doctor` run, with and without `--workflow brownfield`.
  - Context synchronization: pending

- [ ] T03: `Add explicit $sce-<slug> invocation example to Codex's generated Input section` (status:todo)
  - Task ID: T03
  - Scope: In — extend the existing target-specific `argumentsReference` parameterization in `workflow-composite.pkl`/`workflow-content.pkl` so Codex's rendered `## Input` section states a concrete `$sce-{slug}` example beside its existing "invocation input" prose, without introducing `$ARGUMENTS`; extend `generation-contract-check.pkl`'s existing no-`$ARGUMENTS`/target-neutral-reference assertions to cover the new line; confirm OpenCode/Claude/Pi command and skill bodies are unchanged. Out — any other `SKILL.md` section; any hook/runtime code.
  - Dependencies: T01, T02
  - Done when: each generated `.agents/skills/{slug}/SKILL.md`'s `## Input` section names a `$sce-{slug}` example, contains no `$ARGUMENTS`, and OpenCode/Claude/Pi generated payloads are byte-identical to their pre-task output.
  - Verify: `nix run .#pkl-check-generated`; direct diff of OpenCode/Claude/Pi generated payload before and after this task.
  - Context synchronization: pending

## Open questions

None. The change request specifies the exact policy behavior and the one genuinely unresolved technical detail — the upstream `agents/openai.yaml` schema — was confirmed against the current OpenAI Codex documentation before writing this plan rather than guessed at (see Assumptions). The remaining choice this plan makes on the user's behalf, the per-workflow `default_prompt` wording, is a reversible content detail recorded under Assumptions rather than a scope, criteria, or ordering question.
