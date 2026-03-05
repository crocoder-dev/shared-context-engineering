# SCE setup git-hooks CLI UX

## Scope

Task `sce-setup-githooks-any-repo` `T04` defines the `sce setup` command-surface behavior for required-hook setup and combined target+hooks runs.

## Command surface

- `sce setup --hooks`
- `sce setup --hooks --repo <path>`
- `sce setup --opencode --hooks`
- `sce setup --claude --hooks`
- `sce setup --both --hooks`
- `sce setup` (interactive target selection plus hook install in one run)

`--hooks` runs required-hook installation (`pre-commit`, `commit-msg`, `post-commit`) through the setup service hook installer.
When `--repo` is omitted, setup targets the current working directory.

## Option compatibility and validation

Validation is deterministic and enforced during setup option resolution:

- `--repo` requires `--hooks`
- `--hooks` can be combined with exactly one target flag to run config install and required-hook install in one invocation
- `--repo` may only be provided once and must include a value
- `--repo` path is canonicalized and must resolve to an existing directory before hook setup runs

Target-install mode contract:

- `sce setup` defaults to interactive target selection
- default interactive `sce setup` installs selected config assets and required hooks in one run
- `--opencode`, `--claude`, and `--both` remain mutually exclusive for non-interactive target install
- `--non-interactive` is an explicit fail-fast control that disables prompting and requires one target flag (`--opencode`, `--claude`, or `--both`)
- legacy one-purpose invocations remain valid (`sce setup --hooks` for hooks-only, and `sce setup --opencode|--claude|--both` for config-only)
- interactive setup without a TTY returns actionable guidance to rerun with `--non-interactive` plus a target flag

## Output contract

Successful hook setup emits deterministic human/automation-friendly output including:

- repository root
- effective hooks directory
- per-hook outcome lines with canonical lowercase statuses (`installed`, `updated`, `skipped`)
- backup status per hook (`backup: '<path>'` or `backup: not needed`)

When config install and hook install run together, CLI output is deterministic: config-install summary first, one blank separator line, then hook-install summary.

## Implementation anchors

- `cli/src/app.rs`
- `cli/src/services/setup.rs`
- `cli/src/command_surface.rs`
