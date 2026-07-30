# Optional Install-Channel Integration-Test Entrypoint

This file is retained as historical context for the removed optional install-channel integration-test runner.

## Current State

- The former standalone `integrations/install/` Rust runner is not an active current-state surface.
- The former `apps.install-channel-integration-tests` flake app is not exposed by the current root flake.
- Default `nix flake check` does not run the removed integration runner or its former `integrations-install-*` checks.
- Current install-channel validation is covered by the active release, package, and distribution checks documented in the install-channel and release contracts.

## Historical Scope

The removed runner previously provided opt-in install-channel integration coverage for npm, Bun, and Cargo behind:

```bash
nix run .#install-channel-integration-tests -- --channel <npm|bun|cargo|all>
```

That command is intentionally not documented as current usage. See `context/plans/remove-integrations-install.md` for the removal decision and validation evidence.

## Related Current Context

- `context/overview.md`
- `context/architecture.md`
- `context/sce/cli-first-install-channels-contract.md`
