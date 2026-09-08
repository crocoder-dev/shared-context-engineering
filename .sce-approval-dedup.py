# preparation trigger
from pathlib import Path

files = [
    Path('config/pkl/base/workflow-content.pkl'),
    Path('config/pkl/base/workflow-next-task.pkl'),
    Path('config/pkl/renderers/generation-contract-check.pkl'),
    Path('context/sce/dedup-ownership-table.md'),
    Path('context/plans/compress-workflow-execution-preamble.md'),
]


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    if old not in text:
        raise SystemExit(f'missing replacement in {path}: {old[:80]!r}')
    path.write_text(text.replace(old, new, 1))

# Entrypoint: parse/pass approval only; phase owns all gate behavior.
replace_once(
    files[0],
    '''This phase always shows an implementation gate before it modifies any file, and it\nis the only phase permitted to ask the user for confirmation. Both properties are\nload-bearing, so reach them through the reference rather than acting from this\nsummary.\n\nBranch on `auto-approve`:\n\n`approved` -> Also pass the `approve` flag. The **Task execution phase** then shows its implementation gate as a summary and proceeds without asking.\n\nelse -> Do not pass the `approve` flag. The **Task execution phase** shows its implementation gate and waits for the user's decision.\n\nDo not present an additional implementation confirmation.\n''',
    '''Pass the `approve` flag only when `auto-approve` is `approved`; otherwise omit it.\nThe **Task execution phase** owns the implementation gate, approval question, wait,\nuser-decision handling, and no-edit-before-approval boundary. Do not duplicate that\nprocedure here or present an additional implementation confirmation.\n'''
)

# Package command body: same ownership boundary.
replace_once(
    files[1],
    '''    Branch on `auto-approve`:\n    \n    `approved` -> Also pass the `approve` flag. \\(taskExecutionUpper.render.apply(mode)) then shows its implementation gate as a summary and proceeds without asking.\n    \n    else -> Do not pass the `approve` flag. \\(taskExecutionUpper.render.apply(mode)) shows its implementation gate and waits for the user's decision.\n    \n    \\(taskExecutionUpper.render.apply(mode)) exclusively owns:\n    \n    - Presenting the implementation summary.\n    - Requesting implementation confirmation.\n    - Implementing the task.\n    - Running task-level verification.\n    - Updating the task status and evidence.\n    \n    Do not present an additional implementation confirmation.\n''',
    '''    Pass the `approve` flag only when `auto-approve` is `approved`; otherwise omit it.\n    \\(taskExecutionUpper.render.apply(mode)) exclusively owns the implementation gate,\n    approval question, wait, decision handling, implementation, task-level verification,\n    and task-status/evidence update. Do not duplicate that procedure here or present an\n    additional implementation confirmation.\n'''
)

# Task execution intro: remove duplicate ownership list and keep phase ownership concise.
replace_once(
    files[1],
    '''This phase exclusively owns:\n\n- Presenting the implementation summary.\n- Requesting implementation confirmation.\n- Implementing the task.\n- Running task-level verification.\n- Updating the task status and evidence.\n\nDo not present an additional implementation confirmation anywhere else.\n\n''',
    '''This phase owns the implementation gate and approval lifecycle, then implements,\nverifies, and records exactly one approved task. No other phase may ask for implementation\nconfirmation.\n\n'''
)

# Task execution: output.md owns exact gate content; this phase owns behavior.
replace_once(
    files[1],
    '''    The gate must be shown even when:\n    \n    - The task appears straightforward.\n    - The workflow believes approval was already implied.\n    - The handoff is stale or incomplete.\n    - The user is likely to approve.\n    \n    When the `approve` flag is absent, end the gate with exactly one approval\n    question:\n    \n    `Continue with implementation now? (yes/no)`\n    \n    Stop and wait for the user's answer. Do not \\(returnYamlLower.render.apply(mode)), and make no file\n    modifications, until the user has answered.\n    \n    When the `approve` flag is supplied, show the gate as a summary, omit the\n    approval question, do not wait, and continue at \\(taskExecutionHeadings.stepReference.apply(mode, 4)).\n''',
    '''    Always show the gate, including for straightforward, pre-approved, stale, or\n    incomplete input. `references/output.md` owns its exact content and question text.\n    \n    Without `approve`, render the gate's approval question and wait. Do not\n    \\(returnYamlLower.render.apply(mode)) or modify files until the user answers. With `approve`, render\n    the gate without its question, do not wait, and continue at\n    \\(taskExecutionHeadings.stepReference.apply(mode, 4)).\n'''
)

# Output reference becomes pure layout contract: drop behavioral Rules section.
replace_once(
    files[1],
    '''    ## Rules\n    \n    - Show the gate exactly once for an unchanged task.\n    - Do not modify files before approval.\n    - Do not add requirements absent from the reviewed task.\n    - Do not present multiple competing approaches unless a material decision is\n      required.\n    - Do not emit YAML while waiting for the user's answer. Stop after the gate and\n      wait.\n    - If the handoff is stale or incomplete, show the known task information and\n      identify the problem under **Risks or trade-offs**.\n''',
    '''    If the handoff is stale or incomplete, show the known task information and\n    identify the problem under **Risks or trade-offs**.\n'''
)
replace_once(
    files[1],
    '''## Rules\n\n- Show the gate exactly once for an unchanged task.\n- Do not modify files before approval.\n- Do not add requirements absent from the reviewed task.\n- Do not present multiple competing approaches unless a material decision is\n  required.\n- Do not emit YAML while waiting for the user's answer. Stop after the gate and\n  wait.\n- If the handoff is stale or incomplete, show the known task information and\n  identify the problem under **Risks or trade-offs**.\n''',
    '''If the handoff is stale or incomplete, show the known task information and\nidentify the problem under **Risks or trade-offs**.\n'''
)

# Add generation assertions for the ownership split and exact-question retention.
insert_before = '''hidden assertValidationIsObservational = (documents: Mapping) ->\n'''
addition = '''local approvalQuestion = "`Continue with implementation now? (yes/no)`"\n\nhidden assertNextTaskApprovalOwnership = (documents: Mapping) ->\n  if (\n    documents.every((path, text) ->\n      if (path.endsWith("/skills/sce-next-task/SKILL.md"))\n        text.contains("Pass the `approve` flag only when")\n        && text.contains("Task execution phase** owns the implementation gate")\n        && !text.contains(approvalQuestion)\n        && !text.contains("shows its implementation gate and waits")\n      else if (path.endsWith("/skills/sce-next-task/references/task-execution.md"))\n        text.contains("references/output.md` owns its exact content and question text")\n        && text.contains("Without `approve`, render the gate's approval question and wait")\n        && !text.contains(approvalQuestion)\n      else if (path.endsWith("/skills/sce-next-task/references/output.md"))\n        text.split(approvalQuestion).length == 2\n        && !text.contains("## Rules\\n")\n        && !text.contains("Do not modify files before approval")\n      else true\n    )\n  ) "sce-next-task approval ownership: entrypoint routes flag, task execution owns behavior, output owns exact gate text"\n  else throw("sce-next-task approval procedure must have one behavioral owner and one exact-layout owner")\n\n'''
text = files[2].read_text()
if insert_before not in text:
    raise SystemExit('missing generation assertion insertion point')
files[2].write_text(text.replace(insert_before, addition + insert_before, 1))
replace_once(
    files[2],
    '''  ["sync-debt-blocked-routing"] = assertSyncDebtBlockedRouting.apply(workflowDocuments)\n''',
    '''  ["sync-debt-blocked-routing"] = assertSyncDebtBlockedRouting.apply(workflowDocuments)\n  ["next-task-approval-ownership"] = assertNextTaskApprovalOwnership.apply(workflowDocuments)\n'''
)

# Durable ownership table.
replace_once(
    files[3],
    '''| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task`; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |\n''',
    '''| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task` parses and conditionally passes `approve`; `references/output.md` owns only exact gate content/order; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |\n'''
)

# Record this as the second commit/change in the same plan/PR.
plan = files[4].read_text()
plan = plan.replace(
    '''This plan covers finding 1 of the workflow duplication audit only. Implementation\nand focused generation checks are complete. Full `nix flake check` remains for PR CI.\n''',
    '''This plan started with finding 1 of the workflow duplication audit. PR #270 now also\ncontains a separate follow-up commit for the next-task approval-gate duplication: the\nentrypoint routes the optional approval flag, task execution owns approval behavior, and\n`references/output.md` owns only the exact gate layout/question. Full `nix flake check`\nremains for PR CI.\n'''
)
append = '''\n## Follow-up: approval-gate ownership deduplication\n\n- [x] T02: `Deduplicate next-task approval-gate procedure` (status:done)\n  - Scope: `sce-next-task` entrypoint, task-execution reference, output reference,\n    canonical Pkl source, generation assertions, tracked Pi/Claude/Codex mirrors,\n    and the ownership table.\n  - Ownership after change: the entrypoint parses `approved` and conditionally passes\n    `approve`; task execution exclusively owns show/wait/approve/decline/block and\n    the no-edit-before-approval boundary; `references/output.md` owns the gate field\n    order and exact approval question only.\n  - Behavior preserved: the gate is always shown, pre-approval never skips it, the\n    non-preapproved path waits in the same workflow, ambiguous answers may ask the\n    same question once more, rejection returns `declined`, and editing remains\n    forbidden before approval.\n  - Verify: generation contract checks assert the ownership split and that the exact\n    approval question occurs once in `references/output.md`, never in `SKILL.md` or\n    `task-execution.md`; generated inventory stays unchanged.\n  - Context synchronization: synced.\n'''
if '## Follow-up: approval-gate ownership deduplication' not in plan:
    plan = plan.rstrip() + '\n' + append
files[4].write_text(plan)

# Generate candidate payload, verify contract, refresh tracked mirrors only.
import subprocess, tempfile, shutil
subprocess.run(['pkl', 'eval', 'config/pkl/renderers/generation-contract-check.pkl'], check=True)
root = Path(tempfile.mkdtemp(prefix='sce-approval-dedup-'))
try:
    subprocess.run(['pkl', 'eval', '-m', str(root), 'config/pkl/generate.pkl'], check=True)
    mappings = {
        '.pi': 'config/.pi',
        '.claude': 'config/.claude',
        '.agents': 'config/.agents',
    }
    for tracked, generated in mappings.items():
        for rel in [
            'skills/sce-next-task/SKILL.md',
            'skills/sce-next-task/references/task-execution.md',
            'skills/sce-next-task/references/output.md',
        ]:
            src = root / generated / rel
            dst = Path(tracked) / rel
            if not dst.exists():
                raise SystemExit(f'missing tracked mirror: {dst}')
            shutil.copyfile(src, dst)
finally:
    shutil.rmtree(root)

subprocess.run(['nix', 'run', '.#pkl-check-generated'], check=True)
subprocess.run(['git', 'diff', '--check'], check=True)

# Ensure no unrelated generated paths or temporary files are committed.
subprocess.run(['git', 'status', '--short'], check=True)

# Commit only the intended change.
subprocess.run(['git', 'add',
    'config/pkl/base/workflow-content.pkl',
    'config/pkl/base/workflow-next-task.pkl',
    'config/pkl/renderers/generation-contract-check.pkl',
    '.pi/skills/sce-next-task/SKILL.md',
    '.pi/skills/sce-next-task/references/task-execution.md',
    '.pi/skills/sce-next-task/references/output.md',
    '.claude/skills/sce-next-task/SKILL.md',
    '.claude/skills/sce-next-task/references/task-execution.md',
    '.claude/skills/sce-next-task/references/output.md',
    '.agents/skills/sce-next-task/SKILL.md',
    '.agents/skills/sce-next-task/references/task-execution.md',
    '.agents/skills/sce-next-task/references/output.md',
    'context/sce/dedup-ownership-table.md',
    'context/plans/compress-workflow-execution-preamble.md',
], check=True)
subprocess.run(['git', 'commit', '-m', 'config: Deduplicate next-task approval gate'], check=True)
subprocess.run(['git', 'push', 'origin', 'HEAD:codex/compress-workflow-preambles'], check=True)
print(subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip())
