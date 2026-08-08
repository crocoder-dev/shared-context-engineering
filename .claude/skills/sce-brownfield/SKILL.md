---
name: sce-brownfield
description: >
  Reconstruct durable context from an existing repository's own evidence
compatibility: claude
---

# SCE Brownfield

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.

This workflow reconstructs durable `context/` memory for a repository that has
none, or that has gaps. It is a cold-start and gap-fill tool, not a recurring
context-maintenance or drift-repair command. Ongoing maintenance stays owned by
the task and plan synchronization phases.

## Input

`$ARGUMENTS` is `[rebuild] [path ...]`. Parse it into two parts before any
investigation:

- An optional leading literal token `rebuild`. Its presence, and only its
  presence, grants rewrite authority over existing context files.
- Zero or more remaining tokens, each an additional local documentation path.
  A path may be a file or a directory, inside or outside the repository.

Parsing rules:

- Empty `$ARGUMENTS` is valid and selects additive mode with no extra paths.
- `rebuild` is recognized only as the first token. In any later position it is a
  path.
- Never infer `rebuild` from conversation content, repository state, or the
  apparent staleness of existing context. Only the literal leading token sets it.
- A remaining token that does not resolve to an existing readable local path is
  invalid input.

When input is invalid, render the **Invalid usage** layout from
`references/output.md` naming the offending token, and stop. Do not guess the
token's meaning, and do not investigate or write anything.

## Workflow

### 1. Confirm the context root

Before any investigation, evidence gathering, or write, check whether `context/`
exists.

When `context/` does not exist, there is no durable memory to fill. Render the
**Missing context bootstrap gate** layout from `references/output.md` with
`sce setup --bootstrap-context` as the required action, and stop. Read no
repository evidence and write no file.

Bootstrapping is the user's action, not this workflow's. This workflow never
creates the `context/` root, and never creates a file outside it.

Wait for the user. When they report the command ran, run this step again and
continue in this session. Do not restart argument parsing.

When `context/` exists, record which of the baseline files and directories are
present, then continue to the next step.

### 2. Gather evidence

Evidence is strictly local. Gather it in the priority order below. When two
sources disagree, the earlier source in this order wins, and the disagreement is
recorded for step 5.

#### 2.1 Current code first

Read the repository's current source before anything that describes it.
Establish:

- Entry points, executables, and published interfaces.
- Module and package boundaries, and the dependency direction between them.
- Data shapes, persistence, and external integrations.
- Error handling, configuration surfaces, and observable behavior.

Current code is the highest-authority evidence class. Nothing later in this order
overrides it.

#### 2.2 Tests, schemas, migrations, build and runtime configuration

Read, when present:

- Test suites, for intended behavior, invariants, and edge cases the code alone
  does not state.
- Schemas and migrations, for the data model and its history.
- Build, packaging, and dependency manifests, for the artifact set and toolchain.
- Runtime and deployment configuration, for environments, services, and
  operational behavior.

This class is executable truth. Treat it as second only to current code.

#### 2.3 Discover documentation explicitly

Do not assume a README is the only documentation. Sweep the repository for it:

- Repository-root Markdown and plain-text documents.
- `docs/`, `doc/`, `documentation/`, `wiki/`, `adr/`, `decisions/`, `rfcs/`,
  `design/`, and `notes/` directories, at any depth.
- Per-package or per-module `README`, `CONTRIBUTING`, `ARCHITECTURE`, and
  `CHANGELOG` files.
- Comment blocks that document a whole module rather than a single line.
- Issue, pull-request, and agent-instruction files committed to the repository,
  such as `AGENTS.md` or equivalent.

Record what the sweep found and what it did not. Documentation describes intent
and may be stale; it never outranks code.

#### 2.4 Read argument-supplied paths

Read every path supplied as an argument, as an additional documentation source
at the same authority as step 2.3. A supplied directory is read recursively for
documentation files.

A supplied path that exists but yields nothing usable is recorded as a gap, not
a failure.

#### 2.5 Read at least three months of Git history

Read no less than three months of history measured back from the current date.

Use it for:

- When and why current structure arrived.
- Migrations, renames, deletions, and reversals.
- Recurring risk areas and repeatedly repaired code.

Read older history only when recent evidence points at a still-relevant
architectural decision, migration, rename, deletion, or recurring risk. Follow
that thread as far back as it stays relevant, and no further.

History explains change. It is the weakest evidence class for current truth: a
commit message describes an intention at a moment, not the state of the code now.

#### 2.6 No network access

This workflow performs no network access. Do not fetch a URL, search the web,
query a package registry, call a remote API, or read any external documentation
source. Evidence is the local repository plus argument-supplied local paths, and
nothing else.

When local evidence is insufficient, that is a gap or a clarification question.
It is never a reason to go looking outside the repository.

### 3. Score every important fact

An important fact is any statement that would be written into `context/` as
durable truth. Maintain an internal ledger with one entry per important fact:
the statement, its supporting evidence, its status, and a confidence score from
`1` to `100`.

Assign the score from the evidence, not from how plausible the statement sounds:

- `90`–`100` **Verified** — directly observable in current code or executable
  configuration.
- `70`–`89` **Strongly supported** — consistent across two independent evidence
  classes, with no contradiction.
- `50`–`69` **Inferred** — supported by one evidence class and contradicted by
  none, but not directly observable.
- `1`–`49` **Clarification required** — guessed, supported only by documentation
  or history that the code does not confirm, or genuinely ambiguous.

A fact whose evidence conflicted and was resolved under step 5 is scored after
resolution and carries the status **Contradiction resolved**.

The ledger is internal state and chat evidence only. No score, and nothing
derived from one, is ever written under `context/`.

### 4. Block on low-confidence facts

Any important fact scoring below `50` blocks. It is not written as truth, and
it is not silently downgraded, hedged, or omitted to avoid asking.

Collect every blocking fact, group the questions by area so the user answers a
coherent set rather than a list of fragments, and render the **Clarification
gate** layout from `references/output.md`. Each question must offer at least two
concrete options drawn from the evidence, plus an explicit freeform answer.

Stop and wait. Write no context file while waiting.

When the user answers, apply the answers to the ledger, rescore the affected
facts, and continue in this session at step 5. Do not restart investigation, and
do not re-ask an answered question.

When a question goes unanswered, its fact is not written. Record it under
**Gaps** in the report.

### 5. Resolve and disclose contradictions

A contradiction is material when the conflicting statements would produce
different context. Ignore cosmetic disagreements such as wording or formatting.

Resolve each material contradiction by evidence priority: current code, then
executable configuration, then documentation, then history.

Classify each one:

- **Stale documentation** — documentation describes behavior the code no longer
  has.
- **Superseded decision** — history records a decision that later work reversed.
- **Divergent implementations** — two parts of the current code disagree with
  each other.
- **Unexplained history** — history and code disagree with no evidence of which
  is current.

Every material contradiction is disclosed in the report with its classification
and the interpretation written to context, whatever its confidence. Never resolve
one silently. A contradiction that cannot be resolved by evidence priority is a
blocking clarification under step 4.

### 6. Infer the context structure

Derive the `context/{domain}/` structure from the repository's own boundaries —
its modules, services, deployable units, and data ownership — not from a generic
template.

- One topic per file. A domain that owns several distinct concerns gets several
  focused files rather than one large one.
- Name domains in the repository's own terminology.
- Prefer extending the existing structure over inventing a parallel one. When
  `context/` already carries domains, new files join them.
- Plan the root files against what the evidence actually established:
  `overview.md`, `architecture.md`, `glossary.md`, `patterns.md`, and
  `context-map.md`.

Record the planned file set before writing it.

### 7. Write context

Writes are additive by default. In additive mode:

- Write only missing files and missing domains.
- Never overwrite, truncate, move, rename, or delete an existing context file.
- When a planned file already exists, leave it untouched and record it as
  skipped, even when the existing content looks thinner than what was found.

When the literal `rebuild` token was supplied, rewrite authority extends to
existing context files. Even then:

- Rewrite only files this reconstruction has evidence for.
- Never delete a context file. Rewriting is the whole of the granted authority.
- Never touch `context/plans/`, `context/handovers/`, `context/decisions/`, or
  `context/tmp/`. Those are owned by other workflows.
- Never modify a context file with uncommitted changes.

Every write obeys these content rules:

- Describe current state and resulting behavior, not the investigation that found
  it and not a change narrative.
- Never write a confidence score, a commit hash, a timestamp, or a date under
  `context/`.
- Never attribute a statement to this workflow, this session, or its evidence
  gathering.
- One topic per file, at most 250 lines, relative Markdown links between context
  files.
- A Mermaid diagram where structure, boundaries, or flows are complex enough that
  prose alone would not carry them; concrete code examples only where they
  clarify non-trivial behavior.
- Add a glossary entry for domain language the repository uses and the glossary
  does not define.

Link every created or updated domain file from `context/context-map.md`. When
the map already exists, edit it additively: add the missing entries and leave the
existing ones as they are.

Never write outside `context/`. Do not modify application code, tests, plans, or
configuration.

### 8. Audit the result

After writing, audit what was written. Every item below is blocking: a failure
stops the workflow rather than being noted in passing.

- No confidence score, commit hash, timestamp, or date appears under `context/`.
- Every created or updated domain file is reachable from
  `context/context-map.md`.
- Every written file is at or below 250 lines and covers one topic.
- Every link between context files is relative and resolves.
- No fact scoring below `50` was written as truth.
- No existing context file was overwritten unless `rebuild` was supplied, and
  none was deleted in any mode.
- Nothing was written outside `context/`.
- Every material contradiction found in step 5 appears in the report.

When an audit item fails, repair the written files when the repair is
unambiguous, then rerun the item. When it cannot be repaired without inventing a
fact, render the **Blocked** layout from `references/output.md` naming the failed
item, the files preserved, and the retry condition, and stop.

### 9. Report

Render the **Completed report** layout from `references/output.md` with the
mode, the written and untouched files, the fact ledger, the contradictions, the
gaps, and the audit outcome. Stop.

## Rules

- Reconstruct context at most once per invocation.
- Never create the `context/` root; `sce setup --bootstrap-context` owns that.
- Never write, move, or delete a file outside `context/`.
- Never delete a context file, in either mode.
- Never overwrite an existing context file unless the literal leading `rebuild`
  token was supplied.
- Never infer `rebuild` from anything other than that token.
- Never access the network or read a non-local source.
- Never write a fact scoring below `50` as truth.
- Never resolve a material contradiction without disclosing it.
- Never write a confidence score, hash, timestamp, or date under `context/`.
- Never invoke another skill, sibling package, or workflow command.
- Never synchronize context, validate a plan, select or execute a task, or create
  a Git commit.
- Never treat this workflow as recurring context maintenance.
