# Plan: Cleanup Dead Code

## Change Summary

Remove dead code from the SCE CLI codebase as identified in the analysis. This plan targets specific files and code sections marked with `#[allow(dead_code)]` or `#![allow(dead_code)]` that are confirmed to be unused and safe to remove.

**Scope:**
- Remove entire `hosted_reconciliation.rs` module (1110 lines including tests)
- Remove unused styling helper functions from `style.rs`
- Remove exit code constants from `app.rs`
- **Preserve** token refresh infrastructure in `auth.rs`
- **Preserve** default paths infrastructure in `default_paths.rs`

## Success Criteria

1. All specified dead code is removed without breaking existing functionality
2. `nix flake check` passes after each atomic change
3. No compilation warnings introduced
4. Test suite passes (existing tests for kept functionality)
5. CLI behavior remains unchanged for all active commands

## Constraints and Non-Goals

**In Scope:**
- Physical removal of `hosted_reconciliation.rs` file
- Removal of unused styling helper functions (example_command, placeholder, status_implemented, status_placeholder, heading_stderr)
- Removal of exit code constants (EXIT_CODE_PARSE_FAILURE, EXIT_CODE_VALIDATION_FAILURE, EXIT_CODE_RUNTIME_FAILURE, EXIT_CODE_DEPENDENCY_FAILURE)
- Removal of dead code markers where code is being kept but marked for future use

**Out of Scope:**
- Token refresh infrastructure in `auth.rs` (explicitly kept per requirements)
- Default paths infrastructure in `default_paths.rs` (explicitly kept per requirements)
- Any code removal that would require refactoring active code paths
- Changes to test files for kept functionality

**Non-Goals:**
- General codebase refactoring beyond dead code removal
- Adding new functionality
- Changing CLI behavior or output

## Task Stack

- [ ] T01: Remove `hosted_reconciliation.rs` module (status:todo)
  - Task ID: T01
  - Goal: Completely remove the hosted_reconciliation.rs file and all its references
  - Boundaries (in/out of scope): In - delete cli/src/services/hosted_reconciliation.rs, remove module declaration from cli/src/services/mod.rs if present, remove any imports/references in other files. Out - changes to other modules, new functionality.
  - Done when: hosted_reconciliation.rs file is deleted, codebase compiles without errors, nix flake check passes, no references to hosted_reconciliation remain
  - Verification notes (commands or checks): `nix flake check`, `cargo check --manifest-path cli/Cargo.toml`, verify file no longer exists at cli/src/services/hosted_reconciliation.rs

- [ ] T02: Remove unused styling helpers from `style.rs` (status:todo)
  - Task ID: T02
  - Goal: Remove dead styling helper functions marked with #[allow(dead_code)]
  - Boundaries (in/out of scope): In - remove example_command(), placeholder(), status_implemented(), status_placeholder(), heading_stderr() functions and their #[allow(dead_code)] attributes. Out - changes to used styling functions (heading, command_name, error_code, error_text, success, label, value, prompt_label, prompt_value, clap_help).
  - Done when: All five unused styling helpers are removed, style.rs compiles without dead_code markers, nix flake check passes
  - Verification notes (commands or checks): `nix flake check`, `cargo check --manifest-path cli/Cargo.toml`, verify no dead_code markers remain in style.rs

- [ ] T03: Remove exit code constants from `app.rs` (status:todo)
  - Task ID: T03
  - Goal: Remove unused exit code constants marked with #[allow(dead_code)]
  - Boundaries (in/out of scope): In - remove EXIT_CODE_PARSE_FAILURE, EXIT_CODE_VALIDATION_FAILURE, EXIT_CODE_RUNTIME_FAILURE, EXIT_CODE_DEPENDENCY_FAILURE constants and their #[allow(dead_code)] attributes. Out - changes to ClassifiedError or exit code handling logic, modifications to tests.
  - Done when: All four exit code constants are removed, app.rs compiles without these dead_code markers, nix flake check passes
  - Verification notes (commands or checks): `nix flake check`, `cargo check --manifest-path cli/Cargo.toml`, verify exit code constants are not referenced in tests

- [ ] T04: Clean up remaining dead_code markers on kept code (status:todo)
  - Task ID: T04
  - Goal: Review and remove unnecessary #[allow(dead_code)] markers from code being kept but marked dead
  - Boundaries (in/out of scope): In - review and remove dead_code markers from auth.rs token refresh code (kept per requirements), other files where dead_code markers are no longer needed after T01-T03. Out - adding new dead_code markers, changing functionality.
  - Done when: All unnecessary dead_code markers are removed, compilation succeeds, nix flake check passes
  - Verification notes (commands or checks): `nix flake check`, `cargo check --manifest-path cli/Cargo.toml`, verify no spurious dead_code warnings

- [ ] T05: Validation and context sync (status:todo)
  - Task ID: T05
  - Goal: Final validation and documentation update
  - Boundaries (in/out of scope): In - run full test suite, verify no dead code remains per success criteria, update context if needed. Out - code changes.
  - Done when: All verification commands pass, dead code analysis from original report is confirmed resolved, context files updated if architecture changes warrant it
  - Verification notes (commands or checks): `nix flake check` (full), `cargo test --manifest-path cli/Cargo.toml`, `cargo clippy --manifest-path cli/Cargo.toml`, verify no #[allow(dead_code)] or #![allow(dead_code)] remain except on explicitly kept code

## Open Questions

None. User has explicitly specified:
- Keep token refresh infrastructure (auth.rs)
- Keep default paths infrastructure (default_paths.rs)
- Remove hosted_reconciliation.rs, unused styling helpers, and exit codes
