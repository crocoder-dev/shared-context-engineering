# Plan: uncovered-ac

## Change summary

Add a refresh endpoint.

## Design decisions

- D1: The session store validates refresh tokens.
  - Boundary: `src/auth/` owns token validation.
  - Rationale: The store already tracks revocation.
  - Rejected: Stateless tokens — they cannot be revoked.
  - Trade-offs: One store read per refresh.

## Acceptance criteria

- [ ] AC1: `POST /auth/refresh` rejects a revoked token with 401.
  - Validate: `pnpm test src/auth/refresh.test.ts`
  - Decisions: D1
  - Scenarios: S1
- [ ] AC2: The OpenAPI spec documents `POST /auth/refresh`.
  - Validate: inspection of `openapi.yaml`.
  - Scenarios: none (inspection)

### Verification scenarios

- S1: Revoked refresh token is rejected
  - Given: A revoked refresh token.
  - When: The client calls `POST /auth/refresh`.
  - Then: The response is 401.
  - Check: `pnpm test src/auth/refresh.test.ts -t "rejects a revoked token"`

### Full validation

- `pnpm test`

## Task stack

- [ ] T01: `Add /auth/refresh endpoint` (status:todo)
  - Task ID: T01
  - Covers: AC1
  - Scope: In — route handler. Out — rotation policy.
  - Dependencies: none
  - Done when: The endpoint returns 401 for a revoked token.
  - Verify: `pnpm test src/auth/refresh.test.ts`
  - Context synchronization: pending

## Open questions

None.
