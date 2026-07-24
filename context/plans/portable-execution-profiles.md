# Plan: Portable execution profiles

## Change summary

Amend PR #154 on branch `sce-md-standard` with a follow-up to `context/plans/instruction-unit-standardization-all-harnesses.md`. The completed plan remains the historical record of the nine-section instruction-unit standard; this plan corrects only its assumption that agents and commands project as equivalent logical units across OpenCode, Claude Code, and Pi.

Replace the canonical `agent` / `command` model with portable execution profiles, workflows, and skills. An execution profile owns invocation-wide role policy, allowed skills, and a typed harness-neutral capability ceiling. A workflow owns the user-invoked action, profile binding, entry skill, required skill chain, and a capability policy that narrows its profile. Skills remain reusable procedures and do not carry profiles.

Project those units honestly: OpenCode uses primary agents plus native workflow binding; Claude keeps native agents for explicit `claude --agent` use but composes profile policy into normal commands; Pi has no standalone profile projection and composes profile policy plus an explicit entry-skill read into workflow prompts. Remove Pi's fake `agent-shared-context-*` prompts immediately.

Canonical policy uses capability IDs (`repository.read`, `repository.search`, `repository.write`, `process.execute`, `interaction.ask`, `skill.invoke`, and `vcs.commit`). Target metadata translates capabilities to native tool names only. Projection metadata separately reports carrier, profile binding, tool-control strength, semantic-control strength, destination, and optional root mirror.

## Success criteria

- [x] SC1: Canonical logical units are modeled and named as execution profiles, workflows, and skills in both manual and automated variants.
- [x] SC2: Instruction bodies are structured as typed sections, and one `renderBody` boundary preserves the required nine-section order plus optional `Reference` and `Examples`.
- [x] SC3: Every workflow references one existing execution profile and entry skill; the entry skill is in `requiredSkills`; every profile-allowed skill resolves in the same profile inventory.
- [x] SC4: Execution profiles and workflows carry typed harness-neutral `ToolPolicy`; workflow capabilities are subsets of their profile ceiling, and effective approval requirements follow the approved union/intersection rule.
- [x] SC5: Target metadata contains capability-to-native-tool translation rather than canonical policy intent, and projections report enforcement strength independently.
- [x] SC6: Manual OpenCode profiles render as `mode: primary` agents; workflows derive `agent`, `entry-skill`, and ordered `skills` from canonical workflow data and set `subtask: false`.
- [x] SC7: Automated OpenCode uses the same execution-profile/workflow vocabulary and native-binding model without adding Claude or Pi automated outputs.
- [x] SC8: Claude profile agents remain available for explicit whole-session activation, while every normal workflow command contains the expected composed profile marker/policy and stays in the main conversation without `context: fork`.
- [x] SC9: Pi emits no `agent-*.md` profile prompt; every workflow prompt contains the expected composed profile policy and explicitly loads its generated project-local entry skill before acting.
- [x] SC10: Profile native-agent bodies and composed workflows are generated from one canonical `ProfilePolicy`; no target manually duplicates profile policy.
- [x] SC11: The projection inventory deterministically exposes target, carrier, profile binding, tool control, semantic control, destination, and root mirror; generation and parity paths derive from it.
- [x] SC12: Validation rejects all unresolved logical references, invalid capability narrowing, duplicate/invalid projections, target binding errors, policy composition errors, capability translation violations, Pi skill-loading errors, stale Pi agent prompts, and existing structural-body violations.
- [x] SC13: The focused fixtures cover every valid/invalid binding case listed in this plan, and metadata coverage reports profile/workflow/skill plus projection/enforcement/capability coverage.
- [x] SC14: Contributor templates are renamed to `execution-profile.md` and `workflow.md`; architecture, glossary, overview, validator documentation, and migration guidance describe portable policy rather than cross-harness agent parity.
- [x] SC15: Generated inventory totals are projection-derived and regress at 60 rendered instruction files plus 43 manual root mirrors (103 committed instruction files total).
- [x] SC16: Regeneration, focused Pkl checks, generated parity, `nix flake check`, and `git diff --check` pass with all generated and embedded/install-consumed outputs synchronized.

## Constraints and non-goals

- Depends on the completed `instruction-unit-standardization-all-harnesses.md` plan and supersedes only that plan's agent/command projection model.
- Target PR #154 and branch `sce-md-standard` while that PR remains unmerged; plan history is not reopened if PR scope continues.
- Pkl sources remain authoritative. Generated files under `config/.opencode`, `config/.claude`, `config/.pi`, and root mirrors must be regenerated, not manually authored.
- Preserve the current nine-section order, uniqueness, frontmatter, known-heading, skill identity, and generated-parity contracts; composition occurs before structural validation.
- Preserve actual planning, task execution, context sync, handover, commit, bootstrap, and validation procedures except where existing agent bodies incorrectly duplicate workflow behavior.
- Keep skills reusable and profile-free. Do not turn profile policy into an optional skill.
- Canonical capability policy contains intent. Target metadata contains only capability translation/presentation. `Projection.toolControl` and `semanticControl` classify enforcement strength, not permission intent.
- A workflow may only narrow its profile: `workflow.allowedCapabilities` must be a subset of `profile.allowedCapabilities`.
- Effective allowed capabilities equal the workflow allow-set. Effective approval-required capabilities equal `(profile approvals ∪ workflow approvals) ∩ effective allowed`.
- Semantic control remains `prompt` for every initial projection; native activation/tool allowlists do not prove semantic boundaries.
- Remove Pi `agent-shared-context-plan` and `agent-shared-context-code` outputs immediately, with no deprecated wrappers.
- Do not emulate a primary-agent runtime in Pi, add target-specific runtime extensions, make Claude workflows forked subagents, migrate Claude commands to skills, or claim identical permission enforcement.
- Keep automated content OpenCode-only while using the same logical schema and vocabulary as manual content.
- The repository has no checked-in changelog/release-notes surface. Record the two Pi replacement mappings in durable migration documentation and provide exact release-note copy in the final PR handoff rather than inventing a new release framework.
- Keep each task aligned to one coherent atomic commit and leave unrelated worktree state untouched.

## Task stack

- [x] T01: `Introduce structured instruction bodies without generated-byte drift` (status:done)
  - Task ID: T01
  - Goal: Replace string-only body ownership with typed `InstructionBody` sections and one Markdown serialization boundary while preserving current output bytes.
  - Boundaries (in/out of scope): In — `shared-content-common.pkl`, automated common schema, grouped manual/automated content modules, shared renderer helpers, all required and optional sections. Out — logical kind renames, profile composition, capability policy, target projection changes, semantic body narrowing.
  - Done when: Every active body is authored as typed sections; `renderBody` is the only nine-section serializer; optional sections retain canonical order; regeneration produces no instruction-file byte changes.
  - Verification notes (commands or checks): Evaluate manual/automated aggregation and renderers through `nix develop -c pkl eval`; regenerate to a temporary directory and compare all owned paths; run `nix run .#pkl-check-generated` and `git diff --check`.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/shared-content-common.pkl`, `shared-content-automated-common.pkl`, all six grouped manual/automated content modules, both aggregation modules, `instruction-unit-templates.pkl`, the shared OpenCode/Claude/Pi renderer content helpers, `context/{overview,architecture,glossary,context-map}.md`, and `context/sce/instruction-unit-validator.md`.
  - Evidence: Manual and automated aggregation plus OpenCode, automated OpenCode, Claude, Pi, template, metadata-coverage, and validator Pkl evaluations exited 0; validator summary remained `VALIDATION_OK` with 62 production units, 107 committed generated files, 5 valid fixtures, and 10 invalid fixtures; `nix run .#pkl-check-generated` exited 0 with `Generated outputs are up to date`, proving staged regeneration introduced no generated instruction-file drift; `nix flake check --print-build-logs` exited 0 with all checks passed, including Pkl parity and 131 Rust tests; `git diff --check` exited 0.
  - Notes: `InstructionBody` now types all nine required sections plus nullable ordered `Reference` and `Examples`; the automated schema aliases the canonical type; `renderBody` is the sole production Markdown section serializer and is reused by contributor templates and every target renderer. Logical kinds, target metadata, frontmatter, profile behavior, capability policy, and generated bytes are unchanged. Context impact was root-edit required because canonical authoring and rendering ownership changed; overview, architecture, glossary, context map, and validator context now describe the typed boundary, and the post-sync parity/validator/diff checks pass.

- [x] T02: `Model manual profiles, workflows, skills, and canonical capability policy` (status:done)
  - Task ID: T02
  - Goal: Convert the manual canonical aggregation to `ExecutionProfile`, `WorkflowUnit`, and `SkillUnit`, including workflow/profile/skill relationships and typed capability policy.
  - Boundaries (in/out of scope): In — `ToolPolicy`, canonical capability vocabulary, `ProfilePolicy`, allowed skills, workflow `executionProfile`/`entrySkill`/`requiredSkills`, effective-policy helper, manual profile/workflow declarations, and renderer adaptations needed to consume the new model. Out — final target-specific binding behavior, inventory projections, automated conversion.
  - Done when: Both manual profiles and all five workflows use the typed schema; relationships and capability ceilings resolve; current target renderers evaluate from the new names without parallel `agents`/`commands` ownership.
  - Verification notes (commands or checks): Focused Pkl evaluation proving profile/skill references, entry-skill membership, capability subset, and effective approval calculations; metadata coverage; generated parity.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/shared-content-common.pkl`, `shared-content.pkl`, `instruction-unit-inventory.pkl`; manual OpenCode/Claude/Pi renderer adapters; `metadata-coverage-check.pkl`; new focused `portable-execution-profile-check.pkl`; and synchronized `context/{overview,architecture,glossary,context-map}.md` plus `context/sce/portable-execution-profiles.md`.
  - Evidence: The focused model check evaluated with `PORTABLE_EXECUTION_PROFILE_MODEL_OK` and counts 2 profiles / 5 workflows / 8 skills / 7 capabilities / 5 effective policies; manual aggregation, all three manual target renderers, and metadata coverage evaluated successfully; structural validation remained `VALIDATION_OK` for 62 rendered units and 107 committed files with all 15 fixtures passing; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check --print-build-logs` passed; `git diff --check` passed.
  - Notes: Manual canonical ownership is now `executionProfiles` / `workflows` / `skills`; target carrier names remain unchanged until projection work. `effectiveToolPolicy` preserves the workflow allow-set and computes approvals as `(profile approvals ∪ workflow approvals) ∩ workflow allowed`. Automated conversion, target-native capability translation/binding, profile composition, and projection inventory remain deferred to T03–T09. Context impact was root-edit required because canonical logical-unit and policy ownership changed; root context and the new focused portable-profile contract now describe the transitional current state.

- [x] T03: `Apply the logical and capability model to automated OpenCode units` (status:done)
  - Task ID: T03
  - Goal: Convert automated canonical content to the same execution-profile/workflow/skill and capability-policy model without changing its OpenCode-only topology or deterministic posture.
  - Boundaries (in/out of scope): In — automated aggregation/common/grouped modules, six workflows, nine skills, automated profile ceilings, entry/required skills, renderer consumption. Out — automated Claude/Pi outputs and manual behavior changes.
  - Done when: Automated units no longer use canonical agent/command terminology; every workflow narrows a valid profile policy and preserves automation-specific gates and interactive-planning units.
  - Verification notes (commands or checks): Focused automated Pkl evaluation, capability-subset checks, metadata coverage, regeneration/parity, and inspection that no automated Claude/Pi projection appears.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/shared-content-automated-{common,plan,code,commit}.pkl`, `shared-content-automated.pkl`, `instruction-unit-inventory.pkl`; `config/pkl/renderers/opencode-automated-content.pkl`, `metadata-coverage-check.pkl`, and `portable-execution-profile-check.pkl`; synchronized `context/{overview,architecture,glossary,context-map}.md` and `context/sce/portable-execution-profiles.md`.
  - Evidence: The focused portable-profile gate reported manual 2/5/8 and automated 2/6/9 profile/workflow/skill counts, six automated effective policies, seven capabilities, `targets = List("opencode")`, and `PORTABLE_EXECUTION_PROFILE_MODEL_OK`; automated aggregation, metadata coverage, and automated OpenCode renderer evaluations exited 0; structural validation remained `VALIDATION_OK` for 62 rendered units and 107 committed files with all 15 fixtures passing; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check --print-build-logs` passed; `git diff --check` passed.
  - Notes: Automated canonical ownership is now `executionProfiles` / `workflows` / `skills`, with shared type aliases rather than a parallel `ContentUnit`. Relationship and narrowing checks preserve all six workflow bindings, including `change-to-plan-interactive` → `sce-plan-authoring-interactive`; the automated planning profile excludes `process.execute` to match its bash-blocked posture. OpenCode renderer carrier names and generated bytes remain unchanged pending T04–T06. Context impact was root-edit required because canonical terminology and aggregation ownership now apply to both profiles.

- [x] T04: `Narrow execution-profile policy and add structured composition helpers` (status:done)
  - Task ID: T04
  - Goal: Make plan/code profiles broad invocation policies rather than duplicated workflows and generate native-agent/composed-workflow bodies from those policies.
  - Boundaries (in/out of scope): In — manual and automated profile policies, generic native-agent section construction, section-aware `composeProfile`, deterministic HTML marker, related-unit generation, composed preconditions/guardrails/failure handling, and canonical skill allowlists. Out — target frontmatter and projection inventory adoption.
  - Done when: Plan policy owns planning/context boundaries without `/change-to-plan` ordering; code policy owns controlled repository/operational behavior without a universal one-task claim; one-task behavior remains in `next-task`/`sce-task-execution`; composition never uses heading string replacement.
  - Verification notes (commands or checks): Evaluate native and composed body helpers; inspect exact marker/profile fragments and section order; run structural validator against helper outputs and generated parity for any adopted outputs.
  - Completed: 2026-07-24
  - Files changed: canonical manual/automated plan and code profile sources; shared base/automated/renderer composition helpers; all four target content adapters; focused portable-profile checks; regenerated manual and automated native profile carriers/root mirrors; synchronized root context and `context/sce/portable-execution-profiles.md`.
  - Evidence: The focused portable-profile gate reported `PORTABLE_EXECUTION_PROFILE_MODEL_OK` and additionally validated broad-profile ownership, exact `<!-- sce-execution-profile: shared-context-code -->` composition markers, profile precondition/guardrail/failure fragments, generated skill relationships, and structurally valid native/manual-composed/automated-composed helper output. Structural validation reported `VALIDATION_OK` for 62 rendered units, 107 committed files, and all 15 fixtures; metadata coverage evaluated successfully; regeneration completed; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check --print-build-logs` passed including 131 Rust tests; `git diff --check` passed.
  - Notes: Profile policy is now broad and workflow-neutral; one-task behavior remains in `next-task` and `sce-task-execution`. `nativeAgentBody` is adopted by current profile carriers, while `composeProfile` is implemented and tested but target workflow adoption remains T06–T08. Context impact was root-edit required because canonical role boundaries and composition architecture changed.

- [x] T05: `Replace fixed destinations with explicit target projections` (status:done)
  - Task ID: T05
  - Goal: Make carrier, profile binding, enforcement classification, destination, and root mirror explicit for every manual and automated unit.
  - Boundaries (in/out of scope): In — `Projection`, profile/workflow/skill logical kinds, projection matrix, duplicate-target/carrier prevention, projection-derived counts and path collections, removal of Pi profile projections. Out — final target frontmatter semantics and full validator fixture expansion.
  - Done when: `UnitDestinations` and fixed target assumptions are gone; manual profiles project only to OpenCode/Claude, workflows and skills project as approved, automated units remain OpenCode-only, and inventory computes 60 rendered files plus 43 root mirrors.
  - Verification notes (commands or checks): Evaluate inventory and metadata coverage; assert no Pi profile projection and no duplicate target/carrier pair; inspect deterministic projection/path ordering and derived 60/43/103 counts.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/instruction-unit-inventory.pkl`; projection consumers in `metadata-coverage-check.pkl`, `portable-execution-profile-check.pkl`, and `instruction-unit-validator.pkl`; synchronized root context plus `context/sce/{portable-execution-profiles,instruction-unit-validator}.md`.
  - Evidence: Inventory evaluation reported manual 2/5/8 logical units with 43 projections, automated 2/6/9 with 17 projections, and projection-derived counts of 60 generated instruction files, 43 root mirrors, and 103 committed projected files. Focused checks proved manual profiles project exactly to OpenCode/Claude, workflows/skills retain approved target coverage, automated units remain OpenCode-only, semantic control remains prompt-classified, paths are deterministic, and duplicate projection findings are empty. Metadata coverage evaluated successfully; structural validation reported `VALIDATION_OK` for 60 rendered projections and 103 committed projected files with all 15 fixtures passing; `nix run .#pkl-check-generated`, `nix flake check --print-build-logs` including 131 Rust tests, and `git diff --check` passed.
  - Notes: `UnitDestinations`, `unit.targets`, and fixed agent/command/skill logical kinds are removed. The two generated Pi profile prompts and mirrors are intentionally no longer approved projections but remain generation/parity-owned until T08 deletes their renderer/generator paths. Context impact was root-edit required because target topology, inventory architecture, validator scope, and canonical terminology changed.

- [x] T06: `Render OpenCode profiles and native-bound workflows` (status:done)
  - Task ID: T06
  - Goal: Implement OpenCode primary-agent projection, native profile binding, and capability-derived permissions for manual and automated content.
  - Boundaries (in/out of scope): In — manual/automated OpenCode content and metadata adapters; capability bindings; profile `mode: primary`; canonical workflow-derived `agent`, `entry-skill`, ordered `skills`; `subtask: false`; thin workflow bodies; removal of command ownership/skill-chain maps from metadata. Out — Claude/Pi rendering.
  - Done when: Every generated profile agent is primary; every workflow resolves its canonical profile agent and skill chain; interactive workflows stay in the primary conversation; permission blocks are derived from effective canonical capabilities plus target translation rather than authored policy strings.
  - Verification notes (commands or checks): Focused OpenCode native-binding fixtures; inspect manual/automated frontmatter; metadata coverage; regenerate and validate OpenCode outputs; parity and `git diff --check`.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/shared-content-common.pkl`; manual/automated OpenCode content and metadata renderers; `metadata-coverage-check.pkl` and `portable-execution-profile-check.pkl`; generated manual/root-mirror and automated OpenCode profile/workflow files; synchronized `context/{overview,architecture,glossary,context-map}.md` and `context/sce/portable-execution-profiles.md`.
  - Evidence: The focused portable-profile gate evaluated all checks with empty capability-translation and OpenCode native-binding problem listings and `PORTABLE_EXECUTION_PROFILE_MODEL_OK`; metadata coverage and structural validation passed with 60 rendered projections, 103 committed projected files, and all 15 fixtures; frontmatter audit found 4 primary profile agents and 11 non-subtask workflows; regeneration completed and `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check --print-build-logs` passed including Pkl parity and 131 Rust tests; `git diff --check` passed.
  - Notes: OpenCode profile agents now derive primary-mode permissions from canonical profile policy. Workflow commands derive profile title, entry/ordered skills, `subtask: false`, and effective capability permissions from canonical workflow data; target metadata no longer owns command-agent maps, skill chains, or authored permission blocks. Manual disallowed tools retain `ask`, automated disallowed tools use `block`, and shared native tools conservatively become `ask` when any mapped effective capability requires approval. The focused gate also exposed and fixed a lazy `composeProfile` parameter/property shadowing recursion without changing generated bodies. Context impact was root-edit required because OpenCode binding and target-translation architecture changed.

- [x] T07: `Render Claude native profiles and composed workflows` (status:done)
  - Task ID: T07
  - Goal: Keep explicit `claude --agent` profile files while making normal commands self-contained through profile composition and capability-derived allowed tools.
  - Boundaries (in/out of scope): In — Claude content/metadata adapters, native profile bodies, composed workflow bodies/markers, effective capability translation, allowed-tool ceiling checks, removal of logical binding from target metadata. Out — `context: fork`, command-to-skill carrier migration, Pi changes.
  - Done when: Both native profile files remain; all five normal commands compose the expected profile policy in the main conversation; allowed tools equal translated effective capabilities and never exceed canonical policy; no `context: fork` appears.
  - Verification notes (commands or checks): Valid composed-Claude fixture plus missing/wrong marker and missing-policy-fragment checks; inspect allowed-tools derivation; regenerate, run structural validation/parity, and `git diff --check`.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/renderers/{claude-content,claude-metadata,metadata-coverage-check,portable-execution-profile-check}.pkl`; generated Claude agents/commands under `config/.claude/` and root `.claude/`; synchronized `context/{overview,architecture,glossary,context-map}.md` and `context/sce/portable-execution-profiles.md`.
  - Evidence: Claude content and metadata coverage evaluated successfully; the focused portable-profile gate reported `PORTABLE_EXECUTION_PROFILE_MODEL_OK` and proved both native profiles, all five composed workflows, valid/missing/wrong-marker plus missing-policy-fragment cases, exact capability-derived tool metadata, and no forked context; structural validation reported `VALIDATION_OK` for 60 rendered projections and 103 committed projected files with all 15 fixtures; regeneration completed; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check --print-build-logs` passed including Pkl parity and 131 Rust tests; `git diff --check` and the no-`context: fork` audit passed.
  - Notes: Claude profile `tools` now derive from canonical profile ceilings, and command `allowed-tools` derive from effective workflow policies through a Claude-only capability translation. Normal commands render `composeProfile(...)` with stable markers in the main conversation; native profile files remain for explicit whole-session activation. Target metadata no longer owns per-command tool strings or logical binding. Pi and validator-wide binding fixture expansion remain T08/T09. Context impact was root-edit required because Claude target binding and capability-translation architecture changed.

- [x] T08: `Render Pi composed workflows and remove fake agent prompts` (status:done)
  - Task ID: T08
  - Goal: Project profiles into Pi workflow prompts, force canonical entry-skill loading, and delete all managed `agent-shared-context-*` prompts.
  - Boundaries (in/out of scope): In — Pi content/metadata cleanup, command argument hints, composed profile markers/policy, explicit `.pi/skills/<entrySkill>/SKILL.md` read instructions, projection-driven generation, stale managed-output removal from config/root trees. Out — compatibility wrappers and Pi extension enforcement.
  - Done when: `pi-content.pkl` exposes no `agentPrompts`; agent descriptions/hints/skill references are removed; five workflow prompts compose policy and require full entry-skill loading; the four config/root fake agent files are absent; generated skill paths resolve.
  - Verification notes (commands or checks): Valid Pi composed fixture; checks for marker, policy fragments, explicit skill read, and generated skill existence; `find config/.pi/prompts .pi/prompts -name 'agent-*.md'` returns none; regeneration/parity detects no stale managed files.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/renderers/{pi-content,pi-metadata,metadata-coverage-check,portable-execution-profile-check}.pkl`, `config/pkl/generate.pkl`; five generated Pi workflow prompts under both `config/.pi/prompts/` and `.pi/prompts/`; removal of four generated/mirrored `agent-shared-context-*` prompts; synchronized `context/{overview,architecture,glossary,context-map}.md` and `context/sce/portable-execution-profiles.md`.
  - Evidence: Pi content, metadata coverage, and focused portable-profile Pkl evaluations exited 0 with `PORTABLE_EXECUTION_PROFILE_MODEL_OK`; the focused gate validates all five profile markers/policy fragments, exact full entry-skill read instructions, projected config/root skill paths, and structural validity. Structural validation remained `VALIDATION_OK` for 60 rendered projections and 103 committed projected files with all 15 fixtures passing. Regeneration completed; `find config/.pi/prompts .pi/prompts -name 'agent-*.md'` returned no files; marker/read audit found all five prompts in both trees; `nix run .#pkl-check-generated`, `nix flake check --print-build-logs` including 131 Rust tests, and both staged/unstaged `git diff --check` passed.
  - Notes: Pi now has no profile-agent renderer or generator surface and no compatibility wrappers. Its workflow prompts render canonical `composeProfile(...)` bodies and add a Pi-specific precondition to read `.pi/skills/{entrySkill}/SKILL.md` completely before acting. Argument hints remain workflow-only metadata. Context impact was root-edit required because the approved cross-target projection architecture and generated ownership changed.

- [x] T09: `Enforce the portable binding and capability contract in Pkl` (status:done)
  - Task ID: T09
  - Goal: Extend structural validation, fixtures, metadata coverage, and generated-path checks to prove the complete logical/projection/target contract.
  - Boundaries (in/out of scope): In — validator/check fixtures, metadata coverage, projection/destination parity, capability bindings, policy fragments, stale Pi prompt detection, count regression, `check-generated.sh` and flake filesets/check wiring. Out — new runtime enforcement.
  - Done when: Fixtures prove valid OpenCode native, Claude composed, and Pi composed bindings; invalid fixtures cover missing profile, missing entry skill, entry skill absent from required skills, capability ceiling violation, unexpected Pi profile projection, missing OpenCode primary mode, mismatched agent, missing `subtask: false`, missing/wrong composed marker, missing guardrail, excessive target tools, missing Pi skill read, unresolved skill path, duplicate projection, omitted enforcement classification, destination mismatch, and stale Pi agent prompt; existing structural fixtures still pass.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary`; metadata coverage evaluation; deliberate temporary stale-output parity failure; confirm projection-derived 60 rendered and 103 committed counts.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/instruction-unit-inventory.pkl`, `config/pkl/check-generated.sh`, `config/pkl/renderers/{instruction-unit-validator,instruction-unit-validator-check,metadata-coverage-check,portable-execution-profile-check}.pkl`, `flake.nix`, `context/{overview,architecture,glossary,context-map}.md`, and `context/sce/{instruction-unit-validator,portable-execution-profiles}.md`.
  - Evidence: Structural validation reported `VALIDATION_OK` for 60 rendered projections, 103 committed files, 8 valid fixtures, and 18 invalid fixtures; the portable gate reported 3 valid and 10 invalid focused contract fixtures with `PORTABLE_EXECUTION_PROFILE_MODEL_OK`; metadata coverage reported 15 manual logical units, 17 automated logical units, 60 projections, 103 committed files, and seven capability translations for each native-tool target with `METADATA_COVERAGE_OK`. A temporary `.pi/prompts/agent-t09-stale-fixture.md` made `nix run .#pkl-check-generated` fail at `.pi/prompts` as expected and was removed by a trap; the clean parity run then reported generated outputs up to date. `nix flake check --print-build-logs` passed with the three Pkl gates wired into `pkl-parity`; staged and unstaged `git diff --check` plus the no-Pi-agent-prompt path audit passed.
  - Notes: The structural validator now enforces OpenCode primary/native bindings, effective permission blocks, Claude/Pi composition markers and guardrails, Claude tool ceilings, and Pi entry-skill loading/path resolution. The focused portable fixtures cover logical-reference, capability-narrowing, projection-classification/destination, unresolved-path, and stale-output failures that cannot be represented as valid typed production objects. No runtime enforcement was added. Context impact is root-edit required because the repository-wide Pkl validation and parity architecture changed.

- [x] T10: `Rename templates and document the execution-profile migration` (status:done)
  - Task ID: T10
  - Goal: Align contributor-facing templates and durable context with portable execution profiles and provide migration/release-note guidance.
  - Boundaries (in/out of scope): In — canonical template source; generated `templates/execution-profile.md`, `templates/workflow.md`, and retained skill template; removal of old agent/command templates; `context/architecture.md`, `context/glossary.md`, `context/overview.md`, `context/context-map.md`, and validator documentation; Pi replacement mapping; exact release-note copy for PR #154 handoff. Out — a new changelog/release system or unrelated documentation cleanup.
  - Done when: Documentation no longer claims Pi agent parity; it states the policy/translation/enforcement rule; templates require canonical profile/workflow fields; migration text maps `agent-shared-context-plan → change-to-plan` and `agent-shared-context-code → next-task`; final handoff includes concise release-note copy.
  - Verification notes (commands or checks): Regenerate templates; grep for stale cross-harness agent-parity and old template-path claims; validate links and paths; run generated parity and context reference checks.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/base/instruction-unit-templates.pkl`, `config/pkl/generate.pkl`; generated `templates/{execution-profile,workflow,skill}.md` with `templates/{agent,command}.md` removed; `context/{architecture,glossary,overview,context-map}.md`; `context/sce/{instruction-unit-validator,portable-execution-profiles}.md`.
  - Evidence: Regeneration emitted only `templates/execution-profile.md`, `templates/workflow.md`, and retained `templates/skill.md`; metadata coverage reported `METADATA_COVERAGE_OK` for 60 projections and 103 committed instruction files; the portable model gate reported `PORTABLE_EXECUTION_PROFILE_MODEL_OK`; structural validation reported `VALIDATION_OK` with 60 production units, 103 committed files, eight valid fixtures, and 18 invalid fixtures; `nix run .#pkl-check-generated` reported generated outputs up to date; targeted path/reference audits proved old current-state template claims and Pi agent prompts absent while both migration mappings resolve to generated workflow prompts; `nix flake check --print-build-logs` passed after the two new generated template paths were marked intent-to-add so Git-backed flake evaluation included them; `git diff --check` passed.
  - Notes: The profile and workflow templates distinguish canonical harness-neutral capability intent from target-native frontmatter projection examples and require the canonical relationship/policy fields. Durable context now states that target metadata translates capabilities while projection metadata classifies enforcement, and maps Pi `agent-shared-context-plan` to `change-to-plan` plus `agent-shared-context-code` to `next-task` without compatibility wrappers. Context impact was root-edit required because contributor vocabulary and repository-wide policy documentation changed.

- [x] T11: `Regenerate, validate, and clean the complete projection model` (status:done)
  - Task ID: T11
  - Goal: Run final plan validation and cleanup with complete evidence, synchronized generated outputs, and no unrelated changes.
  - Boundaries (in/out of scope): In — full regeneration, focused Pkl checks, projection/count/path audits, generated parity, full flake checks (including Rust tests), diff hygiene, context-sync verification, plan status/evidence, and release-note handoff text. Out — new behavior or compatibility files.
  - Done when: All success criteria have evidence; 60 config-generated instruction files and 43 root mirrors are derived and present; fake Pi agent prompts and old templates are absent; no temporary artifacts remain; context describes current truth.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix develop -c pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary`; `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check --print-build-logs`; targeted deterministic path/reference/capability audits; `git diff --check`; `git status --short`.
  - Completed: 2026-07-24
  - Files changed: `config/pkl/renderers/portable-execution-profile-check.pkl` and `context/plans/portable-execution-profiles.md`.
  - Evidence: Full regeneration exited 0. Metadata coverage reported 15 manual logical units, 17 automated logical units, 60 projections, 60 generated instruction files, 43 root mirrors, 103 committed instruction files, seven OpenCode and seven Claude capability translations, and `METADATA_COVERAGE_OK`. Structural validation reported 60 production units, 103 generated-file units, zero diagnostics, eight valid fixtures, 18 invalid fixtures, and `VALIDATION_OK`. The portable model gate reported manual 2/5/8 and automated 2/6/9 counts, three valid and ten invalid fixtures, seven capabilities, and `PORTABLE_EXECUTION_PROFILE_MODEL_OK`. Targeted audits found zero Pi `agent-*.md` prompts, zero old templates, five composed Pi workflows in each config/root tree, all ten profile-marker and entry-skill-read copies, and both migration mappings. `nix run .#pkl-check-generated`, `nix flake check --print-build-logs` including 131 Rust tests, and staged plus unstaged `git diff --check` all exited 0.
  - Notes: Initial direct evaluation of the portable model gate exposed stale Dynamic collection access and a helper agent fixture missing the now-required OpenCode `mode: primary`; both were corrected in scope and the focused gate, parity, and full flake suite passed afterward. Context impact was verify-only: root and focused context already describe the validated current behavior. No `context/tmp` changes or untracked temporary artifacts remain.

## Validation Report

### Commands run

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` -> exit 0 (all generated targets regenerated).
- `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` -> exit 0 (`METADATA_COVERAGE_OK`; 60/43/103 path totals).
- `nix develop -c pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary` -> exit 0 (`VALIDATION_OK`; zero production/generated diagnostics and zero fixture failures).
- `nix develop -c pkl eval config/pkl/renderers/portable-execution-profile-check.pkl` -> exit 0 after the in-scope gate fix (`PORTABLE_EXECUTION_PROFILE_MODEL_OK`).
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date`).
- `nix flake check --print-build-logs` -> exit 0 (`all checks passed`; 131 Rust tests passed).
- Targeted Pi/template/migration audit -> exit 0 (no Pi agent prompts or old templates; ten composed prompt copies; mappings present).
- `git diff --check && git diff --cached --check` -> exit 0.
- `git status --short -- context/tmp` plus untracked-file audit -> exit 0 (no task temporary artifacts).

### Failed checks and follow-ups

- The first portable-model evaluation failed on stale Dynamic collection APIs, then exposed a helper fixture missing `mode: primary`. Both failures were fixed within the focused validation gate and all affected/full checks passed on rerun. No follow-up remains.

### Success-criteria verification

- [x] SC1-SC4: Focused portable-model checks prove manual/automated logical kinds, references, typed policies, narrowing, and effective approvals.
- [x] SC5-SC8: Metadata coverage and structural validation prove translation-only target metadata, OpenCode primary/native binding, automated OpenCode-only topology, and Claude native/composed behavior.
- [x] SC9-SC10: Pi audits and composition checks prove no fake profile prompts, full entry-skill loading, and canonical profile-policy composition.
- [x] SC11-SC13: Inventory/validator outputs prove explicit projection fields, deterministic paths, all required invalid cases, and complete fixture/coverage totals.
- [x] SC14: Template/path and durable-context audits prove the renamed contributor templates and migration guidance.
- [x] SC15: Metadata coverage and structural validation prove 60 generated destinations plus 43 root mirrors, totaling 103 committed instruction files.
- [x] SC16: Regeneration, focused checks, parity, full flake checks, and diff hygiene all pass.

### Residual risks

- `nix flake check` validated the current `x86_64-linux` system and reported incompatible systems as omitted; no plan-specific residual risk was identified.

### Release-note handoff text

> SCE instruction units now use portable execution profiles, workflows, and profile-free skills with canonical harness-neutral capability policy. OpenCode uses primary profile agents and native-bound workflows, Claude composes profile policy into normal commands while retaining explicit agents, and Pi uses composed workflow prompts only. Pi users should replace `agent-shared-context-plan` with `change-to-plan` and `agent-shared-context-code` with `next-task`.

## Open questions

None. The approved decisions are: use this new follow-up plan in the same PR, remove Pi fake agent prompts without wrappers, and keep typed capability intent canonical while target metadata translates tool names and projections classify enforcement.
