from pathlib import Path
import subprocess

pr_head = 'bd0e8c6daeebbaaf2055dd4387dafc8ba106b033'
pr_branch = 'codex/compress-workflow-preambles'

subprocess.run(['git', 'config', 'user.name', 'David Abram'], check=True)
subprocess.run(['git', 'config', 'user.email', 'david@crocoder.dev'], check=True)
subprocess.run(['git', 'checkout', '--detach', pr_head], check=True)

old = '''The gate must be shown even when:\n\n- The task appears straightforward.\n- The workflow believes approval was already implied.\n- The handoff is stale or incomplete.\n- The user is likely to approve.\n\nWhen the `approve` flag is absent, end the gate with exactly one approval\nquestion:\n\n`Continue with implementation now? (yes/no)`\n\nStop and wait for the user's answer. Do not return internal state, and make no\nfile modifications, until the user has answered.\n\nWhen the `approve` flag is supplied, show the gate as a summary, omit the\napproval question, do not wait, and continue at step 2.4.\n'''
new = '''Always show the gate, including for straightforward, pre-approved, stale, or\nincomplete input. `references/output.md` owns its exact content and question text.\n\nWithout `approve`, render the gate's approval question and wait. Do not return\ninternal state or modify files until the user answers. With `approve`, render the\ngate without its question, do not wait, and continue at step 2.4.\n'''

paths = [
    Path('config/pkl/base/workflow-next-task.pkl'),
    Path('.pi/skills/sce-next-task/references/task-execution.md'),
    Path('.claude/skills/sce-next-task/references/task-execution.md'),
    Path('.agents/skills/sce-next-task/references/task-execution.md'),
]
for path in paths:
    text = path.read_text()
    if old not in text:
        raise SystemExit(f'missing approval reference block in {path}')
    path.write_text(text.replace(old, new, 1))

question = '`Continue with implementation now? (yes/no)`'
for target in ['.pi', '.claude', '.agents']:
    skill = Path(target) / 'skills/sce-next-task/SKILL.md'
    execution = Path(target) / 'skills/sce-next-task/references/task-execution.md'
    output = Path(target) / 'skills/sce-next-task/references/output.md'
    assert question not in skill.read_text(), skill
    assert question not in execution.read_text(), execution
    assert question in output.read_text(), output
    assert "approval question and wait" in execution.read_text(), execution

subprocess.run(['pkl', 'eval', 'config/pkl/renderers/generation-contract-check.pkl'], check=True)
subprocess.run(['nix', 'run', '.#pkl-check-generated'], check=True)
subprocess.run(['git', 'diff', '--check'], check=True)
subprocess.run(['git', 'add'] + [str(p) for p in paths], check=True)
subprocess.run(['git', 'commit', '--amend', '--no-edit'], check=True)
subprocess.run(['git', 'push', '--force-with-lease', 'origin', f'HEAD:{pr_branch}'], check=True)
print(subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip())
