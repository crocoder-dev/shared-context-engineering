# Codex apply-patch diff evidence runtime

This document owns the existing `sce hooks codex` `PostToolUse(apply_patch)`
evidence path. It is complementary to the Codex mutation-scope adapter and does
not write mutation-scope state.

`cli/src/services/hooks/codex/apply_patch/` parses Codex's
`*** Begin Patch` ... `*** End Patch` format, including `Add File`, `Delete
File`, `Update File`, and optional `Move to` operations. Its stages are:

- `parser.rs` builds a typed `CodexPatch` from the raw
  `tool_input.command` text;
- `path.rs` resolves source and destination paths from the event `cwd` against
  the real Git root; and
- `normalize.rs` converts supported touched-line evidence into the SCE
  `Index:`-form unified diff accepted by `crate::services::patch::parse_patch`.

The outer intake unwraps only the upstream-compatible `<<EOF`, `<<'EOF'`, and
`<<\"EOF\"` forms. Unsupported prefixes or quoting, incomplete delimiters,
trailing garbage, malformed patches, and missing/non-string commands fail open
with no evidence. Parse failures use
`sce.hooks.codex.apply_patch.parse_failed`.

The handler requires an absolute event `cwd` inside the real Git worktree and
resolves every source and move destination independently. Valid `..` and
absolute-inside paths are accepted; missing Add File targets are checked through
their nearest existing prefix. Outside paths, malformed/NUL paths, symlink
escapes, and ambiguous mappings fail open before normalization or database
access and log `sce.hooks.codex.apply_patch.path_resolution_failed`. The
containment rule is recorded by
[`codex-canonical-worktree-path-resolution.md`](../decisions/2026-08-23-codex-canonical-worktree-path-resolution.md).

Normalization keeps touched `+`/`-` lines from Add and Update operations under
bounded, deterministic, event-scoped synthetic identities derived from
`tool_use_id`. Positions are evidence identities, not source line numbers.
Unchanged context lines are dropped; Delete File and empty Move-to operations
produce no evidence. A wholly empty normalized result is a successful no-op.
The identity scheme is recorded by
[`codex-event-scoped-apply-patch-evidence-identities.md`](../decisions/2026-08-23-codex-event-scoped-apply-patch-evidence-identities.md).

A non-empty result is persisted through the existing `insert_diff_trace` as one
`payload_type = "patch"` row with `tool_name = "codex"`, `tool_version = None`,
the idempotent `cx_<session_id>` session prefix, and a reported non-blank model
ID preserved without a fabricated provider prefix. Timestamp failure skips the
insert. Every successful or fail-open path returns empty stdout.

After commit, the existing `combine_patches` and `intersect_patches` behavior
uses the event-scoped identities and historical `kind`+`content` fallback. It
does not prove which physical occurrence of repeated identical content came
from a particular event. Delete File, pure rename, and Bash-created filesystem
mutations have no line-level evidence in this complementary pipeline.

This path writes only `diff_traces`; the Codex mutation-scope adapter writes
only `mutation_trace_*` rows. No Agent Trace schema migration, snapshot, or
Codex-specific Agent Trace builder is part of either path.

