---
name: sce-audit
description: >
  Audit an SCE repository's durable current-state context against the repository's
  current implementation, classify context drift, repair only evidence-backed
  current-state drift, and verify the result. Use when the user asks to run an SCE
  context audit, reconcile context/ with code, find stale or missing SCE context,
  or check whether the repository's SCE context correctly describes the system as
  implemented. Do not use for brownfield reconstruction, decisions, plans,
  handovers, task execution, plan validation, or ordinary code review.
---

# SCE Audit

## Purpose

Own this workflow from repository binding through its terminal user-visible
response. Execute the steps below directly and in order. Intermediate findings are
internal state, not inter-SCE workflow handoffs. Do not invoke another SCE skill,
sibling SCE package, or SCE workflow command.

This workflow performs an explicit whole-repository reconciliation of existing
current-state `context/` against the system as implemented now. It complements
`sce-brownfield`: brownfield reconstructs missing context, while audit checks and
repairs existing context. It also complements task context synchronization: task
sync maintains context incrementally, while audit is an explicit whole-context
check.

Relevant non-SCE skills may be used as helper capabilities during the active step.
A helper must return control to this workflow and must not alter the workflow's
scope, protected paths, write boundaries, verification, or terminal output.

## User-visible output

Read `references/output.md` before emitting any gate or terminal response. Use only
an applicable layout from that file. Do not expose the internal evidence ledger or
raw chain-of-thought. Summarize findings and evidence in the fields the layout
provides.

## Input

Bind to exactly one repository.

- Prefer the repository explicitly named, linked, attached, or opened by the user.
- When exactly one repository is already unambiguously in scope, use it.
- When repository access can resolve the target without asking, inspect available
  repository metadata first.
- When more than one repository remains plausible, ask which repository to audit
  and stop.

The repository identifier is environment binding, not audit scoping. A successful
run always audits the complete current-state context surface. Do not accept a path,
domain, file, or glob as a way to narrow the audit. If the user explicitly requests
a partial audit, render the **Unsupported scoped audit** layout and stop.

## Workflow

### 1. Confirm the context root

Confirm that `context/` exists before gathering implementation evidence.

When it does not exist, render the **Missing context bootstrap gate** layout with
`sce setup --bootstrap-context` as the required action and stop. Do not create the
context root from this workflow.

### 2. Establish the current-state context surface

Audit every file under `context/` whose purpose is to describe the system as it
exists now.

Always include these root files when present:

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`

Include current-state domain files and operational or contract documents linked
from the context map or otherwise clearly part of the repository's current-state
context.

The following areas are protected lifecycle or historical state, not current-state
audit targets:

- `context/decisions/`
- `context/plans/`
- `context/handovers/`
- `context/tmp/`

Never create, edit, move, rename, or delete anything in a protected area. Do not
use a protected file as authority for what the system does now. If a current-state
file links to protected history, the link may be checked for integrity, but the
historical content does not outrank implementation evidence.

Record the audit surface before classifying findings.

### 3. Gather implementation evidence

Use current implementation evidence as authority. Read enough of the repository to
establish the behavior, boundaries, contracts, and ownership that current-state
context is expected to describe.

Use this evidence priority when sources disagree:

1. Current source code and generated executable behavior.
2. Tests, schemas, migrations, build manifests, dependency manifests, executable
   configuration, deployment configuration, and runtime configuration.
3. Repository documentation that describes current behavior.
4. Existing current-state `context/`, which is the subject being audited rather
   than an authority over implementation.

Inspect repository structure broadly enough to cover:

- Entry points, executables, packages, modules, services, and published interfaces.
- Dependency direction and important architectural boundaries.
- Data models, persistence, schemas, migrations, and external integrations.
- Configuration surfaces and deployment/runtime behavior.
- Error handling, important invariants, and behavior protected by tests.
- User-visible or operator-visible capabilities that belong in durable context.

Do not use Git history, commit messages, plans, decisions, handovers, or external
web documentation as current-state authority. Repository access through a connected
Git provider is allowed for reading the repository itself; unrelated network
research is not evidence for this audit.

### 4. Build the bidirectional audit ledger

Audit in both directions before writing anything.

#### 4.1 Context -> implementation

For every material current-state claim in the audit surface, compare the claim with
implementation evidence and classify it as exactly one of:

- **verified** — current implementation evidence supports the claim.
- **drifted** — the same concern exists, but the current implementation differs
  materially from what context says.
- **orphaned** — context describes a capability, boundary, component, integration,
  or contract that implementation evidence proves no longer exists.
- **unverifiable** — available current implementation evidence is insufficient or
  contradictory, so changing the claim would require guessing.

#### 4.2 Implementation -> context

For every material implemented capability, boundary, contract, ownership rule, or
operational fact that belongs in durable current-state context, check whether
context represents it and classify it as:

- **verified** — context already represents it accurately.
- **missing** — implementation evidence proves the fact, but current-state context
  omits it materially.
- **unverifiable** — it is unclear whether the fact belongs in durable context or
  current evidence is insufficient to state it safely.

A wording difference is not drift. Classify only differences that would cause a
reader or coding agent to form a materially wrong model of the implemented system.

Keep the ledger internal. For every non-verified finding, retain the context path or
implementation location, concise evidence, classification, and intended action.

### 5. Decide which findings are repairable

Repair only proven current-state drift backed by implementation evidence.

- **verified** -> make no change.
- **drifted** -> update the existing current-state statement when the correct state
  is directly supported by implementation evidence.
- **missing** -> add the missing current-state fact to the most appropriate existing
  context file, or create a focused current-state domain file when no existing file
  owns the topic.
- **orphaned** -> remove or replace the obsolete current-state statement only when
  current implementation evidence proves it false. When removal would require
  guessing about intended ownership or replacement, reclassify it as
  **unverifiable**.
- **unverifiable** -> never edit the affected claim from this finding. Preserve it
  and report what evidence is missing.

Before writing, detect whether a target context file has uncommitted user changes
when the environment exposes that information. Never overwrite a dirty target file.
Move that repair to **unverifiable/blocked** and preserve the user's work.

If the environment cannot modify repository files without staging, committing,
pushing, or otherwise crossing the Git boundaries in this skill, do not perform the
write. Preserve the repository and render the **Blocked** layout with the proposed
repair targets and retry condition.

### 6. Apply bounded context repairs

Write only current-state context required by repairable findings.

Every write must obey all of these rules:

- Describe the system as it exists now, not the audit process and not a change
  narrative.
- Preserve the repository's established context structure and terminology.
- Prefer editing the file that already owns the topic over creating a parallel file.
- Keep one topic per file and keep files concise; when the repository follows the
  SCE 250-line hygiene rule, do not exceed it.
- Use relative Markdown links between context files.
- Update `context/context-map.md` when a new current-state file is created or an
  existing discoverability entry must change because of a proven repair.
- Add glossary entries only for domain language the implementation currently uses
  and the glossary does not define.
- Never write confidence scores, audit classifications, commit hashes, timestamps,
  or audit dates into durable current-state context.

Never modify application code, tests, schemas, migrations, build files, runtime
configuration, deployment configuration, or any other implementation artifact.
Never modify `context/decisions/`, `context/plans/`, `context/handovers/`, or
`context/tmp/`.

### 7. Verify after writing

After all bounded repairs, perform a fresh verification pass. Do not assume a write
is correct because it matched the intended patch.

Verify all of the following:

- Every changed current-state claim is supported by current implementation evidence.
- Every repaired **drifted**, **missing**, or **orphaned** finding now classifies as
  **verified**.
- No **unverifiable** finding was silently changed.
- No protected path changed.
- No implementation file changed.
- Dirty context files, when detectable, were preserved.
- New current-state files are discoverable through `context/context-map.md`.
- Relative links introduced or changed by the audit resolve.
- No audit metadata, confidence score, commit hash, timestamp, or audit date was
  written into durable context.

When the environment exposes a repository diff, inspect it as part of this step and
confirm that every changed path and hunk belongs to a ledger repair. Never stage or
commit the diff.

When a verification failure can be repaired without inventing a fact, repair the
context and rerun the failed check. Otherwise render the **Blocked** layout and stop.

### 8. Report

Render the **Completed audit** layout from `references/output.md`.

Report counts for `verified`, `drifted`, `missing`, `orphaned`, and `unverifiable`
from the pre-repair audit, list context files changed, summarize unresolved findings,
and state whether post-write verification passed.

Stop. Do not start another SCE workflow.

## Rules

- Audit at most one repository per invocation.
- Audit the full current-state context surface; never silently narrow scope.
- Use implementation and executable configuration as current-state authority.
- Never treat decisions, plans, handovers, temporary context, or Git history as
  current-state authority.
- Never modify `context/decisions/`, `context/plans/`, `context/handovers/`, or
  `context/tmp/`.
- Never modify implementation code, tests, configuration, schemas, or migrations.
- Never repair an **unverifiable** finding by guessing.
- Never create a decision or invoke `sce-decision`.
- Never create or update a plan, execute a task, synchronize plan state, or invoke
  `sce-next-task`, `sce-change-to-plan`, or `sce-validate`.
- Never stage files, create a Git commit, push, switch branches, rebase, reset, or
  rewrite repository history.
- Never create the `context/` root.
- Never infer success when post-write verification fails.
