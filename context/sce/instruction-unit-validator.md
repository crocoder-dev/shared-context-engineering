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

A passing result reports `productionUnitCount = 60`, `generatedFileUnitCount = 103`, zero rendered-model and generated-file diagnostics, five valid fixtures, ten invalid fixtures, zero fixture failures, and `status = "VALIDATION_OK"`.

## Input ownership

Canonical manual and automated bodies are authored as typed `InstructionBody` sections and serialized by the shared `renderBody` boundary before target rendering. Production validation consumes the resulting document objects from the manual OpenCode, Claude, and Pi renderers and the automated OpenCode renderer. Unit paths, kinds, profiles, targets, and slugs come from `instruction-unit-inventory.pkl`; the resulting unit list is sorted by destination path before validation.

The same explicit projection inventory drives direct validation of 60 approved config instruction destinations and 43 tracked manual root mirrors. Generated-file inputs are projection-path-sorted and parsed into frontmatter/body before applying the same rules, while generated-output parity separately proves byte equality for all generation-owned files. Transitional Pi profile prompts remain parity-owned but are not approved projections; T08 removes them.

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
- OpenCode agent `permission.skill` entries resolving to that inventory, except wildcard `*`.

Diagnostics use the stable shape:

```text
<path> [<agent|command|skill>] <rule>: <message>; expected: <shape>
```

## Fixtures

The Pkl check module includes valid agent, command, skill, manual-profile, and automated-profile fixtures. It also proves the ten required invalid cases:

1. missing `Purpose`;
2. wrong section order;
3. duplicate section;
4. unknown heading;
5. non-final `Examples`;
6. body-level `When to use`;
7. missing frontmatter field;
8. skill name/directory mismatch;
9. invalid command skill reference;
10. invalid agent skill-permission reference.

The check module constrains production diagnostics and fixture-failure listings to be empty, so evaluation fails when the production model becomes invalid or a fixture no longer proves its expected rule.

## Integration boundary

`config/pkl/generate.pkl` emits both config instruction outputs and the tracked manual root mirrors under `.opencode/`, `.claude/`, and `.pi/`, plus the root `templates/` copies. `config/pkl/check-generated.sh` checks all generation-owned config outputs, root instruction mirrors, and templates. The root flake's `pkl-parity` check evaluates this validator before regenerating into a temporary tree and comparing every owned path, so `nix flake check` enforces both structure and parity. Local-only settings, dependency artifacts, and package locks remain outside generation/parity ownership.
