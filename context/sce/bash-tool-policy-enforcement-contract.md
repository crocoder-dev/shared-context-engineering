# Bash Tool Policy Enforcement Contract

## Scope

Task `bash-tool-policy-enforcement` `T01` defines the canonical contract for repo-configured bash-tool command blocking.
This document is the implementation target for `T02` through `T08`.

In scope for this contract:

- config keys and schema shape in `.sce/config.json`
- deterministic command normalization and prefix matching rules
- custom blocked-command entries and built-in preset activation
- fixed preset-owned denial messages and overlap/conflict rules
- operator-visible block behavior and cross-target parity expectations
- explicit matching limitations for later implementation tasks

Out of scope for this contract task:

- Rust config parser implementation
- generated OpenCode or Claude enforcement assets
- setup/install wiring for new enforcement files

## Config surface contract

The blocked-command policy surface is owned by a nested policy namespace so future policy domains can live beside bash enforcement under one stable root:

```json
{
  "policies": {
    "bash": {
      "presets": ["forbid-git-commit", "use-nix-flake-over-cargo"],
      "custom": [
        {
          "id": "prefer-jj-status",
          "match": {
            "argv_prefix": ["git", "status"]
          },
          "message": "Use `jj status` instead of `git status`."
        },
        {
          "id": "block-rm",
          "match": {
            "argv_prefix": ["rm"]
          },
          "message": "This repository does not allow `rm` via the bash tool."
        }
      ]
    }
  }
}
```

`policies` is the stable top-level policy namespace.
This task defines only one child policy domain:

- `bash`: bash-tool command blocking policy

`policies.bash` contains exactly two optional keys:

- `presets`: array of preset IDs
- `custom`: array of custom policy entries

If `policies` is absent, `policies.bash` is absent, or both arrays are omitted/empty, no bash-tool command policy blocks are active.

No other `policies.*` domains are defined by this task.
Downstream work may add sibling policy domains later without reshaping the `policies.bash` contract.

## Validation contract

- `policies` must be a JSON object when present.
- Unknown top-level config keys outside the repo's approved config schema still fail validation.
- Unknown keys under `policies` fail validation.
- `policies.bash` must be a JSON object when present.
- Unknown keys under `policies.bash` fail validation.
- `presets` must be an array of unique strings.
- Each preset ID must be one of the built-in IDs defined in this contract.
- `custom` must be an array of objects.
- Each custom policy must contain exactly `id`, `match`, and `message`.
- Custom `id` values must be unique within `custom` and must not collide with preset IDs.
- `message` must be a non-empty string.
- `match` must be an object containing exactly `argv_prefix`.
- `argv_prefix` must be a non-empty array of non-empty strings.
- Exact duplicate custom `argv_prefix` values are invalid to avoid ambiguous denial messages.
- `use-pnpm-over-npm` and `use-bun-over-npm` are mutually exclusive and fail validation if both are enabled.
- `forbid-git-all` and `forbid-git-commit` may both be enabled; this is valid but redundant.

## Matching model

### Normalized command shape

Policy evaluation operates on one normalized argv token list resolved from `policies.bash`.

Normalization rules are deterministic and intentionally limited:

1. Start from the bash-tool command string before execution.
2. Tokenize it using shell-style quoting rules that preserve quoted substrings as one token.
3. If tokenization fails, do not emit a policy denial; fall through to the normal tool/runtime path.
4. Drop leading environment assignments like `FOO=bar`.
5. Unwrap these leading wrapper binaries only:
   - `env`
   - `/usr/bin/env`
   - `command`
   - `nohup`
   - `sudo`
6. After wrapper removal, normalize the executable token to its basename, so `/usr/bin/git` matches `git`.
7. Leave all remaining argv tokens unchanged and compare them case-sensitively.

Example normalized argv results:

- `git commit -m "msg"` -> `["git", "commit", "-m", "msg"]`
- `FOO=bar /usr/bin/git push` -> `["git", "push"]`
- `sudo npm install` -> `["npm", "install"]`

### Intentional limitations (original scope)

This contract does not require full shell parsing.
The enforcement layer only needs to reason about the single normalized argv produced by the rules above.

The following are intentionally out of scope for matching guarantees in this plan:

- alias expansion, functions, subshells, and process substitution
- parsing command intent from comments, heredocs, or multi-line scripts

## Shell Operator Parsing Extension (2026-03)

The original contract intentionally excluded shell control operators (`|`, `&&`, `||`, `;`, `&`) from the matching model. A subsequent plan (`shell-operator-parsing`) extended the implementation to handle these operators:

**Extended behavior:**
- Commands are split into segments at shell control operators (`|`, `&&`, `||`, `;`, `&`)
- Each segment is normalized independently using the same normalization rules
- If ANY segment matches a blocking policy, the entire command is blocked
- This applies to both preset policies (e.g., `forbid-git-all`) and custom policies

**Implementation:**
- `parseCommandSegments()` function in `config/lib/bash-policy-plugin/bash-policy/runtime.ts`
- Integrates with existing `evaluateBashCommandPolicy()` function
- Preserves original single-command behavior for commands without operators

**Examples:**
- `cat abc | git diff` with `forbid-git-all` -> blocked (segment "git diff" matches)
- `git status && npm install` with `forbid-git-all` -> blocked (segment "git status" matches)
- `ls; git push` with `forbid-git-commit` -> blocked (segment "git push" matches)
- `cat file | ls` with no git policies -> allowed (no segment matches git policies)

## Nested Shell Extension (2026-03)

The enforcement layer now also inspects a narrow set of nested command wrappers so repo policies still apply when commands are routed through shell entrypoints commonly used by Nix workflows.

**Extended behavior:**
- `nix ... -c <argv...>` and `nix ... --command <argv...>` unwrap to the nested argv after the `-c` or `--command` flag
- `sh -c "..."`, `bash -c "..."`, and combined short-option forms such as `bash -lc "..."` parse the nested command string into shell segments
- Nested parsing is recursive, so `nix develop -c sh -c 'cd cli && cargo fmt --check'` is evaluated against the inner `cargo fmt --check` argv
- Repo policy examples should distinguish verification from autofix: blocking `cargo test` or `cargo fmt --check` is compatible with still allowing direct `cargo fmt` when the repository keeps formatter autofix flows separate from verification
- If any nested segment matches a blocking policy, the full command is blocked

**Still out of scope:**
- shells or wrappers other than the explicit cases above
- arbitrary script files passed to shells without `-c`
- alias expansion, shell functions, and runtime evaluation features beyond tokenization of the `-c` payload

## Policy entry semantics

Every `policies.bash` entry matches by argv prefix.

- A full-binary block uses a one-token prefix such as `["git"]`.
- A narrower subcommand block uses a longer prefix such as `["git", "commit"]`.
- A policy matches when the normalized argv starts with the configured `argv_prefix` exactly.
- Matching is exact token equality only; no regex, glob, substring, or case-folding behavior is allowed.

Custom policy entries are repo-configured and own their own `message` text.

Built-in presets expand to one or more internal argv-prefix policy entries with repo-owned fixed messages.
Config does not support overriding a preset message while keeping the preset ID.

## Active-policy precedence

If multiple active policies match one normalized argv, the chosen denial is deterministic:

1. longest matching `argv_prefix`
2. custom policy over preset when prefix lengths tie
3. earlier custom entry order in config when multiple custom prefixes tie
4. preset catalog order defined in this contract when multiple preset prefixes tie

This allows a repository to add a narrower custom rule without redefining or mutating a built-in preset.

## Preset catalog

The initial preset catalog is fixed to these IDs and behaviors.

The canonical authored preset source lives at `config/pkl/base/bash-policy-presets.pkl` and is rendered to JSON by `config/pkl/generate.pkl` into generated target runtime assets at `lib/bash-policy-presets.json` so CLI validation and OpenCode enforcement share the same preset IDs, argv-prefix matchers, fixed messages, and conflict metadata.

### `forbid-git-all`

- Match prefixes: `["git"]`
- Denial message: `This repository blocks \`git\` via SCE bash-tool policy. Use \`jj\` or the repo-approved alternative instead.`

### `forbid-git-commit`

- Match prefixes:
  - `["git", "add"]`
  - `["git", "commit"]`
  - `["git", "push"]`
- Denial message: `This repository blocks direct \`git add\`, \`git commit\`, and \`git push\`. Use \`jj\` instead.`

### `use-pnpm-over-npm`

- Match prefixes: `["npm"]`
- Denial message: `This repository prefers \`pnpm\` over \`npm\`. Use \`pnpm\` instead.`

### `use-bun-over-npm`

- Match prefixes: `["npm"]`
- Denial message: `This repository prefers \`bun\` over \`npm\`. Use \`bun\` instead.`

### `use-nix-flake-over-cargo`

- Match prefixes: `["cargo"]`
- Denial message: `This repository prefers Nix flake entrypoints over direct \`cargo\` commands. Run Cargo through the documented \`nix develop\` / flake workflows instead.`

## Block behavior contract

Policy enforcement runs before the bash tool launches the command.

For a blocked command, the enforcement layer must:

- stop execution before any subprocess starts
- report that the command was blocked by policy
- identify the chosen policy ID
- surface the chosen policy message text exactly

For a non-matching command, enforcement must allow the bash tool to continue normally and must not inject unrelated warning text.

## Config reporting contract for downstream tasks

`T02` must treat `policies.bash` as a first-class validated config domain.
`sce config show` and `sce config validate` should expose:

- enabled preset IDs
- custom policy IDs, match prefixes, and messages
- validation failures for unknown preset IDs, invalid custom entries, conflicting preset combinations, and duplicate custom prefixes
- redundancy reporting for `forbid-git-all` plus `forbid-git-commit` without treating that pair as invalid

## Related files

- `context/plans/bash-tool-policy-enforcement.md`
- `context/cli/config-precedence-contract.md`
- `config/pkl/base/bash-policy-presets.pkl`
- `config/pkl/generate.pkl`
- `config/lib/bash-policy/bash-policy-runtime.ts`
- `config/lib/bash-policy/opencode-bash-policy-plugin.ts`
- `cli/src/services/config.rs`
