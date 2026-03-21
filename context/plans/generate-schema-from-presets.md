# Plan: Generate JSON Schema Preset Enum from bash-policy-presets.pkl

## Change summary

Refactor `sce-config-schema.pkl` to dynamically generate its preset enum values and mutually exclusive validation constraints from `bash-policy-presets.pkl` instead of hardcoding them. This establishes a single source of truth for preset definitions and prevents drift between the preset catalog and the JSON schema.

## Success criteria

- `sce-config-schema.pkl` imports `bash-policy-presets.pkl` and derives all preset-related enum values from it
- The `policies.bash.presets` enum is generated from the preset catalog IDs
- The `policies.bash.custom.id` `not` constraint (reserved IDs) is generated from the preset catalog IDs
- The `policies.bash.presets.allOf` mutually exclusive constraint is generated from the preset catalog's `mutually_exclusive` field
- Running `nix run .#pkl-check-generated` passes with the new generated schema
- The rendered `config/schema/sce-config.schema.json` contains identical preset enum values and validation rules as before (no functional change to the schema content)

## Constraints and non-goals

**Constraints:**
- Must maintain backward compatibility with existing `sce/config.json` files
- The generated JSON schema must be valid JSON Schema draft 2020-12
- Pkl syntax must remain valid and render correctly

**Non-goals:**
- No changes to `bash-policy-presets.pkl` structure or content
- No changes to runtime enforcement code (OpenCode plugin, Claude hook)
- No changes to CLI config parsing
- Redundancy warnings remain in runtime JSON only (not surfaced in JSON schema)

## Task stack

### T01: Refactor sce-config-schema.pkl to import and use bash-policy-presets.pkl

- **Task ID:** T01
- **Status:** done
- **Completed:** 2026-03-21
- **Goal:** Restructure `sce-config-schema.pkl` to dynamically build the JSON schema using preset data imported from `bash-policy-presets.pkl`
- **Boundaries (in/out of scope):**
  - In: Import statement, dynamic enum generation, dynamic mutually exclusive constraint generation, dynamic reserved ID list generation
  - Out: Changes to `bash-policy-presets.pkl`, changes to runtime enforcement, changes to CLI parsing
- **Done when:**
  - `sce-config-schema.pkl` imports `bash-policy-presets.pkl`
  - Preset enum values are derived from `presets` list IDs
  - Mutually exclusive `allOf` constraint is derived from `mutually_exclusive` field
  - Reserved ID `not` constraint is derived from preset IDs
  - `nix run .#pkl-check-generated` passes
- **Verification notes:**
  - `nix run .#pkl-check-generated`
  - Manually inspect `config/schema/sce-config.schema.json` to confirm enum values match expected: `["forbid-git-all", "forbid-git-commit", "use-pnpm-over-npm", "use-bun-over-npm", "use-nix-flake-over-cargo"]`
- **Files changed:** `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json`
- **Evidence:** `nix flake check` passed; preset enum, mutually exclusive constraint, and reserved ID constraint all verified via jq

### T02: Validate generated schema matches expected output

- **Task ID:** T02
- **Status:** done
- **Completed:** 2026-03-21
- **Goal:** Confirm the generated JSON schema is functionally equivalent to the previous hardcoded version
- **Boundaries (in/out of scope):**
  - In: Compare generated schema against expected preset enum, mutually exclusive constraint, and reserved ID list
  - Out: Changes to Pkl files
- **Done when:**
  - Generated `config/schema/sce-config.schema.json` contains correct preset enum values
  - Generated schema contains correct mutually exclusive validation for `use-pnpm-over-npm` and `use-bun-over-npm`
  - Generated schema contains correct reserved ID list in `custom.id` `not` constraint
- **Verification notes:**
  - `cat config/schema/sce-config.schema.json | jq '.properties.policies.properties.bash.properties.presets.items.enum'`
  - `cat config/schema/sce-config.schema.json | jq '.properties.policies.properties.bash.properties.presets.allOf'`
  - `cat config/schema/sce-config.schema.json | jq '.properties.policies.properties.bash.properties.custom.items.properties.id.not'`
- **Files changed:** None (verification-only task)
- **Evidence:** All three jq checks passed; preset enum, mutually exclusive constraint, and reserved ID constraint all verified

### T03: Run full validation suite

- **Task ID:** T03
- **Status:** done
- **Completed:** 2026-03-21
- **Goal:** Ensure all repository checks pass after the change
- **Boundaries (in/out of scope):**
  - In: Running `nix flake check`, verifying generated output parity
  - Out: Changes to any files
- **Done when:**
  - `nix flake check` passes
  - `nix run .#pkl-check-generated` passes
- **Verification notes:**
  - `nix flake check`
  - `nix run .#pkl-check-generated`
- **Files changed:** None (verification-only task)
- **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity); `nix run .#pkl-check-generated` reported "Generated outputs are up to date."

## Open questions

None. All clarifications resolved:
- Mutually exclusive validation: Generate from `bash-policy-presets.pkl`
- Redundancy warnings: Keep in runtime only (not in JSON schema)

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (cli-tests, cli-clippy, cli-fmt, pkl-parity all passed)
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date.")
- `jq '.properties.policies.properties.bash.properties.presets.items.enum' config/schema/sce-config.schema.json` -> confirmed preset enum values
- `jq '.properties.policies.properties.bash.properties.presets.allOf' config/schema/sce-config.schema.json` -> confirmed mutually exclusive constraint
- `jq '.properties.policies.properties.bash.properties.custom.items.properties.id.not' config/schema/sce-config.schema.json` -> confirmed reserved ID constraint

### Temporary scaffolding
- None removed (no temporary files created during implementation)

### Success-criteria verification
- [x] `sce-config-schema.pkl` imports `bash-policy-presets.pkl` and derives all preset-related enum values from it -> confirmed via T01 implementation
- [x] The `policies.bash.presets` enum is generated from the preset catalog IDs -> confirmed: `["forbid-git-all", "forbid-git-commit", "use-pnpm-over-npm", "use-bun-over-npm", "use-nix-flake-over-cargo"]`
- [x] The `policies.bash.custom.id` `not` constraint (reserved IDs) is generated from the preset catalog IDs -> confirmed via jq output
- [x] The `policies.bash.presets.allOf` mutually exclusive constraint is generated from the preset catalog's `mutually_exclusive` field -> confirmed: `use-pnpm-over-npm` and `use-bun-over-npm` are mutually exclusive
- [x] Running `nix run .#pkl-check-generated` passes with the new generated schema -> exit 0, "Generated outputs are up to date."
- [x] The rendered `config/schema/sce-config.schema.json` contains identical preset enum values and validation rules as before (no functional change to the schema content) -> confirmed via T02 verification

### Residual risks
- None identified.