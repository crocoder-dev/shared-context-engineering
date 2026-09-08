from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old = '  "exactly one file"\n'
new = '  "exactly one ADR"\n'
if s.count(old) != 1:
    raise SystemExit(f'expected one obsolete decision package token, found {s.count(old)}')
p.write_text(s.replace(old, new))
