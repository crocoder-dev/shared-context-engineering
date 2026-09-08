from pathlib import Path
import subprocess

PR_HEAD = "1290925b2331c36aaf7c1c83a80c6afe534776fe"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]
ROOT_PATHS = [
    "context/overview.md",
    "context/architecture.md",
    "context/glossary.md",
    "context/patterns.md",
    "context/context-map.md",
]


def run(*args):
    subprocess.run(list(args), check=True)


def replace_exact(text, old, new, expected=1, label="replacement"):
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{label}: expected {expected} occurrence(s), found {count}")
    return text.replace(old, new)


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "david@crocoder.dev")
run("git", "checkout", "--detach", PR_HEAD)

old_package_execution = """    Before determining terminal status for a `complete` result, verify that the
    handoff contains the resolved plan, task identity, baseline-relative changed
    files, implementation summary, verification evidence, done-check evidence,
    plan update, and context-impact classification listed above. The mandatory
    five-root-file context pass remains required for every completed task,
    regardless of the reported context-impact classification, because it is
    cheap, deterministic, and load-bearing for context accuracy; `context_impact`
    must not be used to waive it.
"""
new_package_execution = """    Before determining terminal status for a `complete` result, verify that the
    handoff contains the resolved plan, task identity, baseline-relative changed
    files, implementation summary, verification evidence, done-check evidence,
    plan update, and context-impact classification listed above.
"""

old_composite_execution = """Before returning a `complete` result, verify that it satisfies the authoritative
handoff contract above. The mandatory five-root-file context pass remains required
for every completed task, regardless of the reported context-impact classification,
because it is cheap, deterministic, and load-bearing for context accuracy;
`context_impact` must not be used to waive it.
"""
new_composite_execution = """Before returning a `complete` result, verify that it satisfies the authoritative
handoff contract above.
"""

old_task_discovery_source = """    Then inspect existing repository context in this order when present:

    1. `context/context-map.md`
    2. Context files for the affected domain or subsystem
    3. `context/overview.md`
    4. `context/architecture.md`
    5. `context/glossary.md`
    6. `context/patterns.md`
    7. Operational, product, or decision records directly related to the change

    Use the context map and existing links to locate authoritative files.
"""
new_task_discovery_source = """    Then inspect existing repository context in this order when present:

    1. The context map and its links to affected domain or subsystem context
    2. The mandatory root pass defined below
    3. Operational, product, or decision records directly related to the change

    The mandatory root-pass subsection owns the exact five-file root set.
"""

old_plan_discovery_source = """    Then read the plan's `Context sync` section and inspect existing repository
    context in this order when present:

    1. Paths named by the plan's `Context sync` section
    2. `context/context-map.md`
    3. Context files for the affected domain or subsystem
    4. `context/overview.md`
    5. `context/architecture.md`
    6. `context/glossary.md`
    7. `context/patterns.md`
    8. Operational, product, or decision records directly related to the finished
       change

    Use the context map and existing links to locate authoritative files.
"""
new_plan_discovery_source = """    Then read the plan's `Context sync` section and inspect existing repository
    context in this order when present:

    1. Paths named by the plan's `Context sync` section
    2. The context map and its links to affected domain or subsystem context
    3. The mandatory root pass defined below
    4. Operational, product, or decision records directly related to the finished
       change

    The mandatory root-pass subsection owns the exact five-file root set.
"""

old_task_discovery_rendered = """Then inspect existing repository context in this order when present:

1. `context/context-map.md`
2. Context files for the affected domain or subsystem
3. `context/overview.md`
4. `context/architecture.md`
5. `context/glossary.md`
6. `context/patterns.md`
7. Operational, product, or decision records directly related to the change

Use the context map and existing links to locate authoritative files.
"""
new_task_discovery_rendered = """Then inspect existing repository context in this order when present:

1. The context map and its links to affected domain or subsystem context
2. The mandatory root pass defined below
3. Operational, product, or decision records directly related to the change

The mandatory root-pass subsection owns the exact five-file root set.
"""

# Task execution no longer owns synchronization policy.
next_task_path = Path("config/pkl/base/workflow-next-task.pkl")
text = next_task_path.read_text()
text = replace_exact(text, old_package_execution, new_package_execution, 1, "package task-execution root-pass restatement")
text = replace_exact(text, old_composite_execution, new_composite_execution, 1, "composite task-execution root-pass restatement")
next_task_path.write_text(text)

# Context sync owns the root set. Discovery references that contract instead of
# relisting the same five files. Apply the same source cleanup to the retained
# plan-sync role so the shared skeleton does not carry the duplication forward.
context_sync_path = Path("config/pkl/base/workflow-context-sync.pkl")
text = context_sync_path.read_text()
text = replace_exact(text, old_task_discovery_source, new_task_discovery_source, 1, "task context discovery root list")
text = replace_exact(text, old_plan_discovery_source, new_plan_discovery_source, 1, "plan context discovery root list")
context_sync_path.write_text(text)

# Tracked composite mirrors.
for target in TARGETS:
    execution_path = Path(target) / "skills/sce-next-task/references/task-execution.md"
    text = execution_path.read_text()
    text = replace_exact(text, old_composite_execution, new_composite_execution, 1, f"{target} task-execution root-pass restatement")
    execution_path.write_text(text)

    sync_path = Path(target) / "skills/sce-next-task/references/context-sync.md"
    text = sync_path.read_text()
    text = replace_exact(text, old_task_discovery_rendered, new_task_discovery_rendered, 1, f"{target} context discovery root list")
    sync_path.write_text(text)

# Generated semantic contract: context-sync's mandatory-root-pass subsection is
# the sole owner of the exact five-file set for next-task. Discovery may refer to
# the contract by name, while task execution may not restate synchronization policy.
contract_path = Path("config/pkl/renderers/generation-contract-check.pkl")
text = contract_path.read_text()
anchor = """hidden assertSyncDebtRecoveryBranch = (documents: Mapping) ->
"""
assertion = """local mandatoryRootContextPaths = new Listing {
  "`context/overview.md`"
  "`context/architecture.md`"
  "`context/glossary.md`"
  "`context/patterns.md`"
  "`context/context-map.md`"
}

hidden assertNextTaskRootPassOwnership = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      if (path.endsWith("/skills/sce-next-task/references/task-execution.md"))
        !text.contains("five-root-file context pass")
        && !text.contains("mandatory root pass")
      else if (path.endsWith("/skills/sce-next-task/references/context-sync.md"))
        text.contains("### The mandatory root pass")
        && text.contains("## 3.3 Discover applicable context")
        && text.contains("## 3.4 Determine whether durable context changed")
        && let (discoveryTail = text.drop(text.indexOf("## 3.3 Discover applicable context")))
          let (discovery = discoveryTail.take(discoveryTail.indexOf("### The mandatory root pass")))
            let (rootTail = text.drop(text.indexOf("### The mandatory root pass")))
              let (rootPass = rootTail.take(rootTail.indexOf("## 3.4 Determine whether durable context changed")))
                discovery.contains("The mandatory root-pass subsection owns the exact five-file root set.")
                && mandatoryRootContextPaths.every((rootPath) -> !discovery.contains(rootPath))
                && mandatoryRootContextPaths.every((rootPath) -> rootPass.contains(rootPath))
      else true
    )
  ) "sce-next-task root pass: context-sync solely owns the exact five-file set"
  else throw("sce-next-task must define the exact five-file mandatory root pass only in context-sync's mandatory-root-pass subsection; discovery may reference it by name and task-execution must not restate it")

"""
if assertion not in text:
    text = replace_exact(text, anchor, assertion + anchor, 1, "root-pass ownership assertion insertion")
check_anchor = """  ["context-sync-validates-task-record"] = assertContextSyncValidatesTaskRecord.apply(workflowDocuments)
"""
check_line = """  ["next-task-root-pass-ownership"] = assertNextTaskRootPassOwnership.apply(workflowDocuments)
"""
if check_line not in text:
    text = replace_exact(text, check_anchor, check_anchor + check_line, 1, "root-pass ownership check registration")
contract_path.write_text(text)

# Durable ownership context.
ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
if "| Mandatory five-root context pass |" not in ownership:
    anchor_row = "| Post-task durable context synchronization |"
    lines = ownership.splitlines()
    for i, line in enumerate(lines):
        if line.startswith(anchor_row):
            lines.insert(i + 1, "| Mandatory five-root context pass | `sce-task-context-sync` mandatory-root-pass subsection in `workflow-context-sync.pkl` | Task execution only hands off `context_impact`; context discovery references the named pass instead of relisting its five files; impact and verification sections may enforce the owned contract without redefining the file set | intentional/keep |")
            break
    else:
        raise SystemExit("ownership table post-task sync row not found")
    ownership_path.write_text("\n".join(lines) + "\n")

# Record the fifth focused change on the active PR plan.
plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T05: `Deduplicate the mandatory five-root context pass`" not in plan:
    plan += """

## Follow-up: mandatory root-pass ownership

- [x] T05: `Deduplicate the mandatory five-root context pass` (status:done)
  - Scope: next-task task execution and task context synchronization, the retained
    plan-sync discovery list in the shared context-sync source, generated semantic
    checks, tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: the task context-sync **mandatory root pass** subsection
    is the sole owner of the exact five root paths. Discovery references that named
    contract instead of relisting the paths; task execution only hands off
    `context_impact` and does not restate synchronization policy.
  - Behavior preserved: the same five root files remain mandatory on every task
    synchronization invocation; missing files remain reportable gaps; impact
    classifications still cannot waive the pass; synchronization verification still
    requires every root file to be checked against code truth.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted Pi/Claude/Codex ownership assertions,
    and `git diff --check`.
  - Context synchronization: synced.
"""
    plan_path.write_text(plan)

# Focused invariants on tracked mirrors and simple Markdown reduction evidence.
removed_lines = 0
for target in TARGETS:
    execution = (Path(target) / "skills/sce-next-task/references/task-execution.md").read_text()
    sync = (Path(target) / "skills/sce-next-task/references/context-sync.md").read_text()

    if "five-root-file context pass" in execution or "mandatory root pass" in execution:
        raise SystemExit(f"{target}: task execution still restates root-pass policy")

    discovery_tail = sync.split("## 3.3 Discover applicable context", 1)[1]
    discovery, root_tail = discovery_tail.split("### The mandatory root pass", 1)
    root_pass = root_tail.split("## 3.4 Determine whether durable context changed", 1)[0]
    if "The mandatory root-pass subsection owns the exact five-file root set." not in discovery:
        raise SystemExit(f"{target}: discovery does not point at root-pass owner")
    for root_path in ROOT_PATHS:
        token = f"`{root_path}`"
        if token in discovery:
            raise SystemExit(f"{target}: discovery still relists {root_path}")
        if token not in root_pass:
            raise SystemExit(f"{target}: root pass lost {root_path}")

    removed_lines += old_composite_execution.count("\n") - new_composite_execution.count("\n")
    removed_lines += old_task_discovery_rendered.count("\n") - new_task_discovery_rendered.count("\n")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-next-task.pkl",
    "config/pkl/base/workflow-context-sync.pkl",
    "config/pkl/renderers/generation-contract-check.pkl",
    "context/plans/compress-workflow-execution-preamble.md",
    "context/sce/dedup-ownership-table.md",
}
for target in TARGETS:
    planned.add(f"{target}/skills/sce-next-task/references/task-execution.md")
    planned.add(f"{target}/skills/sce-next-task/references/context-sync.md")

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

print(f"tracked next-task Markdown lines removed across Pi/Claude/Codex: {removed_lines}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate next-task root context pass")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
