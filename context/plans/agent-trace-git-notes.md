# Plan: Persist Agent Trace JSON to git notes on post-commit

## Change summary

Extend the existing `sce hooks post-commit` Agent Trace flow so every successfully built and schema-validated Agent Trace payload is also written to a git note on the just-created commit. The note content is the full Agent Trace JSON already persisted in `agent_traces.trace_json`. Git-note persistence is best-effort: failures are logged for diagnostics but must not block the git commit or make the post-commit hook command fail when Agent Trace DB persistence succeeded.

The default notes ref is dedicated to SCE Agent Trace data and is configurable.

## Decisions

- Default git-notes ref: `refs/notes/sce-agent-trace`.
- Config surface: add a repo config field for the notes ref (for example `policies.agent_trace.git_notes_ref`, final naming to follow existing config style during implementation) with default `refs/notes/sce-agent-trace`.
- Note content: full Agent Trace JSON string after schema validation, matching the payload persisted to `agent_traces.trace_json`.
- Write posture: best-effort/non-blocking. Git-note write failures are logged and surfaced only as diagnostics, not as post-commit hook failures.
- Write mode: use replace/upsert semantics for the commit note so rerunning the hook for the same commit updates the SCE Agent Trace note instead of failing on an existing note.

## Success criteria

- On a successful `sce hooks post-commit --vcs git --remote-url <url>` run that builds and validates an Agent Trace payload, the current commit has a git note under `refs/notes/sce-agent-trace` by default.
- The note content is the full Agent Trace JSON and can be read back with `git notes --ref refs/notes/sce-agent-trace show <commit>`.
- The git-notes ref is configurable through SCE config and generated schema/docs reflect the default.
- If writing the git note fails, the post-commit hook remains successful when existing Agent Trace DB persistence succeeded; the failure is logged with a stable event name.
- Existing Agent Trace DB insertion remains unchanged and continues to be the source of persisted trace rows.
- Tests cover successful note write orchestration, configured ref use, existing-note replacement/upsert behavior, and non-blocking failure handling.
- Context documents describe the new post-commit git-notes behavior and the no-blocking-error posture.

## Constraints and non-goals

- Constraints:
  - Keep `post-commit` as the integration point because the commit SHA and Agent Trace JSON are available there.
  - Do not write a git note unless Agent Trace JSON validation has passed.
  - Preserve stdout/stderr contracts as much as possible; diagnostics belong in logging/stderr, not new stdout payloads.
  - Reuse existing hook config resolution and git command helper patterns instead of introducing a new dependency.
  - Keep note writes scoped to git; no behavior is required for non-git VCS values.
  - Keep failures non-blocking only for the git-note write step. Existing validation/DB insertion failures keep their current behavior.
- Non-goals:
  - Backfilling git notes for historical commits.
  - Pushing/fetching notes to/from remotes.
  - Changing Agent Trace JSON schema or DB schema.
  - Replacing Agent Trace DB persistence with git notes.
  - Adding a retry queue for failed note writes.

## Assumptions

- The dedicated default ref should be exactly `refs/notes/sce-agent-trace`.
- The implementation may write notes by invoking the local `git` binary through existing command helpers.
- Configurability means changing the notes ref, not disabling the feature. Disabling can be added later if a separate product decision requests it.

## Task stack

- [x] T01: `Add config surface for Agent Trace git-notes ref` (status:done)
  - Task ID: T01
  - Goal: Add a typed SCE config value for the Agent Trace git-notes ref with default `refs/notes/sce-agent-trace`.
  - Boundaries (in/out of scope): In - config type/resolver updates, env/config precedence only if this config area already has an established pattern, unit tests for default and explicit configured ref. Out - git-note writing, hook runtime wiring, generated schema/Pkl output.
  - Done when: runtime config exposes the resolved notes ref; default resolution returns `refs/notes/sce-agent-trace`; explicit config overrides the default; invalid empty/blank refs are rejected or normalized consistently with existing config validation patterns.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo fmt'`; targeted config tests if permitted by policy; otherwise `nix flake check`.
  - Status: done
  - Completed: 2026-07-14
  - Files changed: `cli/src/services/config/types.rs`, `cli/src/services/config/schema.rs`, `cli/src/services/config/resolver.rs`, `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` passed; targeted `cargo test` was blocked by SCE bash policy preferring `nix flake check`; `nix flake check` passed; `nix run .#pkl-check-generated` passed ("Generated outputs are up to date.").
  - Notes: User approved Option A scope expansion to include the minimal Pkl/generated schema update required for explicit config-file override validation. Git-note writing and post-commit hook wiring remain out of scope for T01.

- [ ] T02: `Sync Pkl schema and generated config docs for git-notes ref` (status:todo)
  - Task ID: T02
  - Goal: Update canonical Pkl config schema and regenerate generated JSON/config artifacts so the Agent Trace git-notes ref is documented and parity checks pass.
  - Boundaries (in/out of scope): In - Pkl source, generated JSON schema/config outputs, default/description text for the new ref. Out - Rust resolver logic from T01, hook runtime behavior from later tasks.
  - Done when: generated outputs include the new config field/default; `nix run .#pkl-check-generated` passes; no unrelated generated drift is present.
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`.

- [ ] T03: `Introduce git-notes writer helper for Agent Trace JSON` (status:todo)
  - Task ID: T03
  - Goal: Add a small, testable helper that writes full Agent Trace JSON to a git note for a commit/ref using replace/upsert semantics.
  - Boundaries (in/out of scope): In - helper surface in the hooks or git utility layer, command construction for `git notes --ref <ref> add -f -F <tempfile-or-stdin> <commit>`, validation that commit/ref/content inputs are non-empty, unit tests with injected command runner covering success, configured ref, existing-note replacement flag, and command failure. Out - calling the helper from post-commit runtime, changing Agent Trace build/validation, adding a DB migration.
  - Done when: helper is deterministic, avoids shell interpolation, handles multiline JSON safely, returns structured success/error for caller-side logging, and has focused tests.
  - Verification notes (commands or checks): targeted hooks/git-helper tests if permitted; `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix flake check` as fallback.

- [ ] T04: `Wire git-note persistence into post-commit Agent Trace flow` (status:todo)
  - Task ID: T04
  - Goal: After Agent Trace JSON validation and DB insertion succeed, write the same full JSON to the configured git-notes ref for the committed SHA, while keeping note-write failures non-blocking.
  - Boundaries (in/out of scope): In - post-commit flow wiring, resolved config read, stable log event for note-write failure (for example `sce.hooks.post_commit.agent_trace_git_note_write_failed`), tests proving successful write is attempted after DB insert and failures do not change hook success. Out - backfill, notes push/fetch, non-git VCS note behavior, changing existing DB failure semantics.
  - Done when: default post-commit writes a note under `refs/notes/sce-agent-trace`; configured ref is honored; note write is skipped or treated as no-op for unsupported/non-git contexts if necessary; note write failure logs diagnostics but does not fail the hook after DB persistence succeeds.
  - Verification notes (commands or checks): targeted post-commit hook tests if permitted; manual local check with `git notes --ref refs/notes/sce-agent-trace show HEAD`; `nix flake check`.

- [ ] T05: `Update Agent Trace context for git-notes persistence` (status:todo)
  - Task ID: T05
  - Goal: Document the new git-notes persistence contract in current-state context.
  - Boundaries (in/out of scope): In - update `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/sce/setup-githooks-hook-asset-packaging.md` if hook behavior text needs adjustment, and `context/context-map.md` entries. Out - implementation code, broad docs rewrites unrelated to post-commit Agent Trace persistence.
  - Done when: context states the default notes ref, config override, full-JSON note content, and non-blocking failure behavior; stale `No git-notes persistence` text is removed or qualified.
  - Verification notes (commands or checks): `rg "git-notes|git notes|No git-notes" context/`; manual diff review.

- [ ] T06: `Validate git-notes Agent Trace behavior and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run final validation for the complete plan and clean up any planning or test scaffolding.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, focused grep for stale docs/config strings, cleanup of temporary test repositories or notes refs created during manual checks. Out - new behavior beyond the completed task stack.
  - Done when: `nix flake check` passes or any failure is documented as pre-existing/unrelated; `nix run .#pkl-check-generated` passes; context sync is verified; no temporary scaffolding remains.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `git diff --check`; `rg "refs/notes/sce-agent-trace|agent_trace.*git.*note|No git-notes" cli/ config/ context/`.

## Open questions

None. Plan is ready for T01 execution.
