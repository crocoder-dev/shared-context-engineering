# Decision: Resolve the Codex hook helper from the Git repository root at invocation time

Date: 2026-08-23
Status: Accepted
Plan: `context/plans/codex-cli-integration.md`
Task: `T18`

## Context

Codex runs project hooks with the event's current working directory, which can
be the repository root or an arbitrary nested directory. The generated hook
command must therefore locate the installed SCE helper without relying on the
process working directory or an install-time absolute path. Repository paths
may contain spaces, and hook failures must not block Codex when Git-root
resolution is unavailable. The command also forwards the hook's JSON STDIN to
the Rust dispatcher, so it must not consume, rewrite, or expose that payload.

## Decision

Generated Codex hook commands resolve `git rev-parse --show-toplevel` at
invocation time, invoke the repository-root `.codex/hooks` helper with quoted
shell expansions, and exit successfully without output when Git-root
resolution fails.

## Rationale

Runtime root resolution works from both root and nested Codex working
 directories while avoiding a machine-specific absolute install path. Quoted
expansions preserve repository paths containing spaces. Capturing the Git
command's result keeps its diagnostics out of hook output, and the explicit
fail-open branch preserves Codex's non-blocking hook contract. Passing the
command through the existing helper keeps missing-CLI guidance and STDIN
forwarding in one SCE-owned boundary.

## Alternatives considered

- **Use the current working directory with a relative helper path** — fails for
  nested Codex event directories.
- **Embed an absolute helper path during setup** — is not portable across
  machines, checkouts, or repository moves.
- **Use `eval` or reconstruct the command from unquoted path text** — risks
  shell interpretation and breaks paths containing spaces; it also adds no
  capability beyond quoted parameter expansion.

## Compatibility and risks

- Existing Codex hook registrations and the `.codex/hooks` helper remain the
  same; only command invocation becomes independent of the event cwd.
- A hook invoked outside a Git working tree becomes a silent successful no-op,
  preserving fail-open behavior but producing no SCE evidence.
- The command depends on Git being available at hook runtime, as does the
  repository-root-aware Codex path contract; generated tests cover root,
  nested, spaced-path, and Git-failure cases.

## Guardrails

- Keep exactly the four existing registrations: `UserPromptSubmit`, `Stop`,
  `PreToolUse` for `Bash`, and `PostToolUse` for `apply_patch`.
- Keep all root and helper expansions quoted and do not use `eval`.
- Preserve the helper's existing missing-`sce` stderr guidance and direct STDIN
  forwarding.
- Do not add absolute install-time paths, a new registration system, or a
  `PreToolUse apply_patch` registration.

## Consequences

- Generated Codex hooks work from arbitrary nested repository directories and
  repositories whose paths contain spaces.
- Hook installation remains relocatable, and failure to resolve a Git root is
  non-blocking and silent.
- The generated contract and flake check must continue to exercise invocation
  behavior rather than only inspect the JSON shape.

## Follow-up

- `T19` must retain this invocation contract while proving and documenting the
  complete hardened Codex pipeline.

## References

- Plan: [`codex-cli-integration`](../plans/codex-cli-integration.md)
- Task: `T18`
- Current-state context: [`codex-integration-runtime`](../sce/codex-integration-runtime.md)
- Evidence: [`test-codex-hook-command.sh`](../../scripts/test-codex-hook-command.sh)
- Evidence: [`codex-content.pkl`](../../config/pkl/renderers/codex-content.pkl)
