from pathlib import Path

source = Path('.sce-handoff-dedup.py').read_text()
source = source.replace(
    'next_text = replace_marked(next_text, old_terminal.splitlines()[0], old_terminal.splitlines()[-1], new_terminal, 0, "composite terminal handoff verification")',
    'if old_terminal.splitlines()[0] in next_text:\n    next_text = replace_marked(next_text, old_terminal.splitlines()[0], old_terminal.splitlines()[-1], new_terminal, 0, "remaining terminal handoff verification")',
)
source = source.replace(
    'next_text = replace_marked(next_text, old_return.splitlines()[0], old_return.splitlines()[-1], new_return, 0, "composite return handoff")',
    'if old_return.splitlines()[0] in next_text:\n    next_text = replace_marked(next_text, old_return.splitlines()[0], old_return.splitlines()[-1], new_return, 0, "remaining return handoff")',
)
source = source.replace(
    "old_dynamic_input = '''    A live execution result must have:",
    "old_dynamic_input = '''A live execution result must have:",
)
source = source.replace(
    "new_dynamic_input = '''    A live execution result must have `status: complete` and satisfy",
    "new_dynamic_input = '''A live execution result must have `status: complete` and satisfy",
)
code = compile(source, '.sce-handoff-dedup.py', 'exec')
exec(code, {'__name__': '__main__'})
