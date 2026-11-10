# Decision: Setup upgrades older-series config `$schema` declarations

Date: 2026-11-10
Status: Accepted
Plan: `context/plans/config-schema-series-compatibility.md`
Task: `T08`

## Context

The SCE config schema accepts `$schema` declarations within the CLI's `major.minor` release series ([series-acceptance decision](2026-11-10-config-schema-accepts-release-series-declarations.md)). A file that declares an older series is still rejected. In a default-discovered file, that rejection discards every setting in the file. Users need one supported way to move a stale declaration forward that does not hand-edit files or silently accept other series.

Setup previously never revisited an existing declaration. It treated the global config as read-only ([repository access answers decision](2026-10-22-setup-writes-access-answers-to-repository-config.md)).

## Decision

`sce setup` is the only writer that upgrades a config `$schema` declaration.

- **Acceptance stays by series.** Validation, `sce config validate`, `sce doctor` (including `--fix`), and command startup stay read-only. They report an actionable `$schema` message and never rewrite a file.
- **Canonical files only.** Setup considers only the existing repository `.sce/config.json` and the existing default global config. It never creates a missing file to upgrade it, and it never writes an explicit `--config` / `SCE_CONFIG_FILE` file at a non-canonical path.
- **`$schema` only.** For an older-series declaration, the candidate changes only the `$schema` value to the CLI's concrete URL. The global config write is limited to that value. This amends the read-only global-config statement in the repository access answers decision. Access answers still go only to the repository config.
- **Validate before saving.** The candidate is validated through `parse_file_config` with the file's own source. It is saved through the guarded reread-compare, symlink-refusing, atomic write.
- **No downgrade.** Same-series, newer-series, malformed, foreign, and absent declarations are never written.
- **No field migrations.** An older-series file whose other fields fail validation stays byte-identical.
- **Non-fatal reporting.** Setup reports each upgrade or skip in a `Config schema:` section and exits successfully. A still-invalid file keeps the existing degraded behavior.
- **Ordering.** Upgrades are planned after the Git-repo gate and committed only after the named-remote preflight passes. The remote check reads the validated candidates.

## Rationale

Setup already owns repository bootstrap writes, and the user runs it deliberately. Keeping validation and doctor read-only means no ordinary command mutates config as a side effect. Limiting the global write to `$schema` keeps user-owned settings untouched. Validating the full candidate keeps unrelated field changes from slipping in through a `$schema` bump.

## Alternatives considered

- **Upgrade from `sce doctor --fix` or `sce config`.** Not selected, because those paths stay read-only.
- **Leniently accept other series.** Not selected, because it would silently accept incompatible declarations.
- **A general field-migration framework.** Not selected, because it is out of scope for a declaration bump.
- **Fail setup when an upgrade is skipped.** Not selected, because a dotfile-managed (symlinked) global config should not block repository setup.

## Compatibility and risks

- Historical `schema/v*/config.json` snapshots are left unchanged. The next `nix run .#bump-version` publishes the first series-pattern snapshot.
- A symlinked config is refused and must be updated at its source.

## Guardrails

- The Git-repo gate and named-remote preflight run before every setup write.
- Context-only setup (`--bootstrap-context`) performs no config work.

## Consequences

- A series bump no longer requires hand-editing config files. Running `sce setup` upgrades both canonical files.
- Setup can now write the global config, but only its `$schema` value.

## References

- Plan: [`config-schema-series-compatibility`](../plans/config-schema-series-compatibility.md)
- Current-state context: [`setup-repo-local-config-bootstrap.md`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`config-schema-publication.md`](../sce/config-schema-publication.md)
- Current-state context: [`config-precedence-contract.md`](../cli/config-precedence-contract.md)
- Evidence: [`schema_upgrade.rs`](../../cli/src/services/setup/schema_upgrade.rs)
- Evidence: [`test-setup-schema-upgrade.sh`](../../scripts/test-setup-schema-upgrade.sh)
