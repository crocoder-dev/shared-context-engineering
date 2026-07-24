# Portable execution-profile model

## Current scope

The canonical manual and automated SCE aggregations in `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl` model logical units as:

- `ExecutionProfile`: invocation-wide role policy, allowed skill set, and capability ceiling;
- `WorkflowUnit`: user-invoked action, execution-profile binding, entry skill, ordered required skills, and a narrowing capability policy;
- `SkillUnit`: reusable profile-free procedure.

The manual inventory contains two execution profiles (`shared-context-plan`, `shared-context-code`), five workflows (`next-task`, `change-to-plan`, `handover`, `commit`, `validate`), and eight skills. The automated inventory uses the same vocabulary with two profiles, six workflows, and nine skills; its additional interactive planning workflow and skill remain active alongside the deterministic automated planning path.

Manual and automated target renderers consume `executionProfiles` and `workflows` while exposing target-native carrier collections. Automated topology remains OpenCode-only. OpenCode profile agents render broad invocation policy with `mode: primary`; OpenCode workflow commands bind the canonical profile title, set `subtask: false`, and derive `entry-skill` plus ordered `skills` directly from each workflow. Claude keeps both native profile agents for explicit `claude --agent` activation and composes the bound profile into each normal workflow command. Pi exposes only composed workflow prompts and profile-free skills; it has no profile-agent renderer surface.

The plan profile owns planning/context and no-implementation boundaries without duplicating `/change-to-plan` ordering. The code profile owns controlled repository operations, evidence, and context alignment without imposing one-task execution on every invocation. One-task behavior remains workflow/skill-owned by `next-task` and `sce-task-execution`.

## Policy composition

`shared-content-common.pkl` provides typed, section-aware construction helpers:

- `nativeAgentBody(profile)` copies the canonical `ProfilePolicy.body` and deterministically appends its allowed-skill relationships;
- `composeProfile(profile, workflow)` combines profile and workflow fields before Markdown rendering, emits `<!-- sce-execution-profile: {slug} -->`, and generates profile/required-skill relationships;
- `renderBody(...)` remains the only heading serializer, so composition never searches or replaces Markdown headings.

Composition carries profile policy through purpose, inputs, preconditions, workflow posture, guardrails, outputs, completion criteria, and failure handling while retaining workflow-owned optional `Reference`/`Examples`. Target renderers adopt `nativeAgentBody` for profile carriers. OpenCode keeps workflow bodies thin because its commands bind the native profile directly. Claude commands render `composeProfile(...)` so normal slash-command use receives the same policy without a fork. Pi prompts also render `composeProfile(...)` and prepend a target-specific precondition requiring the full project-local `.pi/skills/{entrySkill}/SKILL.md` read before action.

## Projection inventory

`config/pkl/base/instruction-unit-inventory.pkl` models each canonical unit with logical kind `execution-profile`, `workflow`, or `skill` and a list of explicit `Projection` records. Every projection carries target, carrier, profile binding, tool-control strength, semantic-control strength, generated destination, and nullable root mirror. Policy intent remains canonical; target metadata translates capabilities to native tool names, while projection control fields only classify enforcement strength. A native carrier or tool allowlist does not imply semantic enforcement, which remains `prompt` for every current projection.

Approved manual projections are:

| Logical kind | OpenCode | Claude | Pi |
| --- | --- | --- | --- |
| execution profile | native agent | native agent | none |
| workflow | native-bound command | composed command | composed prompt |
| skill | skill | skill | skill |

Automated profiles, workflows, and skills each have one OpenCode projection and no root mirror. Semantic control is `prompt` for every projection. Tool control is `native` for current OpenCode/Claude profile/workflow carriers and `none` for Pi prompts and skill carriers.

Projection-derived collections are path-sorted and currently contain 60 generated instruction destinations plus 43 manual root mirrors, for 103 committed projected instruction files. Duplicate target/carrier pairs within a unit are rejected. Pi has no generated or mirrored `agent-*` prompt compatibility files; only its five approved workflow prompts and eight skill projections are emitted.

## Capability policy

`config/pkl/base/shared-content-common.pkl` owns the harness-neutral capability vocabulary:

- `repository.read`
- `repository.search`
- `repository.write`
- `process.execute`
- `interaction.ask`
- `skill.invoke`
- `vcs.commit`

`ToolPolicy` carries ordered `allowedCapabilities` and `approvalRequiredCapabilities`. `ProfilePolicy` combines an `InstructionBody`, a profile skill allowlist, and a profile `ToolPolicy`.

A workflow may only narrow its profile capability ceiling. Its effective allow-set is exactly the workflow allow-set. Effective approval requirements are:

```text
(profile approvals ∪ workflow approvals) ∩ workflow allowed capabilities
```

`effectiveToolPolicy` implements this rule in canonical capability order.

## OpenCode translation and enforcement

`config/pkl/renderers/opencode-metadata.pkl` is the OpenCode-only translation boundary from canonical capabilities to native tool names. Both manual and automated profile permissions derive from profile policy; workflow command permissions derive from each workflow's effective policy. A native tool is `ask` when any effective capability mapped to it requires approval, is allowed when at least one mapped effective capability permits it, and otherwise inherits the profile-specific deny posture (`ask` for manual, `block` for automated).

Skill permission entries derive from profile `allowedSkills` or workflow `requiredSkills`, with the wildcard retaining the deny posture. OpenCode metadata files now own presentation text only; they do not own command-agent maps, skill chains, or authored permission blocks.

## Claude translation and composition

`config/pkl/renderers/claude-metadata.pkl` translates canonical capabilities to Claude native tools. `repository.read/search/write`, `process.execute`, `interaction.ask`, and `skill.invoke` map to the ordered Claude tool set (`Read`, `Glob`, `Grep`, `Edit`, `Write`, `Bash`, `AskUserQuestion`, `Skill`, and `Task`); `vcs.commit` also maps to `Bash`. Native profile `tools` derive from profile ceilings, while command `allowed-tools` derive exactly from effective workflow policies with duplicate native tools removed.

The two Claude native profile files remain available for explicit whole-session activation. All five normal commands instead compose their canonical profile policy into the command body, include the stable profile marker, and remain in the main conversation without `context: fork`. Focused checks cover valid composition, missing/wrong markers, missing policy fragments, exact allowed-tool derivation, and structural validity.

## Relationship contract

For every manual and automated workflow:

- `executionProfile` resolves to an existing profile;
- `entrySkill` resolves and appears in `requiredSkills`;
- each required skill resolves and belongs to the selected profile's allowlist;
- each workflow capability belongs to the profile capability ceiling.

Each canonical aggregation exposes deterministic problem listings and effective workflow policies. `config/pkl/renderers/portable-execution-profile-check.pkl` constrains those problem listings to be empty, verifies profile bindings plus effective approval behavior, preserves the automated planning profile's no-process-execution ceiling, proves automated units remain OpenCode-only, rejects duplicate projections, checks exact destinations/root mirrors and the 60/43/103 count contract, checks broad profile boundaries and stable composition fragments, verifies each Pi prompt's profile marker/policy and full entry-skill read plus projected skill paths, and runs the structural validator against native/composed target output. Three valid and ten invalid portable fixtures cover malformed logical references, capability narrowing, Pi profile projection, duplicate/partially classified/misdirected projections, unresolved Pi skill paths, and stale Pi agent prompts.

## Contributor templates and migration

Contributor-facing authoring starts from `templates/execution-profile.md`, `templates/workflow.md`, and `templates/skill.md`. The execution-profile template requires canonical profile identity plus `ProfilePolicy.body`, `allowedSkills`, and harness-neutral `toolPolicy`. The workflow template requires canonical identity/body, `executionProfile`, `entrySkill`, `requiredSkills`, and a narrowing `toolPolicy`; target-native frontmatter shown in either template is a projection example, not canonical permission ownership.

Pi no longer projects execution profiles as fake prompts and has no compatibility wrappers. Existing Pi invocations migrate as follows:

- `agent-shared-context-plan` → `change-to-plan`
- `agent-shared-context-code` → `next-task`

The replacement prompts compose the appropriate profile policy and must load their project-local entry skill before acting.

## Validation

Run the focused model gate with:

```bash
nix develop -c pkl eval \
  config/pkl/renderers/portable-execution-profile-check.pkl \
  -x summary
```

A passing result reports the manual 2/5/8 and automated 2/6/9 profile/workflow/skill counts, seven capabilities, five manual plus six automated effective policies, the OpenCode-only automated target, three valid plus ten invalid portable fixtures, and `PORTABLE_EXECUTION_PROFILE_MODEL_OK`. This gate, metadata coverage, and structural validation run before parity in both `pkl-check-generated` and the root flake's `pkl-parity` check.
