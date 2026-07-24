# Instruction unit validator

## Scope

The repository-owned instruction-unit validator is implemented entirely in Pkl:

- `config/pkl/renderers/instruction-unit-validator.pkl` owns validation logic, the deterministic 58-projection rendered-model input set, and direct loading of 99 committed projected instruction files.
- `config/pkl/renderers/instruction-unit-validator-check.pkl` owns valid and invalid fixture checks plus the evaluation gate.

Run the focused validation with:

```bash
nix develop -c pkl eval \
  config/pkl/renderers/instruction-unit-validator-check.pkl \
  -x summary
```

A passing result reports `productionUnitCount = 58`, `generatedFileUnitCount = 99`, zero rendered-model and generated-file diagnostics, eight valid fixtures, 24 invalid fixtures, zero fixture failures, and `status = "VALIDATION_OK"`.

## Input ownership

Canonical manual and automated bodies are authored as typed `InstructionBody` sections and serialized by the shared `renderBody` boundary before target rendering. Production validation consumes the resulting document objects from the manual OpenCode, Claude, and Pi renderers and the automated OpenCode renderer. Unit paths, kinds, profiles, targets, and slugs come from `instruction-unit-inventory.pkl`; the resulting unit list is sorted by destination path before validation.

The same explicit projection inventory drives direct validation of 58 approved config instruction destinations and 41 tracked manual root mirrors. Claude profile agents have no approved projection; stale files under either Claude agent directory are rejected by generated-output parity. Generated-file inputs are projection-path-sorted and parsed into frontmatter/body before applying the same rules, while generated-output parity separately proves byte equality for all generation-owned files. Pi profile prompts have no approved projection or generated compatibility output; any stale `agent-*` prompt in a parity-owned Pi prompt directory is detected as generated drift.

## Validation contract

The validator enforces:

- target-aware required frontmatter fields;
- body start at `## Purpose`;
- all nine required sections exactly once and in order;
- only optional `Reference` then final `Examples` after required sections;
- no unknown level-two headings or body-level `When to use`;
- fenced code blocks excluded from heading analysis;
- skill frontmatter `name` matching its destination directory;
- OpenCode command skill references resolving to the automated skill inventory, which is the active superset;
- OpenCode agent `permission.skill` entries resolving to that inventory, except wildcard `*`;
- OpenCode execution-profile agents using `mode: primary`;
- OpenCode workflows binding the canonical profile agent, remaining non-subtask, declaring canonical entry/ordered required skills, and matching complete capability-derived permission blocks including wildcard and required-skill entries;
- command and prompt bodies containing no HTML comments;
- Claude workflows carrying canonical profile preconditions, guardrails, and failure handling with capability-derived `allowed-tools`;
- Pi workflows carrying canonical profile preconditions, guardrails, and failure handling, requiring the full project-local entry-skill read, and resolving that skill to its generated path;
- every manual `next-task` projection carrying not-ready and authorization-required stops, authorized transition to the exact implementation gate, plan re-read, all four continuation outcomes, dependency-aware plan-order selection, and terminal next-task-section semantics;
- every manual `sce-task-execution` projection carrying the exact confirmation gate, no-write `current_task_incomplete` result, and orchestration-owned next-task boundary.

Diagnostics use the stable shape:

```text
<path> [<agent|command|skill>] <rule>: <message>; expected: <shape>
```

## Fixtures

The Pkl check module includes valid agent, command, skill, manual-profile, and automated-profile fixtures plus valid OpenCode-native, Claude-composed, and Pi-composed workflow bindings. Its 24 invalid fixtures retain the structural/frontmatter/binding cases and additionally reject missing not-ready or authorized transitions, missing plan re-read, non-terminal next-task rendering, and a task-execution skill without its no-write stop. The adjacent portable-model gate additionally asserts the exact manual OpenCode Code/Plan Bash postures, command-specific Bash outcomes, explicit permission blocks, wildcard skill posture, required-skill allows, commit-only approval ownership, and 11 valid readiness/continuation state-machine cases.

Logical-reference, capability-ceiling, projection-classification/destination, unresolved Pi skill-path, and stale Claude/Pi profile-output cases use 12 additional typed fixtures in `portable-execution-profile-check.pkl`, because malformed canonical objects cannot inhabit the production Pkl types.

The check module constrains production diagnostics and fixture-failure listings to be empty, so evaluation fails when the production model becomes invalid or a fixture no longer proves its expected rule.

## Integration boundary

`config/pkl/generate.pkl` emits both config instruction outputs and the tracked manual root mirrors under `.opencode/`, `.claude/`, and `.pi/`, plus the root contributor templates `templates/execution-profile.md`, `templates/workflow.md`, and `templates/skill.md`. The templates use canonical logical-unit vocabulary: profile policy owns harness-neutral capability intent, workflow policy may only narrow it, target metadata translates capabilities, and projection metadata classifies enforcement. `config/pkl/check-generated.sh` checks all generation-owned config outputs, root instruction mirrors, and templates. Both `config/pkl/check-generated.sh` and the root flake's `pkl-parity` check evaluate metadata coverage, the portable execution-profile contract, and this structural validator before regenerating into a temporary tree and comparing every owned path. Therefore `nix run .#pkl-check-generated` and `nix flake check` enforce logical relationships, target bindings, structure, path/count coverage, and byte parity together. Local-only settings, dependency artifacts, and package locks remain outside generation/parity ownership.
