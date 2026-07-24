# Portable execution-profile model

## Current scope

The canonical manual and automated SCE aggregations in `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl` model logical units as:

- `ExecutionProfile`: invocation-wide role policy, allowed skill set, and capability ceiling;
- `WorkflowUnit`: user-invoked action, execution-profile binding, entry skill, ordered required skills, and a narrowing capability policy;
- `SkillUnit`: reusable profile-free procedure.

The manual inventory contains two execution profiles (`shared-context-plan`, `shared-context-code`), five workflows (`next-task`, `change-to-plan`, `handover`, `commit`, `validate`), and eight skills. The automated inventory uses the same vocabulary with two profiles, six workflows, and nine skills; its additional interactive planning workflow and skill remain active alongside the deterministic automated planning path.

Manual and automated target renderers consume `executionProfiles` and `workflows` while exposing target-native carrier collections. Automated topology remains OpenCode-only. OpenCode profile agents render broad invocation policy with `mode: primary`; OpenCode workflow commands bind the canonical profile title, set `subtask: false`, and derive `entry-skill` plus ordered `skills` directly from each workflow. Claude exposes composed normal workflow commands and profile-free skills, with no native profile-agent renderer surface. Pi exposes only composed workflow prompts and profile-free skills; it also has no profile-agent renderer surface.

The plan profile owns planning/context and no-implementation boundaries without duplicating `/change-to-plan` ordering; its manual capability ceiling excludes `process.execute`. The code profile owns controlled repository operations, evidence, and context alignment without imposing one-task execution on every invocation; it allows process execution without making `vcs.commit` approval profile-wide. One-task behavior remains workflow/skill-owned by `next-task` and `sce-task-execution`, while commit approval remains owned by the `commit` workflow.

Manual `next-task` uses one cross-harness workflow-control contract. Not-ready and authorization-required reviews stop; auto-authorized or explicitly authorized reviews transition immediately to the exact task-execution confirmation gate without writes. After confirmed execution, plan update, and context synchronization, the workflow re-reads the plan and emits exactly one `next_task`, `plan_complete`, `blocked`, or `current_task_incomplete` result. Next-task selection follows plan order and satisfied dependencies. Only `next_task` emits a command, as the final response content. The reusable task-execution skill owns the no-write confirmation stop and current-task completion result but never selects the next task.

## Policy composition

`shared-content-common.pkl` provides typed, section-aware construction helpers:

- `nativeAgentBody(profile)` copies the canonical `ProfilePolicy.body` and deterministically appends its allowed-skill relationships;
- `nativeWorkflowBody(workflow)` renders a workflow body natively (OpenCode) with metadata-derived Related Units;
- `composeProfile(profile, workflow)` amends the workflow body and overrides only its Preconditions, Guardrails, and Failure handling with the profile fragment; it emits no identity markers or other HTML comments;
- `renderBody(...)` remains the only heading serializer, so composition never searches or replaces Markdown headings.

Composition is selective: the execution profile contributes only the three policy-bearing sections (Preconditions, Guardrails, Failure handling), and every other section — Purpose, Inputs, Workflow, Outputs, Completion criteria, and optional `Reference`/`Examples` — stays exactly as authored in the workflow. Related Units are always metadata-derived: the bound execution profile, the entry skill, then the remaining required skills, each rendered once in that deterministic order, followed by any authored relationship that cannot be derived from `executionProfile`, `entrySkill`, or `requiredSkills`. Authored `body.relatedUnits` therefore holds only non-derived extras (for example, `/next-task` on `change-to-plan`). The OpenCode renderers adopt `nativeAgentBody` for profile carriers and `nativeWorkflowBody` for workflow commands, keeping bodies thin because commands bind the native profile directly. Claude commands render `composeProfile(...)` so normal slash-command use receives the profile policy without a fork. Pi prompts also render `composeProfile(...)` and prepend a target-specific precondition requiring the full project-local `.pi/skills/{entrySkill}/SKILL.md` read before action.

## Projection inventory

`config/pkl/base/instruction-unit-inventory.pkl` models each canonical unit with logical kind `execution-profile`, `workflow`, or `skill` and a list of explicit `Projection` records. Every projection carries target, carrier, profile binding, tool-control strength, semantic-control strength, generated destination, and nullable root mirror. Policy intent remains canonical; target metadata translates capabilities to native tool names, while projection control fields only classify enforcement strength. A native carrier or tool allowlist does not imply semantic enforcement, which remains `prompt` for every current projection.

Approved manual projections are:

| Logical kind | OpenCode | Claude | Pi |
| --- | --- | --- | --- |
| execution profile | native agent | none | none |
| workflow | native-bound command | composed command | composed prompt |
| skill | skill | skill | skill |

Automated profiles, workflows, and skills each have one OpenCode projection and no root mirror. Semantic control is `prompt` for every projection. Tool control is `native` for current OpenCode profile/workflow carriers and Claude workflow commands, and `none` for Pi prompts and skill carriers.

Projection-derived collections are path-sorted and currently contain 58 generated instruction destinations plus 41 manual root mirrors, for 99 committed projected instruction files. Pi has no generated or mirrored `agent-*` prompt compatibility files; only its five approved workflow prompts and eight skill projections are emitted.

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

`config/pkl/renderers/opencode-metadata.pkl` is the OpenCode-only translation boundary from canonical capabilities to native tool names. Both manual and automated profile permissions derive from profile policy; workflow command permissions derive from each workflow's effective policy. A native tool is `ask` when any effective capability mapped to it requires approval and is `allow` when at least one mapped capability is allowed without approval. Excluded Bash capability is always a hard `block`; other excluded tools inherit the profile-specific deny posture (`ask` for manual, `block` for automated).

The resulting manual OpenCode Bash contract is explicit: Shared Context Code, `next-task`, and `validate` render `allow`; Shared Context Plan, `change-to-plan`, and process-excluding `handover` render `block`; `commit` renders `ask` because its workflow retains `vcs.commit` approval. Every manual workflow command emits a complete `permission` block, canonical `entry-skill` and ordered `skills`, an `ask` wildcard skill posture, and `allow` entries for exactly its required skills. Skill permission entries otherwise derive from profile `allowedSkills` or workflow `requiredSkills`; OpenCode metadata files own translation/presentation rather than command-agent maps, skill chains, or canonical permission intent.

## Claude translation and composition

`config/pkl/renderers/claude-metadata.pkl` translates canonical capabilities to Claude native tools. `repository.read/search/write`, `process.execute`, `interaction.ask`, and `skill.invoke` map to the ordered Claude tool set (`Read`, `Glob`, `Grep`, `Edit`, `Write`, `Bash`, `AskUserQuestion`, `Skill`, and `Task`); `vcs.commit` also maps to `Bash`. Command `allowed-tools` derive exactly from effective workflow policies with duplicate native tools removed.

Claude has no native Shared Context profile files. All five normal commands compose their canonical profile policy into the command body without identity markers or other HTML comments and remain in the main conversation without `context: fork`. Focused checks validate canonical profile preconditions, guardrails, and failure handling, including missing/wrong profile-policy fixtures, exact allowed-tool derivation, and structural validity.

## Relationship contract

For every manual and automated workflow, by authored construction:

- `executionProfile` resolves to an existing profile;
- `entrySkill` resolves and appears in `requiredSkills`;
- each required skill resolves and belongs to the selected profile's allowlist;
- each workflow capability belongs to the profile capability ceiling.

These relationships, the effective-approval math, and target bindings are no longer machine-validated: the former `portable-execution-profile-check.pkl` gate and the canonical `*Problems` listings it consumed were removed (see Validation). `effectiveWorkflowPolicies` is still computed in each aggregation because the OpenCode/Claude renderers consume it, but the relationship and narrowing invariants now hold only by authored construction and are guarded indirectly through deterministic regeneration and byte parity.

## Contributor templates and migration

Contributor-facing authoring starts from `templates/execution-profile.md`, `templates/workflow.md`, and `templates/skill.md`. The execution-profile template requires canonical profile identity plus `ProfilePolicy.body`, `allowedSkills`, and harness-neutral `toolPolicy`. The workflow template requires canonical identity/body, `executionProfile`, `entrySkill`, `requiredSkills`, and a narrowing `toolPolicy`; target-native frontmatter shown in either template is a projection example, not canonical permission ownership.

Pi no longer projects execution profiles as fake prompts and has no compatibility wrappers. Existing Pi invocations migrate as follows:

- `agent-shared-context-plan` → `change-to-plan`
- `agent-shared-context-code` → `next-task`

The replacement prompts compose the appropriate profile policy and must load their project-local entry skill before acting.

## Validation

The former focused Pkl gates — `portable-execution-profile-check.pkl`, `metadata-coverage-check.pkl`, and the structural `instruction-unit-validator.pkl`/`instruction-unit-validator-check.pkl` — were removed. The only remaining Pkl gate is that `config/pkl/generate.pkl` evaluates successfully and every committed generated instruction file matches a fresh regeneration byte-for-byte:

```bash
nix run .#pkl-check-generated
```

`config/pkl/check-generated.sh` and the root flake's `pkl-parity` check regenerate into a temporary tree and compare all generation-owned config outputs, root mirrors, and templates. This catches any hand-edited or stale generated file but no longer enforces structural, composition, relationship, capability, or file-count contracts; those hold by canonical construction only.
