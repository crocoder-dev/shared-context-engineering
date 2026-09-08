from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old = '''  "Record qualifying architecture decisions"\n  "sole owner of decision qualification"\n  "System boundaries or ownership"\n'''
new = '''  "Record qualifying architecture decisions"\n  "System boundaries or ownership"\n'''
if s.count(old) != 1:
    raise SystemExit(f'expected one decision workflow token insertion, found {s.count(old)}')
p.write_text(s.replace(old, new))
