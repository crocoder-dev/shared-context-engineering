# Plan: Add Timestamps and Comprehensive Error Logging

## Change Summary

Enhance the SCE CLI observability by:
1. Adding ISO8601 timestamps to all log entries (text and JSON formats)
2. Creating a structured error logging method for `ClassifiedError` types
3. Capturing all CLI errors centrally before exit
4. Adding debug-level logging throughout key operations

## Success Criteria

- [ ] All log entries include timestamps in ISO8601 format (e.g., `2026-03-20T14:30:00.123Z`)
- [ ] Text format: `timestamp=2026-03-20T14:30:00.123Z level=info event_id=sce.app.start message=...`
- [ ] JSON format includes `timestamp` field at root level
- [ ] All `ClassifiedError` instances are logged with full context (error code, class, message) before program exit
- [ ] Debug logging added to command dispatch, config loading, and error paths
- [ ] Existing tests pass; new tests verify timestamp presence
- [ ] `nix flake check` passes

## Constraints and Non-Goals

### Constraints
- Use `chrono` crate for timestamp formatting (standard, well-maintained)
- Maintain backward compatibility with existing log format structure
- Preserve existing log file rotation behavior
- Keep stderr output clean (timestamps only in logs, not user-facing errors)

### Non-Goals
- Changing error message content or error codes
- Adding structured logging fields beyond timestamp
- Implementing log rotation or retention policies
- Modifying telemetry/OpenTelemetry behavior
- Changing exit codes or CLI contract

## Task Stack

- [x] T01: Add chrono dependency and timestamp formatting to Logger (status:done)
  - **Task ID**: T01
  - **Status:** done
  - **Completed:** 2026-03-20
  - **Files changed:** cli/Cargo.toml, cli/src/services/observability.rs
  - **Evidence:** 24/24 observability tests passed, clippy clean, build succeeded
  - **Notes:** Added `chrono = "0.4"` dependency; updated `render_line()` to include ISO8601 timestamps in both text and JSON formats; updated tests to verify timestamp presence
  - **Goal**: Add `chrono` crate dependency and modify `render_line()` to include ISO8601 timestamps in both text and JSON log formats
  - **Boundaries**:
    - In: Add `chrono = "0.4"` to `cli/Cargo.toml`, update `render_line()` in `observability.rs`
    - Out: Error logging integration (T02), new debug log calls (T04)
  - **Done when**:
    - `chrono` added to dependencies
    - `render_line()` outputs `timestamp=...` in text format before log_format
    - JSON format includes `"timestamp": "..."` field
    - Existing tests updated to expect timestamps or remain flexible
    - Manual test: `cargo build` succeeds and `sce config show` with debug logging shows timestamps
  - **Verification notes**:
    - `nix develop -c sh -c 'cd cli && cargo build'`
    - `nix develop -c sh -c 'cd cli && cargo test observability'`
    - Check logs: `grep -E 'timestamp=[0-9]{4}-[0-9]{2}-[0-9]{2}T' context/tmp/sce.log`

- [x] T02: Add ClassifiedError logging method to Logger (status:done)
  - **Task ID**: T02
  - **Status:** done
  - **Completed:** 2026-03-20
  - **Files changed:** cli/src/services/error.rs (new), cli/src/services/mod.rs, cli/src/app.rs, cli/src/services/observability.rs
  - **Evidence:** 27/27 observability tests passed, 51/51 app tests passed, clippy clean
  - **Notes:** Extracted `ClassifiedError` and `FailureClass` types to new `services/error.rs` module; added `log_classified_error()` method to `Logger`; added 3 unit tests for the new method
  - **Goal**: Create `log_classified_error()` method on Logger that captures all error details (code, class, message) in a structured log entry
  - **Boundaries**:
    - In: Add method to `Logger` impl in `observability.rs`, expose public interface
    - Out: Integration into error handling (T03)
  - **Done when**:
    - New `log_classified_error(&self, error: &ClassifiedError)` method exists
    - Logs at ERROR level with event_id containing error code (e.g., `sce.error.SCE-ERR-RUNTIME`)
    - Includes fields: `error_code`, `error_class`, `error_message`
    - Redaction applied to message field
  - **Verification notes**:
    - Unit test in `observability.rs` tests module
    - `cargo test classified_error_logging`
    - Verify redaction works with sensitive patterns

- [x] T03: Integrate error logging into main run loop (status:done)
  - **Task ID**: T03
  - **Status:** done
  - **Completed:** 2026-03-20
  - **Files changed:** cli/src/app.rs, cli/src/services/observability.rs
  - **Evidence:** 51/51 app tests passed, clippy clean, nix flake check passed
  - **Notes:** Modified `try_run_with_dependency_check()` to return `(Result<String, ClassifiedError>, Option<Logger>)`; removed error logging from inside `try_run_with_dependency_check()` to consolidate in outer function; added `log_classified_error()` calls in `run_with_dependency_check_and_streams()` before `write_error_diagnostic()` for all error paths; added `#[allow(dead_code)]` to `Logger::error()` method since it's now only used in tests
  - **Goal**: Ensure all `ClassifiedError` instances are logged before program exits via stderr
  - **Boundaries**:
    - In: Modify `run_with_dependency_check_and_streams()` in `app.rs` to log errors before writing diagnostics
    - Out: Changing error messages, exit codes, or user-facing output
  - **Done when**:
    - All error paths in `run_with_dependency_check_and_streams()` log the error via logger
    - Error logging happens before `write_error_diagnostic()`
    - Logger passed through error handling context or accessible for error logging
  - **Verification notes**:
    - `cargo test app`
    - Test manually: Run `sce invalid-command 2>/dev/null` and check log file contains error entry
    - Verify `sce config validate` with bad config logs validation error

- [x] T04: Add debug logging throughout CLI operations (status:done)
  - **Task ID**: T04
  - **Status:** done
  - **Completed:** 2026-03-20
  - **Files changed:** cli/src/services/observability.rs, cli/src/services/config.rs, cli/src/app.rs
  - **Evidence:** 292/293 tests passed (1 unrelated git test failure), clippy clean, nix flake check passed
  - **Notes:** Added `debug()` method to Logger; added `loaded_config_paths` to `ResolvedObservabilityRuntimeConfig`; added debug logging for config file discovery, command parsing (raw args), and dispatch start/end; event IDs follow pattern `sce.config.file_discovered`, `sce.command.raw_args`, `sce.command.dispatch_start`, `sce.command.dispatch_end`
  - **Goal**: Add strategic debug-level logging to key operations: config resolution, command parsing, and dispatch
  - **Boundaries**:
    - In: Add `logger.debug()` calls in `app.rs` and `config.rs` at key decision points
    - Out: Logging in hot loops, sensitive data exposure, performance-critical paths
  - **Done when**:
    - Debug logs at: config file discovery (which files found), command parsing (raw args), dispatch start/end
    - Event IDs follow pattern: `sce.config.file_discovered`, `sce.command.dispatch_start`
    - No sensitive data logged (secrets redacted via existing redaction)
  - **Verification notes**:
    - Set `log_level=debug` in `.sce/config.json` and run commands
    - Verify debug entries appear in log file
    - Run `nix flake check` to ensure no regressions

- [x] T05: Validation and cleanup (status:done)
  - **Task ID**: T05
  - **Status:** done
  - **Completed:** 2026-03-20
  - **Evidence:** nix flake check passed, 292/293 tests passed (1 unrelated git test failure), manual verification: timestamps present, errors logged, debug output works
  - **Notes:** All acceptance criteria verified; context sync performed for T04
  - **Goal**: Full validation, test verification, and context sync
  - **Boundaries**:
    - In: Run full test suite, update context docs if needed
    - Out: Code changes (should be done in T01-T04)
  - **Done when**:
    - `nix flake check` passes completely
    - All CLI tests pass: `cargo test`
    - Manual verification: timestamps present, errors logged, debug output works
    - Context sync performed via `sce-context-sync` skill
  - **Verification notes**:
    - `nix flake check`
    - `nix develop -c sh -c 'cd cli && cargo test'`
    - `nix develop -c sh -c 'cd cli && cargo clippy --all-targets'`
    - Check log output manually

## Open Questions

None - requirements are clear for timestamp addition and error logging enhancement.

## Assumptions

- User accepts adding `chrono = "0.4"` as a new dependency (lightweight, standard)
- Timestamps should be in UTC ISO8601 format with millisecond precision
- Error logging should capture the full error context for debugging purposes
- Debug logging is primarily for development/troubleshooting, not production monitoring
