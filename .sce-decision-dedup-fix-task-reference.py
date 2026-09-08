from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old = '''sync_text = once(sync_text, gate_start, gate_start + "    " + GATE_OWNER.replace("\\n", "\\n    ").rstrip() + "\\n\\n", "canonical gate owner")\nsync_canonical.write_text(sync_text)\n'''
new = '''sync_text = once(sync_text, gate_start, gate_start + "    " + GATE_OWNER.replace("\\n", "\\n    ").rstrip() + "\\n\\n", "canonical gate owner")\nexplicit_gate_heading = "## 3.5 Record qualifying architecture decisions\\n\\n"\nsync_text = once(sync_text, explicit_gate_heading, explicit_gate_heading + GATE_OWNER, "canonical taskReference gate owner")\nsync_canonical.write_text(sync_text)\n'''
if s.count(old) != 1:
    raise SystemExit(f'expected one canonical gate write block, found {s.count(old)}')
p.write_text(s.replace(old, new))
