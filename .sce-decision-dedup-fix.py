from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old = '''for rel in tracked_rels:\n    baseline = (Path(TARGETS[0]) / rel).read_text()\n    for target in TARGETS[1:]:\n        if (Path(target) / rel).read_text() != baseline:\n            raise SystemExit(f"target mirror drift for {rel}: {target}")\n\n'''
if s.count(old) != 1:
    raise SystemExit(f'expected one strict mirror block, found {s.count(old)}')
p.write_text(s.replace(old, ''))
