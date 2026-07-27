# Bash Policy `satisfied_by` Wrapper Exemption

Custom `policies.bash` entries may declare an optional `satisfied_by` field: a list of wrapper argv prefixes that already satisfy the policy. When the matched command was unwrapped from one of those wrappers, the policy does **not** fire, so a policy steering `rg` toward nix stays quiet for `nix shell nixpkgs#ripgrep -c rg ...` while still blocking a bare `rg`.

This is the abstract rule chosen over a boolean toggle: `satisfied_by` reuses the same argv-prefix vocabulary as `argv_prefix`, so it is not nix-specific and a policy can name any wrapper that already satisfies its intent.

## Config shape

```json
{
  "id": "use-nix-for-ripgrep",
  "match": { "argv_prefix": ["rg"] },
  "satisfied_by": [["nix", "shell", "nixpkgs#ripgrep"]],
  "message": "Run `rg` through nix: `nix run nixpkgs#ripgrep -- <args>`."
}
```

- `satisfied_by` is optional on custom entries only; presets never declare it and are never exempted by wrappers.
- Each entry in `satisfied_by` is a non-empty array of non-empty strings (same per-token rules as `argv_prefix`). An empty `satisfied_by` prefix is a validation error.
- A custom policy with no `satisfied_by` keeps the original behavior: wrapping does not launder it.

## Matching model

Normalization now tracks the argv of each wrapper a segment was unwrapped from, outermost first. In `cli/src/services/bash_policy.rs` this is `NormalizedSegment { argv, wrappers }`; `normalize_segment(segment, wrappers)` threads the wrapper chain through recursive nested-shell unwrapping.

A policy matches a normalized segment only when **both** hold:

1. the segment argv starts with the policy's `argv_prefix`, **and**
2. no wrapper in `normalized.wrappers` starts with any prefix listed in the policy's `satisfied_by`.

`policy_is_satisfied_by_wrapper` performs the wrapper-prefix check using the same `argv_starts_with` helper as `argv_prefix` matching. `select_matching_policy` filters out policies whose `satisfied_by` already covers the current wrapper chain before applying the existing longest-prefix / custom-over-preset / order precedence.

## Examples

Policy `use-nix-for-rg` with `argv_prefix=["rg"]`, `satisfied_by=[["nix","shell","nixpkgs#ripgrep"]]`:

- `rg pattern src` -> blocked (bare invocation)
- `nix shell nixpkgs#ripgrep -c rg pattern src` -> allowed (satisfying wrapper exempts the policy)
- `nix shell nixpkgs#ripgrep -c sh -c 'rg pattern src'` -> allowed (exemption survives a nested shell payload)
- `nix shell nixpkgs#fd -c rg pattern src` -> blocked (a different nix package does not satisfy the policy)
- `sh -c 'rg pattern src'` -> blocked (a plain shell wrapper does not satisfy the policy)

The design point: policies declaring no `satisfied_by` cannot be laundered by wrapping. `nix develop -c cargo test` and `nix shell nixpkgs#cargo -c cargo test` stay blocked under a `cargo test` policy with no `satisfied_by`.

## Scope

- Custom-policy-only; presets cannot declare satisfying wrappers.
- Wrapper matching is exact argv-prefix only (same rules as `argv_prefix`); no regex/glob/substring/case-folding.
- `nix run nixpkgs#ripgrep -- <args>` has no `-c` to unwrap, so it is incidentally allowed regardless of `satisfied_by`; the wrapper exemption is what makes the `nix shell … -c` form also allowed.

## Related files

- `cli/src/services/bash_policy.rs` (`NormalizedSegment`, `normalize_segment`, `policy_is_satisfied_by_wrapper`, `select_matching_policy`)
- `cli/src/services/config/policy.rs` (`CustomBashPolicyEntry.satisfied_by`, `parse_custom_bash_policy_satisfied_by`, text-summary rendering)
- `cli/src/services/config/schema.rs` (`ParsedCustomBashPolicyEntryDocument.satisfied_by`)
- `config/pkl/base/sce-config-schema.pkl` and generated `config/schema/sce-config.schema.json` + `cli/assets/generated/config/schema/sce-config.schema.json`
- `.sce/config.json` (all ten `use-nix-for-*` custom entries now declare their `nix shell nixpkgs#<pkg>` satisfying wrapper)
- `context/sce/bash-tool-policy-enforcement-contract.md` (parent contract)

See also: [bash-tool-policy-enforcement-contract.md](./bash-tool-policy-enforcement-contract.md), [../glossary.md](../glossary.md), [../context-map.md](../context-map.md)