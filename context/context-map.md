# Context Map

Primary context files:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Feature/domain context:
- `context/cli/placeholder-foundation.md` (CLI command surface, setup install flow, shared-runtime sync smoke gate, nested flake release package/app installability, and Cargo local install + crates.io readiness policy)
- `context/sce/shared-context-code-workflow.md`
- `context/sce/shared-context-plan-workflow.md` (canonical `/change-to-plan` workflow and clarification/readiness gate contract)
- `context/sce/plan-code-overlap-map.md` (T01 overlap matrix for Shared Context Plan/Code, related commands, and core skill ownership/dedup targets)
- `context/sce/dedup-ownership-table.md` (current-state canonical owner-vs-consumer matrix for shared SCE behavior domains and thin-command ownership boundaries)
- `context/sce/workflow-token-footprint-inventory.md` (canonical Plan/Execute workflow participant inventory for token-footprint analysis, including ownership and invocation boundaries)
- `context/sce/atomic-commit-workflow.md` (canonical `/commit` command + `sce-atomic-commit` skill contract and naming decision)

Working areas:
- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`

Recent decision records:
- `context/decisions/2026-02-28-pkl-generation-architecture.md`
- `context/decisions/2026-03-03-plan-code-agent-separation.md`
