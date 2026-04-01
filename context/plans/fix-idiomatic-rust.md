# Fix Non-Idiomatic Rust Patterns

## Change Summary
Systematically fix non-idiomatic Rust patterns across the CLI codebase to improve code quality, reduce unnecessary allocations, and align with Rust best practices. This is a refactoring-only change with no behavioral modifications.

## Success Criteria
- [ ] All unnecessary `.clone()` calls removed (where ownership isn't needed)
- [ ] All `let _ = ` patterns replaced with proper error handling or explicit `drop()`
- [ ] All `.unwrap()` calls removed from production code (tests can keep `.expect()`)
- [ ] Explicit `return` statements at end of functions converted to expression form
- [ ] `.to_string()` on string literals replaced with `String::from()` or `&'static str` where appropriate
- [ ] `if x.is_some()` patterns converted to `if let Some(...)` where clearer
- [ ] `nix flake check` passes after all changes
- [ ] No behavioral changes (all existing tests pass)

## Constraints and Non-Goals
- **Scope boundary:** Only idiomatic improvements - no feature changes or bug fixes
- **Test behavior:** Tests should continue to use `.expect()` for test assertions
- **API compatibility:** No changes to public function signatures
- **Performance:** Focus on clarity over micro-optimizations
- **Non-goals:** Major architectural refactoring, dependency updates, new features

## Assumptions
- The existing tests provide sufficient coverage to detect behavioral changes
- Some `.clone()` calls may be necessary due to borrow checker constraints - these will be identified and documented
- Not all `let _ = ` patterns need fixing; only those ignoring meaningful errors

---

## Task Stack

### Phase 1: Critical Fixes (Panics & Silent Errors)

- [ ] **T01: Fix `.unwrap()` calls in production code** (status:todo)
  - **Task ID:** T01
  - **Goal:** Replace all `.unwrap()` calls in non-test production code with proper error handling
  - **Boundaries (in/out of scope):**
    - In: `cli/src/command_surface.rs:100`, production code paths in `app.rs`, `cli_schema.rs`
    - Out: Test code (marked with `#[cfg(test)]`), `expect()` calls with messages
  - **Done when:**
    - No `.unwrap()` calls remain in production code
    - All replaced with `?`, `if let`, or `match` with proper error propagation
    - `nix flake check` passes
  - **Verification notes:**
    - `grep -r "\.unwrap()" cli/src/ --include="*.rs" | grep -v "#\[cfg(test)\]" | grep -v "mod tests"` should return no matches
    - `nix flake check` succeeds

- [ ] **T02: Fix `let _ = ` patterns that silently ignore Results** (status:todo)
  - **Task ID:** T02
  - **Goal:** Replace `let _ = ` patterns that ignore `Result` types with explicit error handling
  - **Boundaries (in/out of scope):**
    - In: `app.rs:125` (writeln ignore), `setup.rs:1150` (cleanup ignore), `observability.rs` file removal ignores, `doctor.rs:1467` (remove_dir_all)
    - Out: Deliberately ignored values that aren't Results (e.g., `let _ = value;`)
  - **Done when:**
    - All `let _ = ` patterns on Result types use `if let Err(e) = ...` or `match`
    - Or use explicit `drop()` for values where errors are truly inconsequential
    - `nix flake check` passes
  - **Verification notes:**
    - `grep -n "let _ = " cli/src/**/*.rs` shows only non-Result ignores
    - `nix flake check` succeeds

### Phase 2: Performance & Clarity (Allocations & Control Flow)

- [ ] **T03: Remove unnecessary `.clone()` calls** (status:todo)
  - **Task ID:** T03
  - **Goal:** Remove unnecessary `.clone()` calls where references suffice
  - **Boundaries (in/out of scope):**
    - In: `app.rs:348` (args.get(1)), `cli_schema.rs:27` (Command clone), `doctor.rs` test clones, `auth_command.rs` token clones
    - Out: Clones required by borrow checker (will be identified with comments)
  - **Done when:**
    - Unnecessary clones removed or documented why clone is required
    - Code compiles without errors
    - `nix flake check` passes
  - **Verification notes:**
    - `nix flake check` succeeds
    - Code review confirms clone necessity where retained

- [ ] **T04: Convert explicit `return` at end of functions to expression form** (status:todo)
  - **Task ID:** T04
  - **Goal:** Convert `return value;` at end of functions to just `value` (Rust idiomatic style)
  - **Boundaries (in/out of scope):**
    - In: `app.rs:98`, `app.rs:141-206` (early returns), `doctor.rs:858, 864`
    - Out: Early returns (returning before end of function), macro-generated returns
  - **Done when:**
    - Final expression returns use implicit form (no `return` keyword)
    - `nix flake check` passes
  - **Verification notes:**
    - Manual review of changed functions
    - `nix flake check` succeeds

- [ ] **T05: Replace `.to_string()` on literals with `String::from()` or `&'static str`** (status:todo)
  - **Task ID:** T05
  - **Goal:** Use more idiomatic string literal conversions
  - **Boundaries (in/out of scope):**
    - In: `doctor.rs` summary/remediation strings, other literal `.to_string()` calls
    - Out: Dynamic string construction, non-literal values
  - **Done when:**
    - String literals use `String::from()` instead of `.to_string()`
    - Or use `&'static str` where ownership isn't needed
    - `nix flake check` passes
  - **Verification notes:**
    - `grep -n '".*"\.to_string()' cli/src/**/*.rs` shows minimal matches
    - `nix flake check` succeeds

### Phase 3: Pattern Modernization

- [ ] **T06: Convert `if x.is_some()` to `if let Some(...)`** (status:todo)
  - **Task ID:** T06
  - **Goal:** Use `if let` pattern matching instead of `is_some()` + `unwrap()` or boolean checks
  - **Boundaries (in/out of scope):**
    - In: `doctor.rs:266` (hook_path_source), `doctor.rs:307, 710, 818`, `setup.rs:147, 509, 522`
    - Out: Cases where the boolean result is actually needed
  - **Done when:**
    - `if let` used where it improves clarity
    - `nix flake check` passes
  - **Verification notes:**
    - Review of changed locations shows improved clarity
    - `nix flake check` succeeds

- [ ] **T07: Fix `into_iter()` vs `iter()` usage** (status:todo)
  - **Task ID:** T07
  - **Goal:** Use `iter()` when ownership isn't needed instead of `into_iter()`
  - **Boundaries (in/out of scope):**
    - In: `app.rs:271` (args collection), `hooks.rs` iterator chains
    - Out: Cases where ownership transfer is required
  - **Done when:**
    - `iter()` used where ownership not needed
    - `nix flake check` passes
  - **Verification notes:**
    - `nix flake check` succeeds
    - Manual review confirms iterator usage

### Phase 4: Validation & Context Sync

- [ ] **T08: Final validation and context sync** (status:todo)
  - **Task ID:** T08
  - **Goal:** Run full test suite, verify no behavioral changes, sync context
  - **Boundaries (in/out of scope):**
    - In: Running `nix flake check`, updating context docs if needed
    - Out: Code changes (all done in previous tasks)
  - **Done when:**
    - `nix flake check` passes with no warnings
    - All tests pass
    - Context updated to reflect code improvements
  - **Verification notes:**
    - `nix flake check` succeeds
    - `cargo test` passes (if run directly)
    - Context files updated if patterns are documented

---

## Open Questions

1. **Q:** Should we add `#[allow(clippy::...)]` annotations for patterns we intentionally keep?
   **A:** Yes, document any intentional non-idiomatic patterns with clippy allow annotations.

2. **Q:** For `let _ = fs::remove_file()` patterns, should we use `if let Err(e)` or just `drop()`?
   **A:** Use `if let Err(e) = ...` and log at debug level if a logger is available; otherwise use explicit `let _ =` with a comment explaining why errors are ignored.

3. **Q:** Are there any `.clone()` calls that should be kept for future compatibility?
   **A:** Review on a case-by-case basis. Add comments explaining why clone is necessary if kept.

---

## Implementation Notes

### Priority Order
1. **T01** (Critical) - Panic risk in production
2. **T02** (Critical) - Silent error handling
3. **T03** (High) - Performance impact from allocations
4. **T04-T07** (Medium) - Code style improvements
5. **T08** (Required) - Final validation

### Dependencies
- T01 and T02 can be done in parallel
- T03-T07 are independent and can be done in any order after T01-T02
- T08 must be done last

### Testing Strategy
- After each task: `nix flake check`
- Before T08: Full test suite verification
- If any test fails, the corresponding task needs revisiting

---

## Rollback Plan
If issues are discovered during T08 validation:
1. Identify which task introduced the issue
2. Revert that specific task's changes
3. Document the blocking issue in this plan
4. Create a follow-up task to address with different approach
