# Shared Context Plan Workflow (`/change-to-plan`)

## Purpose

`/change-to-plan` turns one change request into one scoped implementation plan under `context/plans/`. The generated OpenCode Plan agent is only a routing surface for this command; workflow behavior belongs to the command and its two phase skills.

## Command entrypoint

`/change-to-plan {change request}`

The request must be non-empty. The workflow does not accept approval or execution flags.

## Phase ownership

1. `sce-context-load`
   - Confirms whether `context/` exists.
   - Loads only durable context relevant to the requested focus.
   - Reports gaps and context-versus-code drift without editing context.
   - Returns `loaded` or `bootstrap_required`.
2. `sce-plan-authoring`
   - Runs only from a complete `loaded` handoff.
   - Resolves ambiguity and material decisions before writing.
   - Creates or updates one plan with stable task IDs, explicit scope, dependencies, done checks, and verification notes.
   - Returns `plan_ready`, `needs_clarification`, or `blocked`.

The command forwards each phase result as the authoritative handoff rather than reconstructing it.

## Bootstrap boundary

When context loading returns `bootstrap_required`, the workflow stops without creating context and tells the user to run:

`sce setup --bootstrap-context`

After the user reports that bootstrap completed, the waiting workflow invokes
`sce-context-load` again and continues with the original request in the same
session.

## Planning boundary

- Planning does not implement application or test changes.
- One invocation authors at most one plan.
- Every executable task is sliced as one coherent commit unit by default.
- Durable repository context is read, not synchronized, during planning.
- A ready plan ends with an exact `/next-task {plan-path} {task-id}` handoff; the workflow does not request implementation approval itself.

## Flow

```mermaid
flowchart TD
    A["/change-to-plan {request}"] --> B["sce-context-load"]
    B --> C{"Context available?"}
    C -- "No" --> D["Stop: sce setup --bootstrap-context"]
    C -- "Yes" --> E["sce-plan-authoring"]
    E --> F{"Authoring result"}
    F -- "needs_clarification" --> G["Ask only the reported questions"]
    F -- "blocked" --> H["Report blocker and required action"]
    F -- "plan_ready" --> I["Emit /next-task handoff"]
```

## Canonical sources

- `config/pkl/base/workflow-change-to-plan.pkl`
- Generated baseline: `.pi/prompts/change-to-plan.md`
- Skills: `sce-context-load`, `sce-plan-authoring`
