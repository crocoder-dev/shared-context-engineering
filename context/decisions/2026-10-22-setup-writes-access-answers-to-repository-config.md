# Decision: Setup writes access-policy answers to the repository config

Date: 2026-10-22
Status: Accepted
Plan: `context/plans/setup-auth-mode-repo-local.md`
Task: `T03`
Supersedes: `context/decisions/2026-10-21-setup-writes-access-answers-to-user-layer-only.md`

## Context

Under the [2026-10-21 decision](2026-10-21-setup-writes-access-answers-to-user-layer-only.md), interactive `sce setup` saved `auth.mode` and `telemetry.enabled` to the global user config. As a result, a `mandatory` mode or a telemetry consent chosen in one repository silently applied to every repository on the machine. The user asked for each answer to apply only to the repository being set up. The [repository-consent decision](2026-10-22-repository-telemetry-enabled-grants-consent.md) made a repository `telemetry.enabled: true` able to grant consent, so a repository-saved answer now takes effect.

## Decision

Interactive `sce setup` saves the `auth.mode` and `telemetry.enabled` answers to the repository's committed `<repository root>/.sce/config.json`, not to the global user config.

- Saved answers, prompt suppression, and the `--reconfigure` defaults come from the repository file. When the repository file is invalid, setup skips the access prompts and writes nothing.
- Repository values are setup's saved answers, not locks. Only a global or explicit (`--config` / `SCE_CONFIG_FILE`) `auth.mode: disabled` skips the access prompts. Only a global or explicit `mandatory` suppresses the login offer.
- The repository writer (`persist_repository_access_preferences`) keeps the guardrails of the previous global writer:
  - It writes explicit answers only and preserves unrelated keys.
  - It validates the result with `parse_file_config` and skips no-op writes.
  - It writes atomically (temp file plus rename, keeping the file mode).
  - It refuses a concurrently modified, symlinked, or invalid file.
  - It bootstraps a missing file from the repo-local payload.
- The write runs before the target install, so a failed write aborts setup before any other repository file changes.
- Setup no longer writes access keys to the global config, and the global access writer is removed. Existing global values are not migrated.
- The Claude-native telemetry removal step still acts only on a withdrawal in the global config.

## Rationale

Saving the answers in the repository config keeps each one scoped to the repository it was given for. Treating repository values as saved answers rather than locks follows from setup itself writing them: setup cannot tell a team-authored value from one it wrote earlier. Global and explicit values stay user-side restrictions that setup does not override.

## Alternatives considered

- **Keep writing to the global user config** — rejected: answers would keep leaking across repositories on the same machine.
- **A gitignored per-user repository file (for example `.sce/config.local.json`)** — not selected: the user chose the committed `.sce/config.json`, although this alternative would have kept per-person consent.

## Compatibility and risks

- A committed `telemetry.enabled: true` opts in everyone who clones or pulls the repository, without asking them. A global `telemetry.enabled: false` still opts a user out.
- Releases `v0.4.0-pre-alpha-v11` and earlier reject the `auth` and `telemetry` keys. They skip the whole repository config as invalid, including bash policies and integration targets.
- A team-committed `auth.mode: disabled` no longer locks the setup prompts, so `sce setup --reconfigure` can change it.
- Values left in the global config by earlier setup runs stay in effect: a global `mandatory` still wins, and a global `false` still opts out.
- A repository `telemetry.enabled: false` answer does not trigger the Claude-native telemetry removal. Removal still follows only a global withdrawal.

## Guardrails

- The resolver's `auth.mode` precedence, where the most restrictive value wins, is unchanged.
- Setup never writes an invalid, concurrently modified, or symlinked repository config.
- Non-interactive, hooks-only, and context-only setup never write access keys.

## Consequences

- One repository's access answers no longer affect other repositories.
- Saved access answers become shared team configuration once the file is committed.

## Follow-up

- Durable context for setup and the access policy must describe repository-scoped answers.

## References

- Plan: [`setup-auth-mode-repo-local`](../plans/setup-auth-mode-repo-local.md)
- Task: `T03`
- Current-state context: [`setup-repo-local-config-bootstrap`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`access-policy`](../cli/access-policy.md)
- Evidence: [`access_preferences.rs`](../../cli/src/services/setup/access_preferences.rs)
- Evidence: [`command.rs`](../../cli/src/services/setup/command.rs)
- Evidence: [`mod.rs`](../../cli/src/services/setup/mod.rs)
- Evidence: [`prompt.rs`](../../cli/src/services/setup/prompt.rs)
- Related decision: [`Setup writes access-policy answers to the user layer only`](2026-10-21-setup-writes-access-answers-to-user-layer-only.md)
- Related decision: [`A repository telemetry.enabled true grants telemetry consent`](2026-10-22-repository-telemetry-enabled-grants-consent.md)
