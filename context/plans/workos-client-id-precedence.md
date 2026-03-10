# Plan: Auth Config Source Precedence

## Change Summary

Generalize the current WorkOS client ID precedence plan into a reusable auth-configuration precedence plan that works for any auth runtime value sourced from environment variables, config files, or baked defaults / hardcoded constants. Current code truth is mixed: core runtime config already applies deterministic `flags > env > config file > defaults`, while `cli/src/services/auth_command.rs` still performs direct env-only lookup for WorkOS client ID. This plan aligns auth-related runtime values behind one shared precedence resolver pattern so individual auth settings do not each reimplement bespoke env/config/hardcoded logic.

## Success Criteria

- [ ] Auth runtime values that can come from env, config, or baked defaults resolve through one deterministic precedence contract instead of per-call-site custom lookup
- [ ] `WORKOS_CLIENT_ID` keeps explicit override precedence over config-file and baked-default values
- [ ] The implementation approach is reusable for additional auth-adjacent keys without rewriting merge/source-tracking behavior per key
- [ ] Config-file resolution for supported auth keys uses existing discovery and merge rules: global config first, local config second, with local overriding global per key
- [ ] `sce config show` and `sce config validate` expose resolved source metadata for supported auth keys without dumping full values in normal text output when they look sensitive or credential-like
- [ ] Missing or invalid auth configuration diagnostics describe the full precedence chain and only fail when all allowed layers are absent or invalid for the specific key
- [ ] Unit tests cover reusable precedence behavior for env, global config, local config, baked default, and fully absent/invalid paths

## Constraints and Non-Goals

**In Scope:**
- Define a reusable precedence pattern for auth/runtime settings that may be supplied by env vars, config files, or baked defaults
- Extend runtime config schema and source tracking for supported auth-related keys starting with `workos_client_id`
- Reuse existing global+local config discovery and merge behavior
- Centralize auth-value resolution so auth command code consumes shared resolution instead of direct ad hoc lookup
- Surface resolved source and precedence-aware diagnostics in config/auth inspection paths
- Update focused context files to describe the generalized contract rather than a one-off client-ID rule

**Out of Scope:**
- Changing non-auth config precedence behavior outside the affected shared resolver surface
- Introducing CLI flags for new auth override values in this plan unless a key already has one
- Changing WorkOS API base URL handling unless it is explicitly folded into the generalized auth precedence surface in a later follow-up
- Adding interactive setup for auth configuration
- Rotating or remotely fetching baked defaults
- Broad auth UX redesign beyond value resolution, inspection, and diagnostics

**Non-Goals:**
- Multi-tenant WorkOS app selection
- Secret storage or encryption changes
- Sync-command auth guard changes
- Converting every existing config key in the CLI to a new abstraction if the key is unrelated to auth/runtime configuration
- Publishing production secrets; only intentionally public baked identifiers remain eligible for hardcoded defaults

## Assumptions

- `workos_client_id` is the first concrete auth key to migrate, but the resulting resolver shape should support additional auth-related keys without changing the precedence contract
- Existing config discovery remains canonical: explicit config path/env path override is separate from default discovery, and default discovered config layers merge as `global -> local`
- Not every auth key will necessarily allow a baked default; the shared resolver must support keys whose allowed sources are a subset of `env / config / baked default`
- Baked values used by this contract are approved for shipping in the CLI binary and are not secrets
- Auth commands should consume one shared auth-config resolver surface rather than duplicating precedence logic across service modules

## Task Stack

- [x] T01: Add reusable auth config key support to runtime config resolution (status:done)
  - Task ID: T01
  - Goal: Extend `cli/src/services/config.rs` so auth-related config keys can be declared once with deterministic env/config resolution and source tracking, starting with `workos_client_id`.
  - Boundaries (in/out of scope):
    - IN: Add `workos_client_id` to allowed config keys and parsed file schema
    - IN: Introduce or refactor key metadata so auth-related keys can declare env name, config key, and source reporting in one place
    - IN: Preserve existing discovered config merge order `global -> local`
    - IN: Keep deterministic output/source metadata compatible with `sce config show` / `validate`
    - OUT: Auth command wiring, baked default fallback behavior, context updates
  - Done when:
    - Runtime config can resolve supported auth keys from env or discovered/explicit config inputs through shared key metadata instead of one-off parsing branches
    - Local config overrides global config for `workos_client_id`
    - Unknown-key validation and deterministic output include the new auth key correctly
    - Unit tests cover env, global-only, local-over-global, and default-absent cases for the reusable auth-key path
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --lib config`
    - Verify `sce config show --format json` reports supported auth-key source metadata deterministically

- [x] T02: Introduce shared auth value precedence resolver with optional baked defaults (status:done)
  - Task ID: T02
  - Goal: Add one canonical auth-config resolver that applies an allowed-source chain per key (env > config file > baked default where permitted) and reuse it for `workos_client_id`.
  - Boundaries (in/out of scope):
    - IN: Add resolver abstractions/types that support keys with or without baked defaults
    - IN: Add one canonical baked default constant for `workos_client_id`
    - IN: Ensure the resolver returns resolved value plus source metadata for downstream diagnostics/output
    - IN: Keep precedence deterministic and key-declarative instead of hardcoding each lookup path in callers
    - OUT: Auth command integration, broader CLI flag additions, non-auth config rewrites
  - Done when:
    - A shared resolver can answer supported auth-key lookups without duplicating env/config/hardcoded precedence logic per call site
    - `workos_client_id` resolves with precedence `env > config file > baked default`
    - The resolver can represent keys that intentionally omit baked defaults while still participating in shared diagnostics/source reporting
    - Focused tests cover allowed-source combinations and invalid/absent outcomes coherently
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --lib config`
    - Add focused tests for resolution order and per-key allowed-source combinations

- [x] T03: Wire auth command flows to shared auth config resolution (status:done)
  - Task ID: T03
  - Goal: Replace direct env-only auth lookup in `cli/src/services/auth_command.rs` with the shared auth-config resolver so runtime auth flows use the generalized precedence contract.
  - Boundaries (in/out of scope):
    - IN: Replace direct `WORKOS_CLIENT_ID` lookup in auth command execution
    - IN: Reuse config service resolution rather than reimplementing global/local lookup in auth code
    - IN: Keep login/refresh/status runtime behavior unchanged apart from how supported auth values are obtained
    - OUT: New auth subcommands, token-storage changes, WorkOS base URL redesign
  - Done when:
    - `sce auth login` resolves `workos_client_id` via the shared resolver instead of env-only logic
    - Env values still win over config-file and baked-default values
    - Config-file values win over baked defaults when env is absent
    - Existing auth runtime paths still fail clearly when a required auth value is invalid or disallowed across all permitted layers
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth_command`
    - Add focused tests for auth-command wiring across env, local config, global config, baked default, and invalid/absent outcomes

- [x] T04: Expose generalized precedence-aware config output and diagnostics (status:done)
  - Task ID: T04
  - Goal: Make config inspection and auth failure guidance describe the shared env/config/baked precedence contract for supported auth keys.
  - Boundaries (in/out of scope):
    - IN: Update `sce config show` / `sce config validate` output contracts for supported auth keys
    - IN: Redact or abbreviate text-mode display when values appear sensitive or credential-like
    - IN: Update auth error text so guidance reflects key-specific allowed precedence layers rather than env-only assumptions
    - OUT: Broader secret-redaction redesign or full config-command UX overhaul
  - Done when:
    - Config output documents resolved auth-key values and source consistently in text and JSON modes
    - Auth diagnostics no longer imply env-only configuration and can describe when baked defaults are or are not part of the chain for a given key
    - Contract-focused tests cover precedence-aware messaging and output shape
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth`
    - Run `cargo test --manifest-path cli/Cargo.toml --lib config`
    - Verify `sce config show` text output distinguishes env/config/default sourcing for supported auth keys

- [x] T05: Sync focused context contracts for generalized auth config precedence (status:done)
  - Task ID: T05
  - Goal: Update current-state context files so future sessions reflect the reusable auth precedence contract instead of a one-off client-ID rule.
  - Boundaries (in/out of scope):
    - IN: Update `context/cli/config-precedence-contract.md`
    - IN: Update `context/cli/placeholder-foundation.md`
    - IN: Update `context/overview.md` and `context/glossary.md` only if the generalized auth precedence contract is important at root-context level
    - OUT: Historical narrative or completed-work summaries
  - Done when:
    - Context files describe shared auth-key precedence behavior, including `WORKOS_CLIENT_ID` as the first concrete migrated key
    - Root context edits are limited to true cross-cutting contract changes
    - No stale env-only wording remains in focused auth/config context for migrated keys
  - Verification notes (commands or checks):
    - Verify context statements match implemented precedence exactly
    - Confirm focused context distinguishes generic resolver contract from key-specific allowances such as baked defaults

- [ ] T06: Validation and cleanup (status:todo)
  - Task ID: T06
  - Goal: Validate code, tests, and context alignment for the generalized auth config precedence behavior.
  - Boundaries (in/out of scope):
    - IN: Run focused cargo tests for config/auth/auth_command
    - IN: Run repo-required lightweight validation baseline
    - IN: Verify context sync accuracy after implementation
    - OUT: Manual live WorkOS login against production unless approved credentials/environment are available
  - Done when:
    - Automated checks for touched areas pass
    - Shared precedence behavior is covered by tests for env, local config, global config, baked default, and invalid/absent paths where applicable
    - Context reflects current code truth with no known drift for this feature
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --lib config`
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth`
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth_command`
    - Run `nix run .#pkl-check-generated`
    - Run `nix flake check`

## Open Questions

None.
