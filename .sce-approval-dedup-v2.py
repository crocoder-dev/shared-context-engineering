from pathlib import Path

source = Path('.sce-approval-dedup.py').read_text()

start = source.index('# Add generation assertions for the ownership split and exact-question retention.')
end = source.index('# Durable ownership table.', start)
source = source[:start] + '# Existing generation contracts plus rendered-file inspection verify this change.\n\n' + source[end:]

source = source.replace(
    '  - Verify: generation contract checks assert the ownership split and that the exact\\n    approval question occurs once in `references/output.md`, never in `SKILL.md` or\\n    `task-execution.md`; generated inventory stays unchanged.\\n',
    '  - Verify: existing generation contracts pass; rendered package inspection confirms\\n    the exact approval question remains in `references/output.md`, not `SKILL.md` or\\n    `task-execution.md`; generated inventory stays unchanged.\\n',
)

code = compile(source, '.sce-approval-dedup.py', 'exec')
exec(code, {'__name__': '__main__'})
