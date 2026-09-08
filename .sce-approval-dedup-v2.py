from pathlib import Path

source = Path('.sce-approval-dedup.py').read_text()
start_marker = "addition = '''local approvalQuestion"
start = source.index(start_marker)
body_start = start + len("addition = '''")
end = source.index("'''\ntext = files[2].read_text()", body_start)

assertion = '''local approvalQuestion = "`Continue with implementation now? (yes/no)`"\n\nhidden assertNextTaskApprovalOwnership = (documents: Mapping) ->\n  if (\n    documents.every((path, text) ->\n      if (path.endsWith("/skills/sce-next-task/SKILL.md"))\n        !text.contains(approvalQuestion)\n      else if (path.endsWith("/skills/sce-next-task/references/task-execution.md"))\n        text.contains("approval question and wait")\n        && !text.contains(approvalQuestion)\n      else if (path.endsWith("/skills/sce-next-task/references/output.md"))\n        text.contains(approvalQuestion)\n      else true\n    )\n  ) "sce-next-task approval ownership: execution owns wait behavior and output owns exact question text"\n  else throw("sce-next-task exact approval question must live only in output.md while task execution owns waiting")\n\n'''

source = source[:body_start] + assertion + source[end:]
code = compile(source, '.sce-approval-dedup.py', 'exec')
exec(code, {'__name__': '__main__'})
