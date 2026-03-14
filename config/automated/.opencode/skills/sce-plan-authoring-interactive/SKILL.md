---
name: sce-plan-authoring-interactive
 description: Use when a user wants to create or update a Shared Context Engineering (SCE) implementation plan with interactive clarification. Triggers on requests like "write a plan", "create an implementation roadmap", "draft a rollout plan", "plan this change", or "break this into tasks" for a software change. Interactively resolves ambiguity by asking targeted clarifying questions, then produces a structured markdown plan under `context/plans/` with a change summary, success criteria, constraints, atomic task stack, and verification steps — ready to hand off to the Shared Context Code agent for execution.
compatibility: opencode
---

## Goal
Turn a human change request into `context/plans/{plan_name}.md`.

## Intake trigger
- If a request includes both a change description and success criteria, planning is mandatory before implementation.
- Planning does not imply execution approval.

## Clarification gate (blocking)
- Before writing or updating any plan, run an ambiguity check.
- If any critical detail is unclear, ask 1-3 targeted questions and stop.
- Do not write or update `context/plans/{plan_name}.md` until the user answers.
- Critical details that must be resolved before planning include:
  - scope boundaries and out-of-scope items
  - success criteria and acceptance signals
  - constraints and non-goals
  - dependency choices (new libs/services, versions, and integration approach)
  - domain ambiguity (unclear business rules, terminology, or ownership)
  - architecture concerns (patterns, interfaces, data flow, migration strategy, and risk tradeoffs)
  - task ordering assumptions and prerequisite sequencing
- Do not silently invent missing requirements.
- If the user explicitly allows assumptions, record them in an `Assumptions` section.
- Incorporate user answers into the plan before handoff.

### Clarification gate example

**User request:** "Add rate limiting to the API."

**Clarification questions asked before writing the plan:**
1. Which endpoints should be rate-limited - all public routes, authenticated routes only, or specific ones?
2. What are the limits (e.g., requests per minute per IP/token) and what should happen when a limit is exceeded (429 response, queue, or silent drop)?
3. Should this use an existing library (e.g., `express-rate-limit`) or is a custom middleware preferred?

*(Planning is blocked until the user answers all three.)*

## Plan format
1) Change summary
2) Success criteria
3) Constraints and non-goals
4) Task stack (`T01..T0N`)
5) Open questions (if any)

## Task format (required)
For each task include:
- Task ID
- Goal
- Boundaries (in/out of scope)
- Done when
- Verification notes (commands or checks)

## Atomic task slicing contract (required)
- Author each executable task as one atomic commit unit by default.
- Every task must be scoped so one contributor can complete it and land it as one coherent commit without bundling unrelated changes.
- If a candidate task would require multiple independent commits (for example: refactor + behavior change + docs), split it into separate sequential tasks before finalizing the plan.
- Keep broad wrappers (`polish`, `finalize`, `misc updates`) out of executable tasks; convert them into specific outcomes with concrete acceptance checks.

Use this quick atomicity check before accepting each task:
- `single_intent`: task delivers one primary outcome
- `single_area`: task touch scope is narrow and related
- `single_verification`: done checks validate one coherent change set

Example compliant skeleton:
- [ ] T0X: `[single intent title]` (status:todo)
  - Task ID: T0X
  - Goal: `[one outcome]`
  - Boundaries (in/out of scope): `[tight scope]`
  - Done when: `[clear acceptance for one coherent change]`
  - Verification notes (commands or checks): `[targeted checks for this change]`

Use checkbox lines for machine-friendly progress tracking:
- `- [ ] T01: ... (status:todo)`

## Required final task
- Final task is always validation and cleanup.
- It must include full checks and context sync verification.

## Output contract
- Save plan under `context/plans/`.
- Confirm plan creation with `plan_name` and exact file path.
- Present the full ordered task list in chat.
- Prompt the user to start a new session with Shared Context Code agent to implement `T01`.
- Provide one canonical next command: `/next-task {plan_name} T01`.

---

## Complete plan example

The following shows what a finished plan file looks like for a realistic request: *"Add per-user rate limiting to the REST API using `express-rate-limit`."*

**File:** `context/plans/api-rate-limiting.md`

```markdown
# Plan: api-rate-limiting

## Change summary
Add per-user rate limiting to all authenticated REST API endpoints using the
`express-rate-limit` library. Unauthenticated endpoints are out of scope.

## Success criteria
- Authenticated requests exceeding 100 req/min per token receive a 429 response
  with a `Retry-After` header.
- All existing authenticated-endpoint tests continue to pass.
- A new integration test confirms the 429 path is exercised.

## Constraints and non-goals
- **In scope:** authenticated routes under `/api/v1/`
- **Out of scope:** unauthenticated routes, IP-based limiting, admin bypass logic
- **Constraint:** must use `express-rate-limit ^7`; no custom Redis store in this change
- **Non-goal:** dashboarding or alerting on rate-limit events

## Task stack

- [ ] T01: Install and configure `express-rate-limit` middleware (status:todo)
  - Task ID: T01
  - Goal: Add `express-rate-limit` as a dependency and wire a per-token limiter
    into the authenticated router.
  - Boundaries (in/out of scope): `src/middleware/rateLimiter.ts` and
    `src/router/authenticated.ts` only; no changes to unauthenticated routes.
  - Done when: Middleware is applied to the authenticated router and the dev
    server starts without errors.
  - Verification notes: `npm install` succeeds; `npm run dev` starts; manual
    `curl` with a valid token returns 200.

- [ ] T02: Return correct 429 response with `Retry-After` header (status:todo)
  - Task ID: T02
  - Goal: Configure the limiter to respond with HTTP 429 and a `Retry-After`
    header when the per-token limit is exceeded.
  - Boundaries (in/out of scope): `src/middleware/rateLimiter.ts` handler only;
    no route logic changes.
  - Done when: Exceeding the limit returns `{ "error": "Too Many Requests" }`
    with status 429 and a `Retry-After` value in seconds.
  - Verification notes: `npm run test -- --grep "rate limit"` passes; manual
    burst test with `artillery` confirms 429 after 100 req/min.

- [ ] T03: Add integration test for the 429 path (status:todo)
  - Task ID: T03
  - Goal: Write one integration test that fires 101 requests and asserts the
    101st receives 429 with `Retry-After`.
  - Boundaries (in/out of scope): `tests/integration/rateLimiter.test.ts` only;
    no changes to existing test files.
  - Done when: New test file exists and `npm run test:integration` is green.
  - Verification notes: `npm run test:integration` exits 0; coverage report
    shows the 429 branch covered.

- [ ] T04: Validation and cleanup (status:todo)
  - Task ID: T04
  - Goal: Confirm full test suite passes, no regressions, and context files are
    in sync.
  - Boundaries (in/out of scope): Read-only audit of test results and context
    directory; no new code changes.
  - Done when: `npm test` exits 0; `context/` reflects the completed plan state.
  - Verification notes: `npm test && npm run lint`; verify
    `context/plans/api-rate-limiting.md` task statuses are all `done`.

## Open questions
_(none - all clarifications resolved before planning)_
```
