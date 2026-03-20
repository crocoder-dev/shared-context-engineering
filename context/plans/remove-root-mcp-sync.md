# Change Summary

Retire repository-root `.mcp.json` as a synced compatibility artifact, keep `config/.mcp.json` as the only generated Claude MCP manifest, and update the sync/documentation contract so current behavior no longer assumes a root copy is needed.

# Success Criteria

- `nix run .#sync-opencode-config` no longer creates, replaces, or verifies repository-root `.mcp.json`.
- Repository state no longer tracks a root `.mcp.json` file for this workflow.
- Generated-config ownership docs consistently describe `config/.mcp.json` as the Claude MCP artifact and no longer describe root `.mcp.json` as a required synced target.
- Validation confirms generated-output parity and repo checks still pass without the root MCP copy.

# Constraints and Non-Goals

- In scope: sync-workflow behavior, tracked root artifact removal, and current-state context/docs that describe MCP config ownership and sync behavior.
- In scope: preserving `config/.mcp.json` generation as the canonical Claude MCP manifest.
- Out of scope: changing the `sce mcp` server contract, OpenCode MCP registration, or broader Claude integration beyond removal of the root compatibility copy.
- Out of scope: introducing a new runtime install location for Claude MCP config.
- Planning assumption: current operator intent is to keep only `config/.mcp.json` and not support tools that require repo-root `.mcp.json` auto-discovery.

# Task Stack

- [x] T01: `Stop syncing repository-root .mcp.json` (status:done)
  - Task ID: T01
  - Goal: Remove repository-root `.mcp.json` from the destructive sync workflow so `sync-opencode-config` only replaces generated `config/` and root `.opencode/`.
  - Boundaries (in/out of scope): In - `scripts/sync-opencode-config.sh`, related flake app/help text, and any immediate runtime messaging tied to sync behavior. Out - Pkl generation of `config/.mcp.json` and broader MCP server/schema changes.
  - Done when: sync help/output no longer mentions root `.mcp.json`, the script no longer stages/backs up/copies/verifies a root MCP file, and the synced-target contract is limited to `config/` plus root `.opencode/`.
  - Verification notes (commands or checks): `nix run .#sync-opencode-config -- --help`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-03-19
  - Files changed: `scripts/sync-opencode-config.sh`, `flake.nix`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/sce/mcp-generated-config-canonical-source.md`, `config/pkl/README.md`
  - Evidence: `nix run .#sync-opencode-config -- --help` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.

- [x] T02: `Remove root MCP artifact and repair context` (status:done)
  - Task ID: T02
  - Goal: Delete the tracked repository-root `.mcp.json` file and update current-state documentation/context so `config/.mcp.json` is the only documented Claude MCP artifact.
  - Boundaries (in/out of scope): In - root `.mcp.json` removal plus updates to `context/` and contributor docs that currently describe the root file as a synced target or standard surface. Out - changes to generated manifest contents or OpenCode profile docs unrelated to this ownership shift.
  - Done when: repository root no longer contains the tracked `.mcp.json` artifact, and context/docs consistently describe `config/.mcp.json` as canonical without a root mirror requirement.
  - Verification notes (commands or checks): inspect updated references in `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/sce/mcp-generated-config-canonical-source.md`, and `config/pkl/README.md`; `nix run .#pkl-check-generated`.
  - Completed: 2026-03-20
  - Files changed: `config/pkl/README.md`, `context/architecture.md`, `context/context-map.md`, `context/glossary.md`, `context/overview.md`, `context/sce/mcp-generated-config-canonical-source.md`
  - Evidence: repository-root `.mcp.json` absent from tracked files and worktree; targeted reference audit confirms `config/.mcp.json` is the only documented Claude MCP manifest; `nix run .#pkl-check-generated` passed.

- [x] T03: `Run validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Validate the removal end-to-end, confirm no stale references remain for root `.mcp.json`, and leave the repo in a clean current-state configuration.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, and final context-sync verification for the MCP ownership contract. Out - any new MCP feature work or follow-on compatibility layers.
  - Done when: validation passes, no required workflow still expects root `.mcp.json`, temporary investigation edits are absent, and current-state context matches the final behavior.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted search to confirm root `.mcp.json` is no longer described as a synced/required artifact.
  - Completed: 2026-03-20
  - Files changed: `context/plans/remove-root-mcp-sync.md`
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed; `test ! -e .mcp.json` passed; `git grep -n "repository-root \.mcp\.json\|root \.mcp\.json" -- . ':(exclude)context/plans/remove-root-mcp-sync.md'` returned no matches; root context files verified unchanged and aligned (`verify-only` context-sync pass).

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all 4 configured flake checks passed: `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`)
- `test ! -e .mcp.json` -> exit 0 (repository-root `.mcp.json` absent)
- `git ls-files --error-unmatch .mcp.json` -> exit 1 (expected: file is not tracked)
- `git grep -n "repository-root \.mcp\.json\|root \.mcp\.json" -- . ':(exclude)context/plans/remove-root-mcp-sync.md'` -> exit 1 (expected: no stale non-plan references)
- Context sync classification -> `verify-only`; verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` remain consistent with current code/config behavior.

### Success-criteria verification
- [x] `nix run .#sync-opencode-config` no longer creates, replaces, or verifies repository-root `.mcp.json` -> confirmed by completed T01 implementation evidence and passing generated parity / flake validation with no root `.mcp.json` artifact present.
- [x] Repository state no longer tracks a root `.mcp.json` file for this workflow -> confirmed by `test ! -e .mcp.json` and expected `git ls-files --error-unmatch .mcp.json` failure.
- [x] Generated-config ownership docs consistently describe `config/.mcp.json` as the Claude MCP artifact and no longer describe root `.mcp.json` as a required synced target -> confirmed by targeted repo search and verified current-state context files.
- [x] Validation confirms generated-output parity and repo checks still pass without the root MCP copy -> confirmed by exit-0 results from `nix run .#pkl-check-generated` and `nix flake check`.

### Failed checks and follow-ups
- None. Exit-1 results above were expected absence/no-match checks, not validation failures.

### Residual risks
- None identified for this task scope.

# Open Questions

- None; the scope is explicitly to retire repository-root `.mcp.json` and keep `config/.mcp.json` as the sole generated Claude MCP manifest.
