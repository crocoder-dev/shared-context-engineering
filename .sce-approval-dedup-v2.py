from pathlib import Path

source = Path('.sce-approval-dedup.py').read_text()
start_marker = "addition = '''local approvalQuestion"
start = source.index(start_marker)
body_start = start + len("addition = '''")
end = source.index("'''\ntext = files[2].read_text()", body_start)

assertion = '''local approvalQuestion = "`Continue with implementation now? (yes/no)`"\n\nhidden assertNextTaskApprovalOwnership = (documents: Mapping) ->\n  if (\n    documents.every((path, text) ->\n      if (path.endsWith("/skills/sce-next-task/SKILL.md"))\n        text.contains("Pass the `approve` flag only when")\n        && !text.contains("Branch on `auto-approve`:")\n        && !text.contains(approvalQuestion)\n      else if (path.endsWith("/skills/sce-next-task/references/task-execution.md"))\n        text.contains("Without `approve`, render the gate's approval question and wait")\n        && !text.contains(approvalQuestion)\n      else if (path.endsWith("/skills/sce-next-task/references/output.md"))\n        text.contains(approvalQuestion)\n      else true\n    )\n  ) "sce-next-task approval ownership: entrypoint routes flag, task execution owns behavior, output owns exact gate text"\n  else throw("sce-next-task approval procedure must have one behavioral owner and one exact-layout owner")\n\n'''

source = source[:body_start] + assertion + source[end:]
code = compile(source, '.sce-approval-dedup.py', 'exec')
exec(code, {'__name__': '__main__'})
