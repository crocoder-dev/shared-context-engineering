# SCE setup git-hooks CLI UX

## Scope

Task `sce-setup-githooks-any-repo` `T04` defines the `sce setup` command-surface behavior for required-hook setup mode.

## Command surface

- `sce setup --hooks`
- `sce setup --hooks --repo <path>`

`--hooks` runs required-hook installation (`pre-commit`, `commit-msg`, `post-commit`) through the setup service hook installer.
When `--repo` is omitted, setup targets the current working directory.

## Option compatibility and validation

Validation is deterministic and enforced during setup option resolution:

- `--repo` requires `--hooks`
- `--hooks` cannot be combined with `--opencode`, `--claude`, or `--both`
- `--repo` may only be provided once and must include a value
- `--repo` path is canonicalized and must resolve to an existing directory before hook setup runs

Target-install mode remains unchanged:

- `sce setup` defaults to interactive target selection
- `--opencode`, `--claude`, and `--both` remain mutually exclusive for non-interactive target install

## Output contract

Successful hook setup emits deterministic human/automation-friendly output including:

- repository root
- effective hooks directory
- per-hook outcome lines with canonical lowercase statuses (`installed`, `updated`, `skipped`)
- backup status per hook (`backup: '<path>'` or `backup: not needed`)

## Implementation anchors

- `cli/src/app.rs`
- `cli/src/services/setup.rs`
- `cli/src/command_surface.rs`
