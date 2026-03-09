# Plan: Migrate lexopt to clap

## Change summary

Replace the manual `lexopt`-based CLI parsing with `clap` derive macros across all CLI command parsers. Replace manual shell completion scripts with `clap_complete` for auto-generated completions. Preserve the existing error-code taxonomy, exit-code contract, and stdout/stderr stream contract.

## Success criteria

1. All CLI commands parse correctly via clap derive macros
2. Shell completions generated via `clap_complete` for bash/zsh/fish
3. Exit-code taxonomy preserved (2=parse, 3=validation, 4=runtime, 5=dependency)
4. Error-code taxonomy preserved (SCE-ERR-PARSE, SCE-ERR-VALIDATION, SCE-ERR-RUNTIME, SCE-ERR-DEPENDENCY)
5. Stdout/stderr stream contract preserved (stdout=payload, stderr=diagnostics)
6. "Try:" remediation guidance preserved in error messages
7. All existing tests pass
8. `nix flake check` passes

## Constraints and non-goals

### Constraints
- Must preserve backward-compatible command-line interface (same flags, same behavior)
- Must preserve exit codes and error-code taxonomy for scripting compatibility
- Must preserve stdout/stderr separation for pipe-safe command output

### Non-goals
- No command-line interface changes (no new features, no removed features)
- No changes to runtime behavior beyond argument parsing
- No changes to service-layer logic

## Task stack

- [x] T01: Add clap and clap_complete dependencies (status:done)
- [x] T02: Create clap-based CLI schema module (status:done)
- [x] T03: Migrate app.rs to use clap parser (status:done)
- [x] T04: Remove lexopt from service modules (status:done)
- [ ] T05: Replace completion.rs with clap_complete (status:todo)
- [ ] T06: Remove lexopt dependency (status:todo)
- [ ] T07: Update context documentation (status:todo)
- [ ] T08: Validation and cleanup (status:todo)

---

### T01: Add clap and clap_complete dependencies

**Task ID:** T01

**Goal:** Add clap (derive feature) and clap_complete to Cargo.toml dependencies and update dependency_contract.rs to reference clap instead of lexopt.

**Boundaries (in/out of scope):**
- In scope: Adding `clap` with `derive` feature, adding `clap_complete`, updating `dependency_contract.rs` to reference clap types, removing lexopt reference and test

**Done when:**
- `cli/Cargo.toml` includes `clap` with `derive` feature
- `cli/Cargo.toml` includes `clap_complete`
- `cli/src/dependency_contract.rs` references clap and clap_complete instead of lexopt
- Dependency contract test removed
- `cargo check --manifest-path cli/Cargo.toml` succeeds
- All tests pass

**Verification notes:**
```bash
cargo check --manifest-path cli/Cargo.toml
cargo tree --manifest-path cli/Cargo.toml | grep -E "clap|lexopt"
```

---

### T02: Create clap-based CLI schema module

**Task ID:** T02

**Goal:** Create a new module `cli/src/cli_schema.rs` that defines the complete clap-based CLI structure using derive macros.

**Boundaries (in/out of scope):**
- In scope:
  - Define `Cli` struct with `#[derive(Parser)]`
  - Define `Commands` enum with `#[derive(Subcommand)]` for all top-level commands
  - Define subcommand structs for config, setup, hooks, etc.
  - Map `--format` values to clap `ValueEnum`
  - Map shell values (bash/zsh/fish) to clap `ValueEnum`
- Out of scope:
  - Wiring the new parser into app.rs
  - Removing lexopt code
  - Changing runtime behavior

**Done when:**
- `cli/src/cli_schema.rs` exists with complete clap schema
- Schema covers all current commands: help, config, setup, doctor, mcp, hooks, sync, version, completion
- Schema covers all current options and subcommands
- `cargo check --manifest-path cli/Cargo.toml` succeeds
- Schema compiles without warnings

**Verification notes:**
```bash
cargo check --manifest-path cli/Cargo.toml
# Verify schema covers all commands by checking generated help
cargo test --manifest-path cli/Cargo.toml --no-run
```

---

### T03: Migrate app.rs to use clap parser

**Task ID:** T03

**Goal:** Replace lexopt-based parsing in app.rs with clap-based parsing from cli_schema.rs.

**Boundaries (in/out of scope):**
- In scope:
  - Replace `parse_command` with clap `Cli::parse_from` or `Cli::try_parse_from`
  - Map clap errors to `ClassifiedError` with appropriate class (Parse/Validation)
  - Preserve exit-code mapping for each error class
  - Preserve stdout/stderr stream contract
  - Preserve "Try:" remediation guidance in error formatting
  - Update dispatch to work with clap-derived types
- Out of scope:
  - Changes to service-layer runtime logic
  - Removing service-layer parsers (done in T04)

**Done when:**
- `cli/src/app.rs` uses clap for parsing
- All existing tests in `app::tests` pass
- Exit codes match expected values for parse/validation/runtime/dependency failures
- Error messages include appropriate SCE-ERR-* codes
- Stdout/stderr separation preserved

**Verification notes:**
```bash
cargo test --manifest-path cli/Cargo.toml app::tests
# Manual verification of exit codes
./target/debug/sce does-not-exist; echo $?  # Should be 2
./target/debug/sce setup --repo ../x; echo $?  # Should be 3
./target/debug/sce hooks commit-msg /missing; echo $?  # Should be 4
```

---

### T04: Remove lexopt from service modules

**Task ID:** T04

**Status:** done

**Completion notes:**
- Removed `parse_*` functions and `*_usage_text()` functions from:
  - `version.rs` (removed `parse_version_request`, `version_usage_text`)
  - `doctor.rs` (removed `parse_doctor_request`, `doctor_usage_text`)
  - `sync.rs` (removed `parse_sync_request`, `sync_usage_text`)
  - `mcp.rs` (removed `parse_mcp_request`, `mcp_usage_text`)
  - `config.rs` (removed `parse_config_subcommand`, `parse_config_request`, `config_usage_text`, `ConfigSubcommand::Help`)
  - `setup.rs` (removed `parse_setup_cli_options`, `setup_usage_text`)
  - `hooks.rs` (removed `parse_hooks_subcommand`, `hooks_usage_text`, `ensure_no_extra_hook_args`)
- Removed all `lexopt` imports from service modules
- Removed parse-related tests from service test modules
- All service tests pass (205 unit tests + 19 integration tests)
- `cargo check` succeeds with no errors

**Goal:** Remove lexopt-based parsing from all service modules since clap handles all parsing at the app layer.

**Boundaries (in/out of scope):**
- In scope:
  - Remove `parse_*` functions from service modules that are now handled by clap
  - Remove `lexopt` imports from service modules
  - Update service functions to accept clap-derived types instead of parsing themselves
  - Remove `*_usage_text()` functions (clap generates help automatically)
  - Keep service runtime logic intact
- Out of scope:
  - Changes to runtime behavior
  - Changes to completion.rs (done in T05)

**Done when:**
- No `lexopt` imports remain in service modules
- All service tests pass
- `cargo check --manifest-path cli/Cargo.toml` succeeds

**Verification notes:**
```bash
grep -r "lexopt" cli/src/services/ || echo "No lexopt references in services"
cargo test --manifest-path cli/Cargo.toml
```

---

### T05: Replace completion.rs with clap_complete

**Task ID:** T05

**Goal:** Replace manual shell completion scripts with clap_complete auto-generated completions.

**Boundaries (in/out of scope):**
- In scope:
  - Remove `bash_completion_script()`, `zsh_completion_script()`, `fish_completion_script()`
  - Remove `parse_completion_request()` and `CompletionRequest` struct
  - Use `clap_complete::generate` for shell completions
  - Wire completion generation through the clap schema
- Out of scope:
  - Custom completion script formatting (accept clap_complete defaults)

**Done when:**
- `cli/src/services/completion.rs` uses clap_complete
- No manual completion scripts in source
- `sce completion --shell bash` outputs valid bash completion
- `sce completion --shell zsh` outputs valid zsh completion
- `sce completion --shell fish` outputs valid fish completion
- All completion tests pass

**Verification notes:**
```bash
cargo test --manifest-path cli/Cargo.toml services::completion::tests
./target/debug/sce completion --shell bash | head -5
./target/debug/sce completion --shell zsh | head -5
./target/debug/sce completion --shell fish | head -5
```

---

### T06: Remove lexopt dependency

**Task ID:** T06

**Goal:** Remove lexopt from Cargo.toml now that all parsing uses clap.

**Boundaries (in/out of scope):**
- In scope:
  - Remove `lexopt = "0.3"` from `cli/Cargo.toml`
  - Remove `lexopt` from `cli/src/dependency_contract.rs` if referenced
  - Verify no remaining lexopt imports
- Out of scope:
  - Any other dependency changes

**Done when:**
- `lexopt` not in `cli/Cargo.toml`
- `grep -r "lexopt" cli/src/` returns no results
- `cargo build --manifest-path cli/Cargo.toml` succeeds
- All tests pass

**Verification notes:**
```bash
grep "lexopt" cli/Cargo.toml && echo "FAIL: lexopt still in Cargo.toml" || echo "OK: lexopt removed"
grep -r "use lexopt" cli/src/ && echo "FAIL: lexopt imports found" || echo "OK: no lexopt imports"
cargo test --manifest-path cli/Cargo.toml
```

---

### T07: Update context documentation

**Task ID:** T07

**Goal:** Update context files to reflect the clap migration and updated dependency contract.

**Boundaries (in/out of scope):**
- In scope:
  - Update `context/cli/placeholder-foundation.md` to reference clap instead of lexopt
  - Update `context/overview.md` dependency contract list
  - Update `context/glossary.md` command loop definition
  - Update `context/architecture.md` CLI section
  - Create decision record in `context/decisions/` documenting the migration rationale
- Out of scope:
  - Changes to unrelated context files
  - Changes to application code

**Done when:**
- All references to lexopt in context files updated to clap
- Decision record exists explaining the migration
- `nix run .#pkl-check-generated` passes

**Verification notes:**
```bash
grep -r "lexopt" context/ && echo "FAIL: lexopt references found" || echo "OK: no lexopt references"
nix run .#pkl-check-generated
```

---

### T08: Validation and cleanup

**Task ID:** T08

**Goal:** Final validation that the migration is complete and all contracts are preserved.

**Boundaries (in/out of scope):**
- In scope:
  - Run full test suite
  - Run `nix flake check`
  - Verify all exit codes are correct
  - Verify all error codes (SCE-ERR-*) are correct
  - Verify stdout/stderr separation
  - Verify completion scripts work
  - Clean up any dead code
- Out of scope:
  - New features or behavior changes

**Done when:**
- `cargo test --manifest-path cli/Cargo.toml` passes
- `nix flake check` passes
- Manual verification of exit codes for parse/validation/runtime/dependency failures
- Manual verification of error message format
- Manual verification of completion output

**Verification notes:**
```bash
cargo test --manifest-path cli/Cargo.toml
nix flake check

# Exit code verification
./target/debug/sce --bad-option 2>&1; echo "Exit code: $?"
./target/debug/sce setup --repo ../x 2>&1; echo "Exit code: $?"

# Completion verification
./target/debug/sce completion --shell bash | grep "_sce_complete"

# Error format verification
./target/debug/sce does-not-exist 2>&1 | grep "SCE-ERR-PARSE"
```

---

## Open questions

None - all clarifications resolved.

## Assumptions

1. clap's derive macros provide sufficient flexibility to preserve exact current CLI behavior
2. clap_complete generates acceptable completion scripts for bash/zsh/fish
3. The dependency contract relaxation for clap is acceptable given the maintainability benefits
4. Error message formatting can be customized via clap's error handling hooks
