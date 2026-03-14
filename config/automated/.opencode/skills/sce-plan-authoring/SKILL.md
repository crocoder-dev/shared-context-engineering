---
name: sce-plan-authoring
description: "Transforms a change request into a structured implementation plan saved under context/plans/, breaking work down into atomic, commit-sized tasks with clear goals, scope boundaries, acceptance criteria, and verification steps. Use when a user wants to create or update a project plan, task breakdown, implementation roadmap, or work plan — including requests like "plan this feature", "break this into tasks", "write an implementation plan", or "scope out this change". Handles ambiguity resolution through a blocking clarification gate before writing any plan, and produces a machine-friendly task list ready for handoff to the Shared Context Engineering (SCE) implementation agent."
compatibility: opencode
---

## Goal
Turn a human change request into `context/plans/{plan_name}.md`.

## Intake trigger
- If a request includes both a change description and success criteria, planning is mandatory before implementation.
- Planning does not imply execution approval.

## Clarification gate (blocking)
- Before writing or updating any plan, run an ambiguity check.
- If any critical detail is unclear, stop with structured error listing all unresolved items with category labels.
- Do not write or update `context/plans/{plan_name}.md` until all critical details are resolved.
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
- Incorporate resolved details into the plan before handoff.

**Example structured error (clarification gate triggered):**
```
PLANNING BLOCKED - unresolved critical details:

[scope]       Is the auth middleware change limited to the API gateway, or does it also cover the admin panel?
[dependency]  Should the new Redis client use the existing `ioredis` version (4.x) or upgrade to 5.x?
[criteria]    What does "performance acceptable" mean? Specific p95 threshold required.

Resolve all items above before planning can proceed.
```

## Plan format
1) Change summary
2) Success criteria
3) Constraints and non-goals
4) Assumptions (if any)
5) Task stack (`T01..T0N`)
6) Open questions (if any)

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

## Worked example

**Input change request:**
> "Add rate limiting to the public API so that unauthenticated requests are capped at 60 req/min per IP. Use Redis for the counter store. No changes to authenticated routes."

**Output plan (`context/plans/rate-limiting-public-api.md`):**

```markdown
# Plan: rate-limiting-public-api

## Change summary
Introduce per-IP rate limiting (60 req/min) for unauthenticated requests to the public API using a Redis-backed counter. Authenticated routes are out of scope.

## Success criteria
- Unauthenticated requests exceeding 60/min from a single IP receive HTTP 429.
- Authenticated requests are unaffected.
- Rate limit headers (`X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`) are present on all public responses.

## Constraints and non-goals
- Out of scope: authenticated route limiting, user-level quotas, Redis cluster setup.
- Must not increase p95 latency on public endpoints by more than 5 ms.

## Task stack

- [ ] T01: Add Redis rate-limit middleware for unauthenticated routes (status:todo)
  - Task ID: T01
  - Goal: Implement middleware that increments a Redis counter keyed by IP and returns 429 when the limit is exceeded.
  - Boundaries in scope: middleware module, unauthenticated route registration.
  - Boundaries out of scope: authenticated routes, Redis connection config (use existing client).
  - Done when: Middleware rejects the 61st request in a 60 s window with HTTP 429; requests 1-60 pass through.
  - Verification notes: `npm test src/middleware/rateLimiter.test.ts`; manual smoke test with `wrk -d 65s -c 1 http://localhost:3000/api/public`.

- [ ] T02: Expose rate limit response headers (status:todo)
  - Task ID: T02
  - Goal: Attach `X-RateLimit-*` headers to every response from the rate-limited middleware.
  - Boundaries in scope: header injection in the middleware added in T01.
  - Boundaries out of scope: changing status codes, logging, or metrics.
  - Done when: All public API responses include the three headers with correct values.
  - Verification notes: `curl -I http://localhost:3000/api/public/health` shows all three headers.

- [ ] T03: Validate, measure latency, and sync context (status:todo)
  - Task ID: T03
  - Goal: Run full test suite, confirm no regression on authenticated routes, verify latency budget, update context docs.
  - Boundaries in scope: integration tests, latency benchmarks, context/plans status update.
  - Boundaries out of scope: new feature work.
  - Done when: All tests green; p95 latency delta < 5 ms; plan marked complete.
  - Verification notes: `npm test`; `npm run bench:public`; review `context/plans/rate-limiting-public-api.md` checkboxes.
```

## Output contract
- Save plan under `context/plans/`.
- Confirm plan creation with `plan_name` and exact file path.
- Present the full ordered task list in chat.
- Prompt the user to start a new session with Shared Context Code agent to implement `T01`.
- Provide one canonical next command: `/next-task {plan_name} T01`.
