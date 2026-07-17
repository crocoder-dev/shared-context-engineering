# Plan: commit-skip-attribution-parity

## Change summary

Make the manual `/commit skip` and `/commit oneshot` paths apply the same SCE commit-message attribution policy as a manual `git commit`. The bypass command will prepare its generated message in a temporary file, explicitly run `sce hooks commit-msg` against that file, and then execute a normal hook-enabled `git commit -F <message-file>`. The installed Git `commit-msg` hook may run the policy a second time; the existing idempotent trailer dedupe contract keeps the result at exactly one canonical trailer.

## Success criteria

1. When the existing attribution gate is enabled and staged AI overlap is present, `/commit skip` and `/commit oneshot` create a commit containing exactly one `Co-authored-by: SCE <sce@crocoder.dev>` trailer.
2. When attribution is disabled or staged AI overlap is absent/erroring, the bypass paths do not force or hardcode the trailer.
3. The generated commit message is passed losslessly, including multiline bodies, through a temporary message file.
4. The command runs `sce hooks commit-msg` before creating the commit and stops without committing if that preflight fails.
5. The final `git commit` remains hook-enabled: no `--no-verify`, `SCE_DISABLED`, or `SCE_ATTRIBUTION_HOOKS_DISABLED` suppression is introduced.
6. Temporary message files are removed after both success and failure.
7. Regular manual `/commit` behavior and the automated profile remain unchanged.
8. Generated outputs remain deterministic and repository checks pass.

## Constraints and non-goals

- In scope: the canonical manual commit-command body in `config/pkl/base/shared-content-commit.pkl` and generator-owned outputs derived from it.
- In scope: manual OpenCode, Claude, and Pi command surfaces that share the canonical manual command body.
- In scope: current-state context describing bypass-mode attribution behavior.
- Out of scope: changing `sce hooks commit-msg`, its AI-overlap evidence gate, canonical trailer, opt-out precedence, or idempotent dedupe behavior.
- Out of scope: directly inserting the canonical trailer into every generated commit message.
- Out of scope: changing manual shell `git commit` behavior or required hook installation.
- Out of scope: changing the automated profile in `config/pkl/base/shared-content-automated-commit.pkl`.
- Generated artifacts must be regenerated from canonical Pkl sources rather than edited independently.

## Assumptions

1. `sce` is available when the SCE `/commit` command runs; this is already required by the installed SCE Git hook path.
2. Running the policy once as a command preflight and once through Git is safe because `apply_commit_msg_coauthor_policy` is idempotent and dedupes the canonical trailer.
3. The existing hook policy remains authoritative: the bypass command guarantees policy execution, not unconditional attribution.

## Task stack

- [ ] T01: `Add explicit attribution preflight to manual bypass commits` (status:todo)
  - Task ID: T01
  - Goal: Make `/commit skip` and `/commit oneshot` explicitly process their generated commit message through `sce hooks commit-msg` before executing a normal Git commit.
  - Boundaries (in/out of scope): In — update the bypass branch of the manual commit command in `config/pkl/base/shared-content-commit.pkl`; use a safely quoted temporary file outside the repository, preserve multiline message content, arrange cleanup on success/failure, run `sce hooks commit-msg <message-file>`, then run hook-enabled `git commit -F <message-file>`; regenerate all generator-owned manual command outputs. Out — regular `/commit`, the manual skill's message-writing rules, automated-profile content, Rust hook/config behavior, unconditional trailer insertion, `--no-verify`, and attribution-suppression environment overrides.
  - Done when: Both bypass aliases follow the same temporary-file preflight flow; preflight failure creates no commit and reports the failure; commit failure is reported without retry/amend/fallback; successful execution reports the commit hash; generated OpenCode, Claude, and Pi manual command surfaces contain the preflight contract; no generated surface instructs bypass mode to use the old direct `git commit -m "<message>"` path.
  - Verification notes (commands or checks): Regenerate with `nix develop -c pkl eval -m . config/pkl/generate.pkl`; inspect generated manual command files for `sce hooks commit-msg`, `git commit -F`, cleanup guarantees, and absence of `--no-verify`; run `nix run .#pkl-check-generated`.

- [ ] T02: `Document bypass attribution parity` (status:todo)
  - Task ID: T02
  - Goal: Update durable SCE workflow context to describe the explicit policy-preflight behavior and its relationship to the existing commit-msg hook gates.
  - Boundaries (in/out of scope): In — update `context/sce/atomic-commit-workflow.md` and the relevant `context/context-map.md` description; cross-check `context/sce/agent-trace-commit-msg-coauthor-policy.md` and update it only if needed to state that command preflight is an additional caller of the unchanged canonical policy. Out — historical plan summaries, unrelated commit workflow rules, and changes to the policy's enablement/evidence semantics.
  - Done when: Context states that manual bypass commits explicitly invoke the canonical policy before a normal hook-enabled commit; it explains idempotent double invocation and makes clear that opt-out and overlap gates still decide whether the trailer appears; context-map navigation remains current.
  - Verification notes (commands or checks): Read the updated workflow and policy documents against generated command truth; verify no context text claims that bypass mode unconditionally adds attribution or skips Git hooks.

- [ ] T03: `Validate attribution flow and clean up` (status:todo)
  - Task ID: T03
  - Goal: Validate generated parity, repository checks, bypass-command invariants, and absence of temporary or unintended artifacts.
  - Boundaries (in/out of scope): In — generated parity, full flake checks, targeted inspection or disposable-repository smoke coverage of enabled+overlap, disabled/no-overlap, failure-before-commit, exactly-once trailer, multiline message, and temporary-file cleanup cases where practical; review intended diff and context accuracy. Out — unrelated fixes discovered during validation.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; all manual generated command targets express the same preflight flow; the automated command is unchanged; evidence confirms the policy remains conditional and duplicate-safe; no temporary files, debug scaffolding, or unrelated generated drift remains; any impractical interactive `/commit skip` smoke step is explicitly recorded for human verification rather than reported as executed.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect the final diff/status; compare manual generated command targets; verify the final test commit message with `git log -1 --format=%B` in a disposable repository if an end-to-end command run is available.

## Open questions

_None._
