# Instruction unit validator

## Scope

The repository-owned instruction-unit validator is implemented entirely in Pkl:

- `config/pkl/renderers/instruction-unit-validator.pkl` owns validation logic, the deterministic 60-projection rendered-model input set, and direct loading of 103 committed projected instruction files.
- `config/pkl/renderers/instruction-unit-validator-check.pkl` owns valid and invalid fixture checks plus the evaluation gate.

Run the focused validation with:

```bash
nix develop -c pkl eval \
  config/pkl/renderers/instruction-unit-validator-check.pkl \
  -x summary
```

A passing result reports `productionUnitCount = 60`, `generatedFileUnitCount = 103`, zero rendered-model and generated-file diagnostics, eight valid fixtures, 18 invalid fixtures, zero fixture failures, and `status = "VALIDATION_OK"`.

## Input ownership

Canonical manual and automated bodies are authored as typed `InstructionBody` sections and serialized by the shared `renderBody` boundary before target rendering. Production validation consumes the resulting document objects from the manual OpenCode, Claude, and Pi renderers and the automated OpenCode renderer. Unit paths, kinds, profiles, targets, and slugs come from `instruction-unit-inventory.pkl`; the resulting unit list is sorted by destination path before validation.

The same explicit projection inventory drives direct validation of 60 approved config instruction destinations and 43 tracked manual root mirrors. Generated-file inputs are projection-path-sorted and parsed into frontmatter/body before applying the same rules, while generated-output parity separately proves byte equality for all generation-owned files. Pi profile prompts have no approved projection or generated compatibility output; any stale `agent-*` prompt in a parity-owned Pi prompt directory is detected as generated drift.

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
- OpenCode workflows binding the canonical profile agent, remaining non-subtask, declaring canonical entry/required skills, and matching capability-derived permissions;
- Claude workflows carrying the correct composed-profile marker and guardrails with capability-derived `allowed-tools`;
- Pi workflows carrying the correct composed-profile marker and guardrails, requiring the full project-local entry-skill read, and resolving that skill to its generated path.

Diagnostics use the stable shape:

```text
<path> [<agent|command|skill>] <rule>: <message>; expected: <shape>
```

## Fixtures

The Pkl check module includes valid agent, command, skill, manual-profile, and automated-profile fixtures plus valid OpenCode-native, Claude-composed, and Pi-composed workflow bindings. Its 18 invalid fixtures retain the ten structural/frontmatter/skill-reference cases and add missing OpenCode primary mode, mismatched workflow agent, missing `subtask: false`, missing/wrong composed marker, missing composed guardrail, excessive Claude tools, and missing Pi entry-skill read coverage.

Logical-reference, capability-ceiling, projection-classification/destination, unresolved Pi skill-path, and stale Pi prompt cases use ten additional typed fixtures in `portable-execution-profile-check.pkl`, because malformed canonical objects cannot inhabit the production Pkl types.

The check module constrains production diagnostics and fixture-failure listings to be empty, so evaluation fails when the production model becomes invalid or a fixture no longer proves its expected rule.

## Integration boundary

`config/pkl/generate.pkl` emits both config instruction outputs and the tracked manual root mirrors under `.opencode/`, `.claude/`, and `.pi/`, plus the root contributor templates `templates/execution-profile.md`, `templates/workflow.md`, and `templates/skill.md`. The templates use canonical logical-unit vocabulary: profile policy owns harness-neutral capability intent, workflow policy may only narrow it, target metadata translates capabilities, and projection metadata classifies enforcement. `config/pkl/check-generated.sh` checks all generation-owned config outputs, root instruction mirrors, and templates. Both `config/pkl/check-generated.sh` and the root flake's `pkl-parity` check evaluate metadata coverage, the portable execution-profile contract, and this structural validator before regenerating into a temporary tree and comparing every owned path. Therefore `nix run .#pkl-check-generated` and `nix flake check` enforce logical relationships, target bindings, structure, path/count coverage, and byte parity together. Local-only settings, dependency artifacts, and package locks remain outside generation/parity ownership.
