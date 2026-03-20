# Doctor Database Inventory Contract

## Scope

- This file defines the approved current-state contract for `sce-doctor-database-and-config-coverage` task `T02`.
- It specifies how `sce doctor` must report SCE-managed database surfaces for the active repository and how operators explicitly request an all-SCE database inventory.

## In scope

- repo-scoped doctor reporting for SCE-managed databases relevant to the active repository
- an explicit all-SCE-databases inventory surface under `sce doctor`
- ownership rules for which databases belong in doctor output
- output-shape expectations for text and JSON rendering
- the standing registration rule for future SCE-created database families

## Out of scope

- Rust implementation details
- config-schema validation behavior
- non-database doctor checks or remediation beyond the inventory contract needed by downstream tasks

## Canonical ownership

- `sce doctor` is the canonical operator-facing inventory surface for SCE-managed databases.
- Database inventory remains ownership-based, not filesystem-pattern-based.
- The current canonical SCE-managed database family is Agent Trace local persistence at `<state_root>/sce/agent-trace/local.db`.
- Doctor must not scan arbitrary SQLite files, arbitrary `*.db` files, or non-SCE state to infer inventory.

## Command surface contract

- Default database coverage stays within `sce doctor`.
- Repo-scoped readiness view remains part of the default `sce doctor` output when a repository target is available.
- The all-SCE-databases inventory must be requested through an explicit doctor surface rather than being mixed into the default readiness view.
- The current implementation uses the explicit doctor-owned `--all-databases` flag for that all-databases inventory while preserving:
  - `sce doctor` as the canonical entrypoint
  - diagnosis-by-default behavior
  - compatibility with text output and `--format json`
- The explicit all-databases request surface must be discoverable through `sce doctor --help` and deterministic enough for future automation.

## Repo-scoped inventory contract

- When `sce doctor` resolves an active non-bare repository target, the default readiness output still includes a repo-scoped SCE database inventory section.
- At current scope, that repo-scoped section is intentionally empty because no repo-owned SCE database currently exists.
- The global Agent Trace database does not become repo-scoped inventory merely because doctor is running inside a repository; it remains part of global SCE state.
- Repo-scoped database findings therefore do not currently contribute additional readiness failures beyond the other doctor check domains.

## All-SCE-databases inventory contract

- Doctor must support an explicit all-SCE-databases listing separate from the default readiness view.
- The all-databases inventory enumerates every database currently created by SCE on the local machine according to canonical ownership rules.
- At current scope, the all-databases inventory includes only the canonical Agent Trace local database.
- The listing must remain deterministic in both inclusion rules and ordering.
- The listing must distinguish database family, scope, and canonical path, so operators can tell:
-  - `global` SCE databases from any future `repo`-scoped databases
-  - expected-but-missing canonical databases from existing databases when the contract requires both to surface
- The all-databases inventory is read-only and must not imply repair unless a separately reported doctor problem already has an approved fix path.

## Output-shape expectations

- Text output must keep repo-scoped database reporting separate from the broader all-databases inventory surface.
- JSON output must expose machine-readable database records rather than embedding inventory only in prose.
- Each database record must carry stable fields for at least:
  - database family
  - scope (`global` or `repo`)
  - canonical path
  - ownership status
  - readiness or inventory status
- Repo-scoped JSON output must attach database records to the repository-targeted portion of doctor output.
- All-databases JSON output must render as a stable collection whose ordering is deterministic across identical state.
- If future repo-scoped database families require SCE-owned metadata for ownership attribution, missing or unreadable metadata must produce explicit doctor findings or explicit fallback fields rather than silently dropping a database from inventory.

## Ownership and readiness rules

- Doctor inventory may include only database families whose location and ownership are defined by SCE code or canonical context.
- A database family is global when its canonical path is shared across repositories and not derived from a repo root.
- A database family is repo-scoped when its canonical path is derived from one repository identity and can be attributed back to that repository through SCE-owned path or metadata rules.
- Repo-scoped readiness must not degrade into a generic cross-machine inventory dump; it stays focused on the active repository only.
- All-databases inventory must not redefine readiness semantics for unrelated doctor domains; it is an explicit inventory view layered onto the canonical doctor entrypoint.

## Registration rule for future SCE-managed databases

- Every new SCE-created database family must register doctor coverage in the same change stream that introduces it.
- Registration must define:
  - canonical database family name
  - scope classification (`global` or `repo`)
  - canonical path derivation or discovery method
  - whether it appears in default repo-scoped doctor output, all-databases inventory, or both
  - readiness expectations when missing, unreadable, stale, or unhealthy
  - whether doctor owns any repair path or remains inventory-only for that family
  - the durable context file that becomes the canonical owner for the new database contract
- Future database families must be added by extending ownership-based inventory rules, not by broadening doctor into generic filesystem discovery.

## Downstream implementation targets

- `T03` must implement the doctor parser/help/runtime/output work needed for repo-scoped and all-SCE database inventory.
- `T03` must keep the all-databases request explicit and deterministic.
- `T06` must sync root context so this contract is discoverable from shared overview and glossary surfaces once implementation lands.
