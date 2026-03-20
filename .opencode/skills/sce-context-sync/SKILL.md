---
name: sce-context-sync
description: "Use when user wants to update project documentation to reflect code changes, sync docs with code, refresh project context, or keep AI memory files accurate after completing an implementation task. Scans modified code, classifies the change significance, then updates or verifies Markdown context files under `context/` (overview, architecture, glossary, patterns, context-map, and domain files) so that durable AI memory stays aligned with current code truth."
compatibility: opencode
---

## Principle
- Context is durable AI memory and must reflect current-state truth.
- If context and code diverge, code is source of truth.

## Mandatory sync pass (important-change gated)
For every completed implementation task, run a sync pass over these shared files:
- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`

Classify whether the task is an important change before deciding to edit or verify root context files.

## Root context significance gating
- **Root edits required** - task introduces cross-cutting behavior, repository-wide policy/contracts, architecture boundaries, or canonical terminology changes.
- **Verify-only** - task is localized to a single feature/domain with no root-level behavior, architecture, or terminology impact. Keep root files unchanged; capture details in domain files instead.
- Even when verify-only, still verify `context/overview.md`, `context/architecture.md`, and `context/glossary.md` against code truth before declaring done.

## Step-by-step sync pass workflow

1. **Classify the change** - Important change or verify-only (see [Classification Reference](#classification-reference) below).
2. **Read the affected code** - Review modified files to understand what actually changed.
3. **Verify root files** - Open `context/overview.md`, `context/architecture.md`, and `context/glossary.md`; confirm they match code truth.
4. **Edit or skip root files** - Important change: update relevant root files. Verify-only: leave root files unchanged.
5. **Create or update domain files** - Write or revise `context/{domain}/` files for feature-specific detail (see [Domain File Policy](#domain-file-creation-policy) below).
6. **Ensure feature existence** - Every newly implemented feature must have at least one durable canonical description discoverable from context (domain file or `context/overview.md` for cross-cutting features).
7. **Update `context/context-map.md`** - Add or refresh discoverability links to any new or changed context files.
8. **Add glossary entries** - For any new domain language introduced by the task.
9. **Final check** - Confirm all updated files are <= 250 lines, diagrams are present where needed, and links use relative paths.

### Before/after example
A task adds a new `PaymentGateway` abstraction used only in the payments domain (verify-only - domain-local).

**`context/glossary.md`** - unchanged (no new root-level terminology).

**New file: `context/payments/payment-gateway.md`:**
```markdown
# PaymentGateway

Abstraction over external payment processors (Stripe, Adyen).
Defined in `src/payments/gateway/`.

## Contract
- `charge(amount, token): Result`
- `refund(chargeId): Result`

See also: [overview.md](../overview.md), [context-map.md](../context-map.md)
```

**`context/context-map.md`** - updated with a link to `context/payments/payment-gateway.md`.

---

## Classification Reference

| Important change (root edits required) | Verify-only (root files unchanged) |
|---|---|
| New auth strategy replacing existing one - architecture + terminology | New field on an existing API response - localized, no architecture impact |
| Background job queue used across multiple domains - cross-cutting | Bug fix in a single service's retry logic - no new root-level behavior |
| Renaming a core concept (e.g., `Order` -> `Purchase`) - canonical terminology | New UI component added to an existing feature - no cross-cutting impact |

---

## Domain File Creation Policy

- Use `context/{domain}/` for detailed feature behavior.
- If a feature does not cleanly fit an existing domain file, create a new one - do not defer documentation.
- If the feature appears to be part of a larger future domain, document the implemented slice now in a focused file and link it to related context.
- Prefer a small, precise domain file over overloading `overview.md` with detail.
- If updates for the current feature/domain outgrow shared files, migrate detail into `context/{domain}/` files, keep concise pointers in shared files, and add discoverability links in `context/context-map.md`.

---

## Final-task requirement
- In the final plan task (validation/cleanup), confirm feature existence documentation is present and linked.
- If a feature was implemented but not represented in context, add the missing entry before declaring the task done.

## Quality constraints
- One topic per file.
- Prefer concise current-state documentation over narrative changelogs.
- Link related context files with relative paths.
- Include concrete code examples when needed to clarify non-trivial behavior.
- Every context file must stay at or below 250 lines; if it would exceed 250, split into focused files and link them.
- Add a Mermaid diagram when structure, boundaries, or flows are complex.
- Ensure major code areas have matching context coverage.
