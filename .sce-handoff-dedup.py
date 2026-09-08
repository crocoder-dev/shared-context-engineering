from pathlib import Path
import subprocess

PR_HEAD = "dafbafbbc7f6ae00f9d879b6d9d621000553efb8"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]


def run(*args):
    subprocess.run(list(args), check=True)


def read(path):
    return Path(path).read_text()


def write(path, text):
    Path(path).write_text(text)


def replace_exact(text, old, new, expected=1, label="replacement"):
    actual = text.count(old)
    if actual != expected:
        raise SystemExit(f"{label}: expected {expected} occurrence(s), found {actual}")
    return text.replace(old, new)


def replace_marked(text, start_phrase, end_phrase, new_block, occurrence, label):
    starts = []
    pos = 0
    while True:
        pos = text.find(start_phrase, pos)
        if pos < 0:
            break
        starts.append(pos)
        pos += len(start_phrase)
    if occurrence >= len(starts):
        raise SystemExit(f"{label}: occurrence {occurrence} missing; found {len(starts)}")
    start = starts[occurrence]
    line_start = text.rfind("\n", 0, start) + 1
    prefix = text[line_start:start]
    end = text.find(end_phrase, start)
    if end < 0:
        raise SystemExit(f"{label}: end phrase missing")
    end += len(end_phrase)
    rendered = "\n".join((prefix + line) if line else "" for line in new_block.split("\n"))
    return text[:line_start] + rendered + text[end:]


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "david@crocoder.dev")
run("git", "checkout", "--detach", PR_HEAD)

tracked_docs = []
for target in TARGETS:
    tracked_docs.extend([
        Path(target) / "skills/sce-next-task/SKILL.md",
        Path(target) / "skills/sce-next-task/references/task-execution.md",
        Path(target) / "skills/sce-next-task/references/context-sync.md",
    ])

before_lines = {
    target: sum(len((Path(target) / rel).read_text().splitlines()) for rel in [
        "skills/sce-next-task/SKILL.md",
        "skills/sce-next-task/references/task-execution.md",
        "skills/sce-next-task/references/context-sync.md",
    ])
    for target in TARGETS
}

# 1. Canonical task-execution handoff owner.
next_path = Path("config/pkl/base/workflow-next-task.pkl")
next_text = next_path.read_text()

handoff_fields_start = "A successful `complete` handoff must explicitly contain all of these fields:"
handoff_fields_end = "context synchronization."
next_text = replace_marked(
    next_text,
    handoff_fields_start,
    handoff_fields_end,
    "\\(completeExecutionHandoffDefinition.render.apply(mode))",
    0,
    "dynamic complete handoff definition",
)
next_text = replace_marked(
    next_text,
    handoff_fields_start,
    handoff_fields_end,
    "\\(completeExecutionHandoffDefinition.render.apply(\"composite\"))",
    0,
    "composite complete handoff definition",
)

execution_completion_anchor = '''local executionCompletionBullet = model.semanticReference.apply(
  """
  - One valid terminal YAML result matching `references/execution-contract.yaml`
    was returned.
  """,
  "- One valid terminal internal state was returned."
)
'''
complete_handoff_helper = execution_completion_anchor + '''local completeExecutionHandoffDefinition = model.semanticReference.apply(
  """
  For `complete`, `references/execution-contract.yaml` is the authoritative
  handoff schema. Do not omit, invent, or reconstruct any required field when
  handing it to context synchronization.
  """,
  """
  A successful `complete` handoff must explicitly contain all of these fields:

  - The resolved `plan` object, including its path and completion counts.
  - The selected `task` identity, including its ID and title.
  - `changes.files_changed`, the implementation's baseline-relative changed-file list.
  - `changes.summary`, a concise implementation summary.
  - `verification`, with every reported outcome marked `passed` and its evidence.
  - `done_checks`, pairing every done check with evidence.
  - `plan_update`, proving the selected task was marked complete and evidence recorded.
  - `context_impact`, including classification, affected areas, and reason.

  Do not omit, invent, or reconstruct any of these fields when handing off to context
  synchronization.
  """
)
'''
next_text = replace_exact(next_text, execution_completion_anchor, complete_handoff_helper, 1, "insert complete handoff helper")

old_terminal = '''Before determining terminal status for a `complete` result, verify that the
handoff contains the resolved plan, task identity, baseline-relative changed
files, implementation summary, verification evidence, done-check evidence, plan
update, and context-impact classification listed above. The mandatory five-root-
file context pass remains required for every completed task, regardless of the
reported context-impact classification, because it is cheap, deterministic, and
load-bearing for context accuracy; `context_impact` must not be used to waive it.'''
new_terminal = '''Before returning a `complete` result, verify that it satisfies the authoritative
handoff contract above. The mandatory five-root-file context pass remains required
for every completed task, regardless of the reported context-impact classification,
because it is cheap, deterministic, and load-bearing for context accuracy;
`context_impact` must not be used to waive it.'''
next_text = replace_marked(next_text, old_terminal.splitlines()[0], old_terminal.splitlines()[-1], new_terminal, 0, "dynamic terminal handoff verification")
next_text = replace_marked(next_text, old_terminal.splitlines()[0], old_terminal.splitlines()[-1], new_terminal, 0, "composite terminal handoff verification")

old_return = '''A `complete` result is the authoritative handoff into step 3, which reads the
plan, completed task, changed files, implementation summary, verification
evidence, done-check evidence, and context-impact classification out of it. Step
3 is forbidden from reconstructing any of that, so it has to be present here.'''
new_return = '''A `complete` result is the authoritative handoff into step 3. Pass it unchanged;
step 3 must consume it instead of reconstructing its fields.'''
next_text = replace_marked(next_text, old_return.splitlines()[0], old_return.splitlines()[-1], new_return, 0, "dynamic return handoff")
next_text = replace_marked(next_text, old_return.splitlines()[0], old_return.splitlines()[-1], new_return, 0, "composite return handoff")

old_command_handoff = '''Pass that result verbatim. It is the authoritative handoff, and \\(taskContextSync.render.apply(mode)) owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.
    
    Do not restate, summarize, or reconstruct any part of the execution result.'''
new_command_handoff = '''Pass that `complete` result verbatim as the authoritative live handoff to
    \\(taskContextSync.render.apply(mode)). Do not restate, summarize, or reconstruct it.'''
next_text = replace_exact(next_text, old_command_handoff, new_command_handoff, 1, "package command handoff boundary")
next_path.write_text(next_text)

# 2. Composite entrypoint routes the handoff without retelling its fields.
content_path = Path("config/pkl/base/workflow-content.pkl")
content_text = content_path.read_text()
old_skill_handoff = '''Pass that result verbatim. It is the authoritative handoff, and the **Task context synchronization phase** owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the execution result.'''
new_skill_handoff = '''Pass that `complete` result verbatim as the authoritative live handoff to the
**Task context synchronization phase**. Do not restate, summarize, or reconstruct it.'''
content_text = replace_exact(content_text, old_skill_handoff, new_skill_handoff, 1, "composite entrypoint handoff boundary")
content_path.write_text(content_text)

# 3. Context sync consumes the schema owned by task execution instead of restating it.
sync_path = Path("config/pkl/base/workflow-context-sync.pkl")
sync_text = sync_path.read_text()

sync_anchor = 'local returnInternalState = workflow.semanticReference.apply("Return YAML", "Return internal state")\n'
sync_helper = sync_anchor + '''local completeTaskExecutionHandoff = workflow.semanticReference.apply(
  "the `complete` result contract owned by `sce-task-execution`",
  "the complete handoff contract in `references/task-execution.md`"
)
'''
sync_text = replace_exact(sync_text, sync_anchor, sync_helper, 1, "insert context-sync handoff reference")

old_dynamic_input = '''    A live execution result must have:

    ```\\(yamlFence.render.apply(mode))
    status: complete
    ```

    A cross-session retry has no separate `status` field to check; the
    completed task record's presence in the plan, identified by plan path and
    task ID, is itself the authoritative signal.

    Treat whichever source was supplied — the live execution result, or the
    completed task record read directly from the plan — as the authoritative
    source for:

    - The resolved plan and completed task.
    - Files changed by implementation.
    - The task's `Result`.
    - `Verify` outcomes.
    - `Done when` evidence.
    - Reported context impact.

    Treat `changes.files_changed`, or the completed task record's own `Files
    changed` field on retry, as the authoritative, pre-edit-baseline-relative
    attribution. Use that list when reconciling the implementation; do not
    replace it with a whole-working-tree scan or a fresh diff against `HEAD`.
    If it is absent, not baseline-relative, or contradictory with the rest of
    the record, return a `blocked` Markdown report without modifying context.

    This phase must not be \\(invokedFor.render.apply(mode)) `declined`, `blocked`, or `incomplete`
    execution results.

    Do not reconstruct a missing execution result or completed task record
    from conversation history.'''
new_dynamic_input = '''    A live execution result must have `status: complete` and satisfy
    \\(completeTaskExecutionHandoff.render.apply(mode)). Consume it verbatim; do not
    redefine or reconstruct its fields here.

    A cross-session retry has no separate `status` field to check; the
    completed task record's presence in the plan, identified by plan path and
    task ID, is itself the authoritative signal.

    For a cross-session retry, treat the completed task record read directly
    from the plan as authoritative for task identity, `Files changed`, `Result`,
    `Verify` outcomes, done-check evidence, and reported context impact. Its
    `Files changed` field remains the pre-edit-baseline-relative attribution;
    do not replace it with a whole-working-tree scan or a fresh diff against
    `HEAD`.

    This phase must not be \\(invokedFor.render.apply(mode)) `declined`, `blocked`, or `incomplete`
    execution results.

    Do not reconstruct a missing execution result or completed task record
    from conversation history.'''
sync_text = replace_exact(sync_text, old_dynamic_input, new_dynamic_input, 1, "dynamic context-sync input")

old_dynamic_validation = '''    Confirm that:

    - A live execution result has `status` exactly `complete`; a cross-session
      retry has no `status` field to check and is authoritative by the
      completed task record's presence in the plan.
    - A resolved plan path and task ID are present; a live execution result
      carries them in its `plan` and `task` objects, and a cross-session
      retry receives them directly from the caller that resolved the debt
      task.
    - Exactly one completed task is identified, and — on retry — its record is
      read directly from the plan by that plan path and task ID rather than
      reconstructed in-band.
    - `changes.files_changed`, or the completed task record's own `Files
      changed` field on retry, is present as the pre-edit-baseline-relative
      changed-file list.
    - Changed files and a `Result` (an implementation summary, for a live
      result) are present.
    - `Verify` outcomes (verification evidence, for a live result) are
      present.
    - Done-check evidence is present.
    - A context-impact classification is present.'''
new_dynamic_validation = '''    For a live execution result, confirm `status` is exactly `complete` and the
    result satisfies \\(completeTaskExecutionHandoff.render.apply(mode)). Do not
    reconstruct missing fields.

    For a cross-session retry, confirm that:

    - A resolved plan path and task ID are present.
    - Exactly one completed task record is read directly from the plan by that
      plan path and task ID rather than reconstructed in-band.
    - Its `Files changed` field is present as the pre-edit-baseline-relative
      changed-file list.
    - `Result`, `Verify` outcomes, done-check evidence, and a context-impact
      classification are present.'''
sync_text = replace_exact(sync_text, old_dynamic_validation, new_dynamic_validation, 1, "dynamic context-sync validation")

old_static_authority = '''Whichever was
supplied is the authoritative source, and this phase owns reading the plan,
task, changed files, verification evidence, and reported context impact out
of it.'''
new_static_authority = '''Whichever was
supplied is authoritative; this phase consumes it without redefining its shape.'''
sync_text = replace_exact(sync_text, old_static_authority, new_static_authority, 1, "composite context-sync authority")

old_static_input = '''A live execution result must have:

```text
status: complete
```

A cross-session retry has no separate `status` field to check; the completed
task record's presence in the plan, identified by plan path and task ID, is
itself the authoritative signal.

Use the report format in:

`references/sync-report.md`

Treat whichever source was supplied — the live execution result, or the
completed task record read directly from the plan — as the authoritative
source for:

- The resolved plan and completed task.
- `changes.files_changed`, or the completed task record's own `Files changed`
  field on retry, already attributed relative to the pre-edit Git baseline.
- Files changed by implementation.
- The task's `Result` (or implementation summary, for a live result).
- `Verify` outcomes (or verification evidence, for a live result).
- Done-check evidence.
- Reported context impact.'''
new_static_input = '''A live execution result must have `status: complete` and satisfy
\\(completeTaskExecutionHandoff.render.apply("composite")). Consume it verbatim; do
not redefine or reconstruct its fields here.

A cross-session retry has no separate `status` field to check; the completed
task record's presence in the plan, identified by plan path and task ID, is
itself the authoritative signal.

Use the report format in:

`references/sync-report.md`

For a cross-session retry, treat the completed task record read directly from
the plan as authoritative for task identity, `Files changed`, `Result`, `Verify`
outcomes, done-check evidence, and reported context impact. Its `Files changed`
field remains the pre-edit-baseline-relative attribution; do not replace it with
a whole-working-tree scan or a fresh diff against `HEAD`.'''
sync_text = replace_exact(sync_text, old_static_input, new_static_input, 1, "composite context-sync input")

old_static_validation = '''Confirm that:

- A live execution result has `status` exactly `complete`; a cross-session
  retry has no `status` field to check and is authoritative by the completed
  task record's presence in the plan.
- A resolved plan path and task ID are present; a live execution result
  carries them in its `plan` and `task` objects, and a cross-session retry
  receives them directly from the caller that resolved the debt task.
- Exactly one completed task is identified, and — on retry — its record is
  read directly from the plan by that plan path and task ID rather than
  reconstructed in-band.
- Changed files and a `Result` (an implementation summary, for a live result)
  are present.
- `Verify` outcomes (verification evidence, for a live result) are present.
- Done-check evidence is present.
- A context-impact classification is present.'''
new_static_validation = '''For a live execution result, confirm `status` is exactly `complete` and the
result satisfies \\(completeTaskExecutionHandoff.render.apply("composite")). Do not
reconstruct missing fields.

For a cross-session retry, confirm that:

- A resolved plan path and task ID are present.
- Exactly one completed task record is read directly from the plan by that plan
  path and task ID rather than reconstructed in-band.
- Its `Files changed` field is present as the pre-edit-baseline-relative
  changed-file list.
- `Result`, `Verify` outcomes, done-check evidence, and a context-impact
  classification are present.'''
sync_text = replace_exact(sync_text, old_static_validation, new_static_validation, 1, "composite context-sync validation")
sync_path.write_text(sync_text)

# 4. Refresh tracked composite mirrors with the same ownership split.
old_task_terminal = old_terminal
new_task_terminal = new_terminal
old_task_return = old_return
new_task_return = new_return

old_context_authority = '''Whichever was
supplied is the authoritative source, and this phase owns reading the plan,
task, changed files, verification evidence, and reported context impact out
of it.'''
new_context_authority = '''Whichever was
supplied is authoritative; this phase consumes it without redefining its shape.'''
old_context_input = '''A live execution result must have:

```text
status: complete
```

A cross-session retry has no separate `status` field to check; the completed
task record's presence in the plan, identified by plan path and task ID, is
itself the authoritative signal.

Use the report format in:

`references/sync-report.md`

Treat whichever source was supplied — the live execution result, or the
completed task record read directly from the plan — as the authoritative
source for:

- The resolved plan and completed task.
- `changes.files_changed`, or the completed task record's own `Files changed`
  field on retry, already attributed relative to the pre-edit Git baseline.
- Files changed by implementation.
- The task's `Result` (or implementation summary, for a live result).
- `Verify` outcomes (or verification evidence, for a live result).
- Done-check evidence.
- Reported context impact.'''
new_context_input = '''A live execution result must have `status: complete` and satisfy the complete
handoff contract in `references/task-execution.md`. Consume it verbatim; do not
redefine or reconstruct its fields here.

A cross-session retry has no separate `status` field to check; the completed
task record's presence in the plan, identified by plan path and task ID, is
itself the authoritative signal.

Use the report format in:

`references/sync-report.md`

For a cross-session retry, treat the completed task record read directly from
the plan as authoritative for task identity, `Files changed`, `Result`, `Verify`
outcomes, done-check evidence, and reported context impact. Its `Files changed`
field remains the pre-edit-baseline-relative attribution; do not replace it with
a whole-working-tree scan or a fresh diff against `HEAD`.'''
old_context_validation = old_static_validation
new_context_validation = '''For a live execution result, confirm `status` is exactly `complete` and the
result satisfies the complete handoff contract in `references/task-execution.md`.
Do not reconstruct missing fields.

For a cross-session retry, confirm that:

- A resolved plan path and task ID are present.
- Exactly one completed task record is read directly from the plan by that plan
  path and task ID rather than reconstructed in-band.
- Its `Files changed` field is present as the pre-edit-baseline-relative
  changed-file list.
- `Result`, `Verify` outcomes, done-check evidence, and a context-impact
  classification are present.'''

for target in TARGETS:
    skill = Path(target) / "skills/sce-next-task/SKILL.md"
    text = skill.read_text()
    text = replace_exact(text, old_skill_handoff, new_skill_handoff, 1, f"{target} entrypoint handoff")
    skill.write_text(text)

    task = Path(target) / "skills/sce-next-task/references/task-execution.md"
    text = task.read_text()
    text = replace_exact(text, old_task_terminal, new_task_terminal, 1, f"{target} task terminal handoff")
    text = replace_exact(text, old_task_return, new_task_return, 1, f"{target} task return handoff")
    task.write_text(text)

    sync = Path(target) / "skills/sce-next-task/references/context-sync.md"
    text = sync.read_text()
    text = replace_exact(text, old_context_authority, new_context_authority, 1, f"{target} sync authority")
    text = replace_exact(text, old_context_input, new_context_input, 1, f"{target} sync input")
    text = replace_exact(text, old_context_validation, new_context_validation, 1, f"{target} sync validation")
    sync.write_text(text)

# 5. Record ownership and completed follow-up task.
ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
ownership_anchor = '| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task` parses and conditionally passes `approve`; `references/output.md` owns only exact gate content/order; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |\n'
ownership_row = ownership_anchor + '| Live post-task execution handoff schema | `sce-task-execution` complete-result contract in `workflow-next-task.pkl` | Package mode uses `references/execution-contract.yaml`; composite mode defines the schema once in `references/task-execution.md`; `/next-task` passes it verbatim and task context sync validates/consumes it; cross-session retry records remain a separate persisted shape | intentional/keep |\n'
ownership = replace_exact(ownership, ownership_anchor, ownership_row, 1, "ownership row")
ownership_path.write_text(ownership)

after_lines = {
    target: sum(len((Path(target) / rel).read_text().splitlines()) for rel in [
        "skills/sce-next-task/SKILL.md",
        "skills/sce-next-task/references/task-execution.md",
        "skills/sce-next-task/references/context-sync.md",
    ])
    for target in TARGETS
}
reductions = {target: before_lines[target] - after_lines[target] for target in TARGETS}
if len(set(reductions.values())) != 1 or next(iter(reductions.values())) <= 0:
    raise SystemExit(f"unexpected tracked-target line reductions: {reductions}")
per_target_reduction = next(iter(reductions.values()))

plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T03: `Deduplicate next-task complete execution handoff schema`" in plan:
    raise SystemExit("T03 already exists")
plan += f'''\n\n## Follow-up: complete execution-handoff schema ownership\n\n- [x] T03: `Deduplicate next-task complete execution handoff schema` (status:done)\n  - Scope: the live same-session `complete` result produced by task execution, the\n    next-task handoff boundary, task-context-sync consumption/validation, canonical\n    Pkl sources, tracked Pi/Claude/Codex mirrors, and the ownership table.\n  - Ownership after change: package task execution uses its execution-contract YAML;\n    composite task execution defines the complete handoff fields once in\n    `references/task-execution.md`; `/next-task` passes the result verbatim; context\n    sync validates and consumes that contract without restating its field list.\n  - Separate shape preserved: cross-session synchronization recovery continues to\n    consume the persisted completed-task record and blocker; this task does not alter\n    synchronization-debt recovery semantics.\n  - Result: the selected next-task entrypoint/execution/context-sync documents shrink\n    by {per_target_reduction} Markdown lines per tracked target, while the live complete\n    handoff field list appears once in the composite task-execution reference.\n  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,\n    `nix run .#pkl-check-generated`, targeted ownership assertions across all three\n    tracked targets, and `git diff --check`.\n  - Context synchronization: synced.\n'''
plan_path.write_text(plan)

# 6. Targeted semantic assertions before repository checks.
for target in TARGETS:
    skill = read(Path(target) / "skills/sce-next-task/SKILL.md")
    task = read(Path(target) / "skills/sce-next-task/references/task-execution.md")
    sync = read(Path(target) / "skills/sce-next-task/references/context-sync.md")
    if task.count("A successful `complete` handoff must explicitly contain all of these fields:") != 1:
        raise SystemExit(f"{target}: complete handoff schema is not defined exactly once")
    if task.count("The resolved `plan` object") != 1:
        raise SystemExit(f"{target}: complete handoff field list repeated")
    if "handoff contains the resolved plan, task identity" in task:
        raise SystemExit(f"{target}: terminal-status schema retelling remains")
    if "which reads the\nplan, completed task, changed files" in task:
        raise SystemExit(f"{target}: return-step schema retelling remains")
    if "the complete handoff contract in `references/task-execution.md`" not in sync:
        raise SystemExit(f"{target}: context sync does not reference the owner")
    if "`plan_update`" in sync or "`changes.summary`" in sync:
        raise SystemExit(f"{target}: context sync restates live execution schema fields")
    if "owns reading the plan, task, changed files" in skill:
        raise SystemExit(f"{target}: entrypoint still retells live handoff fields")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-next-task.pkl",
    "config/pkl/base/workflow-context-sync.pkl",
    "config/pkl/base/workflow-content.pkl",
    "context/sce/dedup-ownership-table.md",
    "context/plans/compress-workflow-execution-preamble.md",
}
for target in TARGETS:
    planned.update({
        f"{target}/skills/sce-next-task/SKILL.md",
        f"{target}/skills/sce-next-task/references/task-execution.md",
        f"{target}/skills/sce-next-task/references/context-sync.md",
    })

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate next-task execution handoff")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
print(f"line reduction per tracked target: {per_target_reduction}")
