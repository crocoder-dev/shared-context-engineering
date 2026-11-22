# Test Design

Apply this guidance when a task writes or modifies tests. Establish expected
behavior and important failure cases before implementing. Tests do not have to be
written before the implementation; leave test implementation order to
engineering judgment.

## Requirements

- Before implementation, identify likely failure modes, boundary conditions, and
  alternative interpretations of the requirements.
- Derive expected results from acceptance criteria, specifications, or
  independently established examples. Do not derive expected results from
  production code or reuse its implementation logic.
- Design tests that distinguish correct behavior from plausible incorrect
  implementations. Prefer asymmetric examples and boundary cases on both sides
  of a threshold.
- For high-risk behavior, challenge the test oracle itself. Identify assumptions
  that implementation and tests might share.
- When useful, apply property-based, metamorphic, or differential testing.
  Select properties and comparisons capable of falsifying incorrect behavior.

## Scope

- Keep test design proportional to the approved task.
- Do not introduce new dependencies, testing frameworks, or substantial
  infrastructure without approval.
- Prefer existing testing tools and established repository patterns.
- Do not add tests solely to satisfy a testing technique.
- Preserve existing task boundaries and stop conditions.
