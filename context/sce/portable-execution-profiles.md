# Portable execution-profile model

## Current scope

The canonical manual and automated SCE aggregations in `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl` model logical units as:

- `ExecutionProfile`: invocation-wide role policy, allowed skill set, and capability ceiling;
- `WorkflowUnit`: user-invoked action, execution-profile binding, entry skill, ordered required skills, and a narrowing capability policy;
- `SkillUnit`: reusable profile-free procedure.

The manual inventory contains two execution profiles (`shared-context-plan`, `shared-context-code`), five workflows (`next-task`, `change-to-plan`, `handover`, `commit`, `validate`), and eight skills. The automated inventory uses the same vocabulary with two profiles, six workflows, and nine skills; its additional interactive planning workflow and skill remain active alongside the deterministic automated planning path.

Manual and automated target renderers consume `executionProfiles` and `workflows` but continue to expose target carrier collections named `agents` and `commands`. Automated topology remains OpenCode-only. Generated paths and target-native frontmatter are unchanged at this stage. Native profile-agent bodies now render broad invocation policy rather than workflow sequencing; explicit target projections and final native workflow binding remain later boundaries.

The plan profile owns planning/context and no-implementation boundaries without duplicating `/change-to-plan` ordering. The code profile owns controlled repository operations, evidence, and context alignment without imposing one-task execution on every invocation. One-task behavior remains workflow/skill-owned by `next-task` and `sce-task-execution`.

## Policy composition

`shared-content-common.pkl` provides typed, section-aware construction helpers:

- `nativeAgentBody(profile)` copies the canonical `ProfilePolicy.body` and deterministically appends its allowed-skill relationships;
- `composeProfile(profile, workflow)` combines profile and workflow fields before Markdown rendering, emits `<!-- sce-execution-profile: {slug} -->`, and generates profile/required-skill relationships;
- `renderBody(...)` remains the only heading serializer, so composition never searches or replaces Markdown headings.

Composition carries profile policy through purpose, inputs, preconditions, workflow posture, guardrails, outputs, completion criteria, and failure handling while retaining workflow-owned optional `Reference`/`Examples`. Target renderers currently adopt `nativeAgentBody`; Claude/Pi workflow composition and final OpenCode native binding remain deferred to their projection tasks.

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

## Relationship contract

For every manual and automated workflow:

- `executionProfile` resolves to an existing profile;
- `entrySkill` resolves and appears in `requiredSkills`;
- each required skill resolves and belongs to the selected profile's allowlist;
- each workflow capability belongs to the profile capability ceiling.

Each canonical aggregation exposes deterministic problem listings and effective workflow policies. `config/pkl/renderers/portable-execution-profile-check.pkl` constrains those problem listings to be empty, verifies profile bindings plus effective approval behavior, preserves the automated planning profile's no-process-execution ceiling, proves automated units remain OpenCode-only, checks broad profile boundaries and stable composition fragments, and runs the structural validator against native/composed helper output.

## Validation

Run the focused model gate with:

```bash
nix develop -c pkl eval \
  config/pkl/renderers/portable-execution-profile-check.pkl \
  -x summary
```

A passing result reports the manual 2/5/8 and automated 2/6/9 profile/workflow/skill counts, seven capabilities, five manual plus six automated effective policies, the OpenCode-only automated target, and `PORTABLE_EXECUTION_PROFILE_MODEL_OK`.
