# Shell Operator Parsing for Bash Policy Enforcement

## Change summary

Extend the bash-policy runtime to parse shell control operators (`|`, `&&`, `||`, `;`, `&`) and evaluate each resulting command segment against configured policies. If any segment matches a blocking policy, the entire command is blocked (strict enforcement).

This addresses the current gap where `cat abc | git diff` bypasses `forbid-git-all` because the policy only checks the first executable token.

## Success criteria

1. `cat abc | git diff` with `forbid-git-all` enabled blocks the command
2. `git status && npm install` with `forbid-git-all` blocks the command  
3. `ls; git push` with `forbid-git-commit` blocks the command
4. Non-matching segments still allow their executables (e.g., `cat file | ls` passes with no git policies)
5. All existing tests continue to pass
6. Preset messages remain unchanged for single-command cases

## Constraints and non-goals

- **In scope**: Shell operator parsing (`|`, `&&`, `||`, `;`, `&`), segment evaluation, integration with existing policy matching
- **Out of scope**: 
  - Redirection operators (`>`, `<`, `>>`, `<<`)
  - Nested shell payloads (`bash -lc "..."`)
  - Alias expansion and functions
  - Subshells and process substitution
- **Non-goal**: Change the policy matching logic for individual segments (prefix matching stays the same)
- **Presets**: Modify existing preset behavior (forbid-git-all, forbid-git-commit, etc.) to use shell parsing automatically

## Task stack

- [x] T01: Add shell operator tokenization function (status:todo)
  - Task ID: T01
  - Goal: Create `parseCommandSegments()` function that splits a command string into segments at shell operators
  - Boundaries (in/out of scope): In - splitting on `|`, `&&`, `||`, `;`, `&`. Out - redirection handling, nested quotes
  - Done when: Function returns `[["cat", "abc"], ["git", "diff"]]` for input `"cat abc | git diff"`
  - Verification notes: Add unit test with command variants

- [x] T02: Integrate segment parsing into evaluateBashCommandPolicy (status:done)
  - Task ID: T02
  - Goal: Modify `evaluateBashCommandPolicy` to parse command into segments and check each against policies
  - Boundaries (in/out of scope): In - segment parsing, policy evaluation per segment, early exit on match. Out - error handling changes
  - Done when: `evaluateBashCommandPolicy` returns blocked for `cat abc | git diff` with forbid-git-all
  - Verification notes: Run existing test suite, verify single-command behavior unchanged
  - Status: done
  - Completed: 2025-03-23
  - Files changed: config/lib/bash-policy-plugin/bash-policy/runtime.ts
  - Evidence: 54/54 tests passed, implementation parses segments and blocks on any matching policy

- [x] T03: Add comprehensive test coverage for shell operator cases (status:done)
  - Task ID: T03
  - Goal: Add test cases for all operator types and their combinations
  - Boundaries (in/out of scope): In - all operator types, edge cases. Out - integration tests with actual CLI
  - Done when: Test file covers `|`, `&&`, `||`, `;`, `&` with matching and non-matching segments
  - Verification notes: `bun test config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`
  - Status: done
  - Completed: 2025-03-23
  - Files changed: config/lib/bash-policy-plugin/bash-policy-runtime.test.ts
  - Evidence: 62/62 tests passed (8 new shell operator integration tests)

- [x] T04: Update preset catalog and documentation (status:done)
  - Task ID: T04
  - Goal: Verify preset documentation reflects shell operator handling behavior
  - Boundaries (in/out of scope): In - bash-tool-policy-enforcement-contract.md updates. Out - generating new presets
  - Done when: Contract doc reflects shell operator parsing is now part of matching
  - Verification notes: Review context/sce/bash-tool-policy-enforcement-contract.md
  - Status: done
  - Completed: 2025-03-23
  - Files reviewed: context/sce/bash-tool-policy-enforcement-contract.md
  - Evidence: Contract doc already contains Shell Operator Parsing Extension section (lines 125-144) with implementation details and examples

- [x] T05: Final validation and context sync (status:done)
  - Task ID: T05
  - Goal: Run full test suite and sync any context changes
  - Boundaries (in/out of scope): In - all tests, context consistency. Out - none
  - Done when: All tests pass, context files updated if needed
  - Verification notes: `bun test config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`; check context/sce/ for drift
  - Status: done
  - Completed: 2025-03-23
  - Evidence: 62/62 tests passed

## Validation Report

### Commands run
- `bun test config/lib/bash-policy-plugin/bash-policy-runtime.test.ts` -> exit 0 (62 tests passed, 0 failed)

### Success-criteria verification
- [x] `cat abc | git diff` with `forbid-git-all` enabled blocks the command -> confirmed via new test "blocks cat abc | git diff with forbid-git-all"
- [x] `git status && npm install` with `forbid-git-all` blocks the command -> confirmed via new test "blocks git status && npm install with forbid-git-all"
- [x] `ls; git push` with `forbid-git-commit` blocks the command -> confirmed via new test "blocks ls; git push with forbid-git-commit"
- [x] Non-matching segments still allow their executables -> confirmed via test "allows cat file | ls with forbid-git-all"
- [x] All existing tests continue to pass -> 54/54 original tests pass
- [x] Preset messages remain unchanged for single-command cases -> existing behavior preserved

### Residual risks
- None identified.


## Open questions

None - all critical details clarified with user.

## Implementation notes

### Segment parsing approach

```typescript
// Input: "cat abc | git diff && npm run build; ls &"
const tokens = tokenizeShellCommand(command);
// ["cat", "abc", "|", "git", "diff", "&&", "npm", "run", "build", ";", "ls", "&"]

// Split at operators:
const segments = [
  ["cat", "abc"],
  ["git", "diff"],
  ["npm", "run", "build"],
  ["ls"]
];
```

### Policy evaluation

- Parse all segments upfront
- Evaluate each segment's normalized argv against active policies
- If ANY segment matches a blocking policy, block the entire command
- First matching policy wins for the block message

### Edge cases

- Empty segments (consecutive operators): Skip
- Trailing operator: Skip empty segment after
- Quoted operators: `cat | grep` - pipe is operator, `"cat | grep"` - not an operator inside quotes