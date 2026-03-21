# Plan: Remove Prompt Collection/Tracing for 0.2.0

## Change Summary

Remove prompt collecting/tracing functionality from the generated Claude Code and OpenCode configuration while preserving:
1. Git hooks with automatic SCE attribution (`Co-authored-by: SCE <sce@crocoder.dev>`)
2. SCE bash policies for both Claude and OpenCode

The prompt capture feature is not ready for 0.2.0 and should be removed from the release.

## Success Criteria

1. Prompt capture hooks are removed from Claude settings and generated outputs
2. Bash policy enforcement remains functional for both Claude and OpenCode
3. Git hooks (pre-commit, commit-msg, post-commit) remain unchanged and continue to attribute SCE
4. All context files accurately reflect current state (prompt capture docs removed, bash policy docs retained)
5. `nix flake check` passes
6. `nix run .#pkl-check-generated` passes

## Constraints and Non-Goals

**Constraints:**
- No changes to CLI code (git hooks implementation stays as-is)
- No changes to bash policy enforcement
- Generated outputs must remain deterministic and in sync with PKL sources

**Non-Goals:**
- This plan does not modify the CLI's agent-trace service code
- This plan does not remove other agent-trace context files (those may be cleaned up separately)
- This plan does not change git hook behavior

## Task Stack

- [x] T01: `Remove prompt capture sources from PKL lib` (status:done)
  - Task ID: T01
  - Goal: Remove prompt capture source files and update settings.json to remove UserPromptSubmit hook
  - Boundaries (in/out of scope):
    - In: Delete `config/pkl/lib/claude-capture-prompt.js`
    - In: Delete `config/pkl/lib/claude-prompt-capture-hook.json`
    - In: Update `config/pkl/lib/claude-settings.json` to remove UserPromptSubmit hook (keep PreToolUse for bash policy)
    - In: Update `config/pkl/generate.pkl` to remove prompt capture file generation
    - Out: No changes to bash policy files
    - Out: No changes to CLI code
  - Done when: PKL sources no longer reference prompt capture; `claude-settings.json` only contains PreToolUse hook for bash policy
  - Verification notes: `cat config/pkl/lib/claude-settings.json` shows only PreToolUse hook; `grep -r "capture-prompt" config/pkl/` returns no matches
  - Completed: 2026-03-21
  - Files changed: config/pkl/lib/claude-capture-prompt.js (deleted), config/pkl/lib/claude-prompt-capture-hook.json (deleted), config/pkl/lib/claude-settings.json (modified), config/pkl/generate.pkl (modified)
  - Evidence: pkl-check-generated passed, nix flake check passed

- [x] T02: `Regenerate and verify generated outputs` (status:done)
  - Task ID: T02
  - Goal: Regenerate config outputs and verify prompt capture files are removed
  - Boundaries (in/out of scope):
    - In: Run `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - In: Verify `config/.claude/settings.json` no longer has UserPromptSubmit
    - In: Verify `config/.claude/hooks/sce-capture-prompt.js` and `.json` are removed
    - In: Verify bash policy files remain in generated outputs
    - Out: No manual edits to generated files
  - Done when: Generated outputs match PKL sources; prompt capture files removed; bash policy files present
  - Verification notes: `nix run .#pkl-check-generated` passes; `ls config/.claude/hooks/` shows only `sce-bash-policy-hook.js`
  - Completed: 2026-03-21
  - Files changed: config/.claude/settings.json (regenerated), config/.claude/hooks/sce-capture-prompt.js (deleted), config/.claude/hooks/sce-capture-prompt.json (deleted)
  - Evidence: pkl-check-generated passed, hooks directory shows only sce-bash-policy-hook.js

- [x] T03: `Remove prompt capture context documentation` (status:done)
  - Task ID: T03
  - Goal: Remove context files for prompt capture feature and update context-map
  - Boundaries (in/out of scope):
    - In: Delete `context/sce/agent-trace-prompt-capture-hook.md`
    - In: Delete `context/sce/agent-trace-prompt-persistence-metrics.md`
    - In: Delete `context/sce/agent-trace-prompt-query-command.md`
    - In: Update `context/context-map.md` to remove references to deleted files
    - In: Update `context/overview.md` to remove prompt capture references
    - Out: Keep all other agent-trace context files (git hooks, bash policy, etc.)
    - Out: Keep `context/sce/bash-tool-policy-enforcement-contract.md`
  - Done when: Context files removed; context-map and overview updated; no orphaned references
  - Verification notes: `grep -r "prompt-capture\|prompt capture\|UserPromptSubmit" context/` returns no matches
  - Completed: 2026-03-21
  - Files changed: context/sce/agent-trace-prompt-capture-hook.md (deleted), context/sce/agent-trace-prompt-persistence-metrics.md (deleted), context/sce/agent-trace-prompt-query-command.md (deleted), context/context-map.md (modified), context/overview.md (modified), context/glossary.md (modified), context/sce/agent-trace-pre-commit-staged-checkpoint.md (modified)
  - Evidence: pkl-check-generated passed, grep verification shows no prompt capture references in context/

- [x] T04: `Run full validation suite` (status:done)
  - Task ID: T04
  - Goal: Execute all validation checks and confirm clean state
  - Boundaries (in/out of scope):
    - In: Run `nix flake check`
    - In: Run `nix run .#pkl-check-generated`
    - In: Verify generated files are in sync
    - Out: No code changes in this task
  - Done when: All checks pass; generated outputs match sources
  - Verification notes: `nix flake check` exits 0; `nix run .#pkl-check-generated` exits 0
  - Completed: 2026-03-21
  - Files changed: None (validation-only task)
  - Evidence: nix flake check passed, pkl-check-generated passed, no prompt capture references in config/ or context/

## Open Questions

None - the scope is clear from the user's request.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (cli-tests, cli-clippy, cli-fmt, pkl-parity all passed)
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date)
- `grep -r "capture-capture\|UserPromptSubmit" config/` -> no matches
- `grep -r "prompt-capture\|prompt capture\|UserPromptSubmit" context/` -> no matches (excluding plan file)

### Success-criteria verification
- [x] Prompt capture hooks are removed from Claude settings and generated outputs -> confirmed via `config/.claude/settings.json` contains only PreToolUse hook
- [x] Bash policy enforcement remains functional for both Claude and OpenCode -> confirmed via `config/.claude/hooks/sce-bash-policy-hook.js` exists, `config/.claude/lib/bash-policy-*` files exist
- [x] Git hooks (pre-commit, commit-msg, post-commit) remain unchanged and continue to attribute SCE -> confirmed via `cli/assets/hooks/` unchanged
- [x] All context files accurately reflect current state (prompt capture docs removed, bash policy docs retained) -> confirmed via grep verification
- [x] `nix flake check` passes -> exit 0
- [x] `nix run .#pkl-check-generated` passes -> exit 0

### Residual risks
- None identified.

## Files Affected

### PKL Sources (to be modified/deleted):
- `config/pkl/lib/claude-capture-prompt.js` - DELETE
- `config/pkl/lib/claude-prompt-capture-hook.json` - DELETE
- `config/pkl/lib/claude-settings.json` - MODIFY (remove UserPromptSubmit)
- `config/pkl/generate.pkl` - MODIFY (remove prompt capture generation)

### Generated Files (to be removed by regeneration):
- `config/.claude/hooks/sce-capture-prompt.js`
- `config/.claude/hooks/sce-capture-prompt.json`
- `config/.claude/settings.json` - REGENERATED

### Context Files (to be deleted):
- `context/sce/agent-trace-prompt-capture-hook.md`
- `context/sce/agent-trace-prompt-persistence-metrics.md`
- `context/sce/agent-trace-prompt-query-command.md`

### Context Files (to be updated):
- `context/context-map.md`
- `context/overview.md`

### Files to Keep Unchanged:
- `config/pkl/lib/bash-policy-runtime.js`
- `config/pkl/lib/opencode-bash-policy-plugin.js`
- `config/pkl/lib/claude-bash-policy-hook.js`
- `config/pkl/data/bash-policy-presets.json`
- `config/.opencode/plugins/sce-bash-policy.js`
- `config/.opencode/lib/bash-policy-*`
- `config/.claude/lib/bash-policy-*`
- `config/.claude/hooks/sce-bash-policy-hook.js`
- `cli/assets/hooks/*` (all git hooks)
- `context/sce/bash-tool-policy-enforcement-contract.md`
- `context/sce/agent-trace-commit-msg-coauthor-policy.md`
- All other agent-trace context files