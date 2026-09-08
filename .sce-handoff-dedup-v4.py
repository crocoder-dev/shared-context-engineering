from pathlib import Path
import subprocess

PR_HEAD = "dafbafbbc7f6ae00f9d879b6d9d621000553efb8"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]


def run(*args):
    subprocess.run(list(args), check=True)


def replace_once(text, old, new, label):
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected 1 occurrence, found {count}")
    return text.replace(old, new, 1)


def replace_in_section(text, start_marker, end_marker, pairs, label):
    start = text.index(start_marker)
    end = text.index(end_marker, start)
    section = text[start:end]
    for old, new, sublabel in pairs:
        section = replace_once(section, old, new, f"{label}/{sublabel}")
    return text[:start] + section + text[end:]


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "david@crocoder.dev")
run("git", "checkout", "--detach", PR_HEAD)

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

old_return = '''A `complete` result is the authoritative handoff into step 3, which reads the
plan, completed task, changed files, implementation summary, verification
evidence, done-check evidence, and context-impact classification out of it. Step
3 is forbidden from reconstructing any of that, so it has to be present here.'''
new_return = '''A `complete` result is the authoritative handoff into step 3. Pass it unchanged;
step 3 must consume it instead of reconstructing its fields.'''

old_skill_handoff = '''Pass that result verbatim. It is the authoritative handoff, and the **Task context synchronization phase** owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the execution result.'''
new_skill_handoff = '''Pass that `complete` result verbatim as the authoritative live handoff to the
**Task context synchronization phase**. Do not restate, summarize, or reconstruct it.'''

old_sync_authority = '''Whichever was
supplied is the authoritative source, and this phase owns reading the plan,
task, changed files, verification evidence, and reported context impact out
of it.'''
new_sync_authority = '''Whichever was
supplied is authoritative; this phase consumes it without redefining its shape.'''

old_sync_input = '''A live execution result must have:

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
new_sync_input = '''A live execution result must have `status: complete` and satisfy the complete
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

old_sync_validation = '''Confirm that:

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
new_sync_validation = '''For a live execution result, confirm `status` is exactly `complete` and the
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

before_lines = {}
for target in TARGETS:
    before_lines[target] = sum(
        len((Path(target) / rel).read_text().splitlines())
        for rel in [
            "skills/sce-next-task/SKILL.md",
            "skills/sce-next-task/references/task-execution.md",
            "skills/sce-next-task/references/context-sync.md",
        ]
    )

# Canonical composite task-execution reference only. Package execution keeps its
# existing YAML contract and package-local explanation unchanged.
next_path = Path("config/pkl/base/workflow-next-task.pkl")
text = next_path.read_text()
text = replace_in_section(
    text,
    'nextTaskTaskExecutionReference = """',
    '\n"""\n\nnextTaskOutputReference = """',
    [(old_terminal, new_terminal, "terminal"), (old_return, new_return, "return")],
    "next-task composite task execution",
)
next_path.write_text(text)

content_path = Path("config/pkl/base/workflow-content.pkl")
content = content_path.read_text()
content = replace_once(content, old_skill_handoff, new_skill_handoff, "next-task entrypoint handoff")
content_path.write_text(content)

sync_path = Path("config/pkl/base/workflow-context-sync.pkl")
sync = sync_path.read_text()
sync = replace_in_section(
    sync,
    'taskReference = """',
    '\n"""\n\nhidden taskSkillBody',
    [
        (old_sync_authority, new_sync_authority, "authority"),
        (old_sync_input, new_sync_input, "input"),
        (old_sync_validation, new_sync_validation, "validation"),
    ],
    "task context sync composite reference",
)
sync_path.write_text(sync)

for target in TARGETS:
    skill = Path(target) / "skills/sce-next-task/SKILL.md"
    t = skill.read_text()
    skill.write_text(replace_once(t, old_skill_handoff, new_skill_handoff, f"{target} entrypoint"))

    execution = Path(target) / "skills/sce-next-task/references/task-execution.md"
    t = execution.read_text()
    t = replace_once(t, old_terminal, new_terminal, f"{target} execution terminal")
    t = replace_once(t, old_return, new_return, f"{target} execution return")
    execution.write_text(t)

    context = Path(target) / "skills/sce-next-task/references/context-sync.md"
    t = context.read_text()
    t = replace_once(t, old_sync_authority, new_sync_authority, f"{target} sync authority")
    t = replace_once(t, old_sync_input, new_sync_input, f"{target} sync input")
    t = replace_once(t, old_sync_validation, new_sync_validation, f"{target} sync validation")
    context.write_text(t)

ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
anchor = '| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task` parses and conditionally passes `approve`; `references/output.md` owns only exact gate content/order; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |\n'
row = anchor + '| Live post-task execution handoff schema | composite `references/task-execution.md` generated from `nextTaskTaskExecutionReference` | `/next-task` passes the `complete` result verbatim; composite task context sync validates and consumes that contract without restating the live field list; package mode retains `references/execution-contract.yaml`; cross-session retry records remain a separate persisted shape | intentional/keep |\n'
ownership_path.write_text(replace_once(ownership, anchor, row, "ownership table"))

after_lines = {}
for target in TARGETS:
    after_lines[target] = sum(
        len((Path(target) / rel).read_text().splitlines())
        for rel in [
            "skills/sce-next-task/SKILL.md",
            "skills/sce-next-task/references/task-execution.md",
            "skills/sce-next-task/references/context-sync.md",
        ]
    )
reductions = {target: before_lines[target] - after_lines[target] for target in TARGETS}
if len(set(reductions.values())) != 1 or next(iter(reductions.values())) <= 0:
    raise SystemExit(f"unexpected line reductions: {reductions}")
reduction = next(iter(reductions.values()))

plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T03: `Deduplicate next-task complete execution handoff schema`" in plan:
    raise SystemExit("T03 already present")
plan += f'''\n\n## Follow-up: complete execution-handoff schema ownership\n\n- [x] T03: `Deduplicate next-task complete execution handoff schema` (status:done)\n  - Scope: the composite same-session `complete` result produced by task execution,\n    the next-task handoff boundary, task-context-sync consumption/validation,\n    canonical Pkl sources, tracked Pi/Claude/Codex mirrors, and the ownership table.\n  - Ownership after change: `references/task-execution.md` defines the composite\n    complete handoff fields once; `/next-task` passes the result verbatim; context\n    sync validates and consumes that contract without restating its live field list.\n    Package mode retains its existing `references/execution-contract.yaml`.\n  - Separate shape preserved: cross-session synchronization recovery continues to\n    consume the persisted completed-task record and blocker; this task does not alter\n    synchronization-debt recovery semantics.\n  - Result: the selected next-task entrypoint/execution/context-sync documents shrink\n    by {reduction} Markdown lines per tracked target.\n  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,\n    `nix run .#pkl-check-generated`, targeted ownership assertions across all three\n    tracked targets, and `git diff --check`.\n  - Context synchronization: synced.\n'''
plan_path.write_text(plan)

for target in TARGETS:
    skill = (Path(target) / "skills/sce-next-task/SKILL.md").read_text()
    execution = (Path(target) / "skills/sce-next-task/references/task-execution.md").read_text()
    context = (Path(target) / "skills/sce-next-task/references/context-sync.md").read_text()
    assert execution.count("A successful `complete` handoff must explicitly contain all of these fields:") == 1
    assert execution.count("The resolved `plan` object") == 1
    assert "handoff contains the resolved plan, task identity" not in execution
    assert "which reads the\nplan, completed task, changed files" not in execution
    assert "complete handoff contract in `references/task-execution.md`" in context
    assert "`plan_update`" not in context
    assert "`changes.summary`" not in context
    assert "owns reading the plan, task, changed files" not in skill

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
print(f"line reduction per tracked target: {reduction}")
