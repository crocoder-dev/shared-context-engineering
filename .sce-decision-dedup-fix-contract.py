from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
needle = 'c = between(c, start, end, new_defs, "decision contract definitions")\n'
insert = '''c = between(c, start, end, new_defs, "decision contract definitions")\n# The baseline has a second, older template-token block later in the file.\nmarker = "local requiredDecisionTemplateTokens = new Listing {\\n"\nfirst = c.find(marker)\nsecond = c.find(marker, first + len(marker))\nif first < 0 or second < 0:\n    raise SystemExit("expected both expanded and legacy decision template token blocks")\nblock_end = c.find("}\\n\\n", second)\nif block_end < 0:\n    raise SystemExit("legacy decision template token block end missing")\nc = c[:second] + c[block_end + 3:]\n'''
if s.count(needle) != 1:
    raise SystemExit(f'expected one contract replacement anchor, found {s.count(needle)}')
p.write_text(s.replace(needle, insert))
