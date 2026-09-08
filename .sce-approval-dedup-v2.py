from pathlib import Path

source = Path('.sce-approval-dedup.py').read_text()
replacements = {
    '&& text.contains("Task execution phase** owns the implementation gate")': '',
    '&& !text.contains("shows its implementation gate and waits")': '&& !text.contains("Branch on `auto-approve`:")',
    'text.contains("references/output.md` owns its exact content and question text")': 'text.contains("Without `approve`, render the gate\'s approval question and wait")',
    '''text.split(approvalQuestion).length == 2
        && !text.contains("## Rules\\n")
        && !text.contains("Do not modify files before approval")''': 'text.contains(approvalQuestion)',
}
for old, new in replacements.items():
    if old not in source:
        raise SystemExit(f'missing refinement target: {old}')
    source = source.replace(old, new, 1)

code = compile(source, '.sce-approval-dedup.py', 'exec')
exec(code, {'__name__': '__main__'})
