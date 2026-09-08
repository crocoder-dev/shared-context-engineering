from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
marker = 'run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")\n'
if s.count(marker) != 1:
    raise SystemExit(f'expected one verification start marker, found {s.count(marker)}')
head, _ = s.split(marker, 1)
p.write_text(head + 'print("T10 apply-only complete")\n')
