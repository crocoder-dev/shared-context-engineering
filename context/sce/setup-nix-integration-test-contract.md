# SCE setup Nix integration-test contract

## Scope

Plan `sce-cli-setup-integration-test-improvements` task `T01` defines the canonical scenario matrix and assertion policy for setup integration testing executed through the deterministic Nix entrypoint.

This contract is intentionally current-state oriented and implementation-facing. It defines what `cli/tests/setup_integration.rs` must cover and how assertions stay deterministic across platforms.

## Required execution model

- Integration tests run against the compiled `sce` binary path (not `cargo run`).
- Scenarios execute in isolated ephemeral repositories with deterministic fixture setup.
- Assertions use filesystem and git-resolved state as source of truth for setup outcomes.
- Exit-code and stderr assertions are required for invocation validation/runtime-failure contracts.
- Test runtime isolates local state roots per test temp directory (no shared user-global state).

## Canonical scenario matrix (IDs and assertion anchors)

### Baseline target-install coverage

- `SETUP-TARGET-OPENCODE-RUN1`: `sce setup --opencode` installs `.opencode/` assets in a fresh repo.
- `SETUP-TARGET-OPENCODE-RERUN`: rerun and assert deterministic rerun behavior.
- `SETUP-TARGET-CLAUDE-RUN1`: `sce setup --claude` installs `.claude/` assets in a fresh repo.
- `SETUP-TARGET-CLAUDE-RERUN`: rerun and assert deterministic rerun behavior.
- `SETUP-TARGET-BOTH-RUN1`: `sce setup --both` installs both target trees.
- `SETUP-TARGET-BOTH-RERUN`: rerun and assert deterministic rerun behavior.

Deterministic assertion anchors:

- expected target tree presence/absence per mode
- stable setup summary markers
- deterministic rerun status semantics

### Baseline hook-install coverage

- `SETUP-HOOKS-DEFAULT-RUN1`: `sce setup --hooks` installs required hooks in default `.git/hooks` mode.
- `SETUP-HOOKS-DEFAULT-RERUN`: rerun and assert deterministic per-hook `installed|updated|skipped` semantics resolve to stable outcomes.
- `SETUP-HOOKS-CUSTOM-RUN1`: configure per-repo `core.hooksPath`, run `sce setup --hooks`, and assert required hooks install in resolved custom path.
- `SETUP-HOOKS-CUSTOM-RERUN`: rerun custom-path hook setup and assert deterministic idempotency.

Deterministic assertion anchors:

- printed repository root and effective hooks directory
- per-hook outcome lines with canonical lowercase statuses
- executable-state assertions on platforms that support executable-bit checks

### `--repo` canonicalization path scenarios

- `SETUP-HOOKS-REPO-RELATIVE`: invoke `sce setup --hooks --repo <relative-path>` and assert canonical repo-root/hook-dir output plus filesystem truth.
- `SETUP-HOOKS-REPO-ABSOLUTE`: invoke `sce setup --hooks --repo <absolute-path>` and assert canonical repo-root/hook-dir output plus filesystem truth.

Deterministic assertion anchors:

- output repository root equals canonicalized git-resolved repo path
- output hooks directory equals effective git-resolved hooks target
- required hook files exist in resolved hooks directory

### Setup failure-contract scenarios

- `SETUP-FAIL-REPO-MISSING`: `sce setup --hooks --repo /missing` asserts non-zero exit class and deterministic stderr guidance.
- `SETUP-FAIL-REPO-WITHOUT-HOOKS`: `sce setup --repo <path>` asserts deterministic validation failure contract (`--repo` requires `--hooks`).
- `SETUP-FAIL-NONINTERACTIVE-WITHOUT-TARGET`: `sce setup --non-interactive` asserts deterministic validation failure contract requiring one target flag.

Deterministic assertion anchors:

- process exit code class matches setup failure type
- stderr contains contract-stable guidance text

### True interactive PTY scenarios

- `SETUP-PTY-SELECT-OPENCODE`: PTY-backed `sce setup` flow selects OpenCode and asserts successful install outcomes.
- `SETUP-PTY-CANCEL`: PTY-backed `sce setup` flow cancels selection and asserts non-destructive cancellation outcome.
- `SETUP-NONTTY-INTERACTIVE-FAIL`: non-TTY interactive invocation asserts deterministic actionable guidance.

Deterministic assertion anchors:

- prompt visibility and selection/cancel control flow through PTY
- success/cancel terminal outcomes consistent with current setup UX contract
- non-TTY error guidance includes non-interactive+target remediation path

### Hook update + backup scenarios

- `SETUP-HOOKS-UPDATE-MUTATED`: mutate one installed required hook, rerun `sce setup --hooks`, assert `updated` status and backup creation.
- `SETUP-HOOKS-BACKUP-COLLISION`: pre-create backup suffix collisions (for example `.backup`, `.backup.1`), rerun setup, assert deterministic next-suffix selection.

Deterministic assertion anchors:

- mutated hook transitions to `updated`
- backup path emitted and backup artifact exists
- collision path selection matches next available deterministic suffix

### Permission and writability failure scenarios

- `SETUP-PERM-REPO-ROOT-NONWRITABLE`: repo-root write-probe failure path asserts deterministic setup failure.
- `SETUP-PERM-HOOKS-DIR-NONWRITABLE`: hooks-directory write-probe failure path asserts deterministic setup failure.
- `SETUP-PERM-UNIX-READONLY-GUARD`: unix-guarded read-only-directory scenario validates portable deterministic behavior under POSIX permissions.

Deterministic assertion anchors:

- failure class and stderr guidance match write-probe contract
- unix-only scenarios are `cfg(unix)`-guarded and stable

### Cross-platform validation scenarios

- `SETUP-PLATFORM-MULTIOS-MATRIX`: setup integration validation runs on at least two OS targets.
- `SETUP-PLATFORM-ASSERTION-GUARDS`: executable-bit/path assertions are platform-aware and deterministic for each OS.

Deterministic assertion anchors:

- CI evidence shows multi-OS setup integration execution
- per-OS assertions avoid false failures from platform-specific path/permission semantics

## Assertion signal policy

- Canonical signals:
  - repository filesystem state (installed paths, required files, backup artifacts)
  - git-resolved effective hooks directory for default and custom `core.hooksPath`
  - process exit-code class and contract-stable stderr guidance for failure paths
- Secondary signals:
  - deterministic CLI status lines for setup/hook outcomes
- Non-canonical signals:
  - non-contract free-form wording details outside stable guidance anchors
  - side effects outside the scenario temp root

## OS guard policy

- Permission cases that require POSIX read-only semantics must be `cfg(unix)`-guarded.
- Executable-bit assertions are required only where platform semantics support deterministic checks.
- Path assertions must normalize/canonicalize before equality checks when path style differs by OS.

## CI trigger constraint

- This plan does not change CI workflow trigger definitions (`on:` push/pull_request filters or branch filters).
- Multi-OS validation changes, when needed, are limited to job matrix/runtime configuration only.

## Verification entrypoints

- Scenario-focused local execution is anchored by targeted `cargo test --test setup_integration <slice>` invocations per task.
- Deterministic Nix integration entrypoint remains `nix run .#cli-integration-tests`.

## Parity anchors

This contract is kept in parity with:

- `context/sce/setup-githooks-cli-ux.md`
- `context/sce/cli-security-hardening-contract.md`
- `context/sce/setup-githooks-install-flow.md`
