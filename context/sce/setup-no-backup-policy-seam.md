# Setup remove-and-replace install policy

`cli/src/services/setup/mod.rs` uses a unified remove-and-replace policy for all setup-managed write flows. There is no backup creation or backup-based rollback.

## Current state

- Both config install (`.opencode`/`.claude`) and required hook install use the same remove-and-replace choreography:
  1. Write canonical content to a unique staging file.
  2. Remove the existing target (if present) directly.
  3. Swap the staged content into the final target path.
  4. On swap failure, clean the staging artifact and return deterministic recovery guidance (recover from version control if needed).
- No `.backup` artifacts are created during any setup write flow.
- No backup-based rollback is attempted on swap failure.
- Recovery guidance is generic (not git-specific wording): "Setup does not create backups. Recover '<path>' from version control if needed."

## Implemented behavior

- Config install removes the existing target directory before swapping staged content. On swap failure, it cleans the staging artifact and returns recovery guidance.
- Required hook install removes the existing hook file before swapping staged content. On swap failure, it cleans the staging artifact and returns recovery guidance.
- Success output reports target, file count, and per-hook status (`installed`/`updated`/`skipped`) without any backup-related lines.

## Scope boundary

- This file captures the unified remove-and-replace install policy and its use by both config-install and required-hook install flows.
- Future setup-managed write flows should follow the same remove-and-replace pattern instead of introducing backup creation.

See also: [../overview.md](../overview.md), [../context-map.md](../context-map.md), [setup-githooks-install-flow.md](setup-githooks-install-flow.md)