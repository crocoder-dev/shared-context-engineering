# Instruction unit validator

## Scope

The repository-owned instruction-unit validator is implemented entirely in Pkl:

- `config/pkl/renderers/instruction-unit-validator.pkl` owns validation logic and the deterministic 62-unit production input set.
- `config/pkl/renderers/instruction-unit-validator-check.pkl` owns valid and invalid fixture checks plus the evaluation gate.

Run the focused validation with:

```bash
nix develop -c pkl eval \
  config/pkl/renderers/instruction-unit-validator-check.pkl \
  -x summary
```

A passing result reports `productionUnitCount = 62`, zero production diagnostics, five valid fixtures, ten invalid fixtures, zero fixture failures, and `status = "VALIDATION_OK"`.

## Input ownership

Production validation consumes the rendered document objects from the manual OpenCode, Claude, and Pi renderers and the automated OpenCode renderer. Unit paths, kinds, profiles, targets, and slugs come from `instruction-unit-inventory.pkl`; the resulting unit list is sorted by destination path before validation.

This keeps validation on the Pkl-authored/rendered model rather than rediscovering the same units through a parallel filesystem inventory. Root-mirror file validation remains deferred to T10.

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

T09 provides the standalone Pkl validator and focused check module. Flake/parity invocation and validation of generated root mirrors are not implemented yet; those belong to T10 of `context/plans/instruction-unit-standardization-all-harnesses.md`.
